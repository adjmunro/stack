//! Syncing with the remote: fast-forwarding trunks, clearing merged branches, and restacking.

use std::collections::{BTreeSet, HashMap};

use super::*;

impl Workspace {
    /// Brings stacks up to date after work lands:
    ///
    /// 1. Fetches `remote` (default: `origin`, else the only remote), unless `fetch` is false.
    /// 2. Fast-forwards each trunk to its remote branch, if it has one and hasn't diverged.
    /// 3. Finds branches merged into their trunk: by a merge (its tip is in the trunk), a rebase (every commit's
    ///    patch is in the trunk), or a squash (its whole change matches one trunk commit).
    /// 4. Moves the branches on merged ones onto the nearest unmerged branch or trunk below, replaying only their own
    ///    commits, then archives the merged branches (switching off one that's checked out, if the worktree is clean).
    /// 5. Restacks every trunk.
    ///
    /// Each step is its own journalled command, so undo steps back through them.
    ///
    /// # Errors
    /// [`Error::UnknownRemote`], [`Error::Git`] if fetching fails, or as the steps' own commands.
    pub fn sync(&self, remote: Option<&str>, fetch: bool) -> Result<Synced, Error> {
        let remotes = self.git.remotes()?;
        let remote = match remote {
            Some(remote) if remotes.iter().any(|known| known == remote) => Some(remote.to_owned()),
            Some(remote) => {
                return Err(Error::UnknownRemote {
                    name: remote.into(),
                });
            }
            None => remotes
                .iter()
                .find(|known| *known == "origin")
                .cloned()
                .or_else(|| (remotes.len() == 1).then(|| remotes[0].clone())),
        };
        let mut synced = Synced {
            remote: remote.clone(),
            ..Synced::default()
        };
        if let (Some(remote), true) = (&remote, fetch) {
            self.git.fetch(remote)?;
        }

        let (_, resolution) = self.resolve()?;
        let trunks: Vec<(String, String)> = resolution
            .branches
            .iter()
            .filter(|(_, entry)| entry.role == Role::Trunk)
            .map(|(name, entry)| (name.clone(), entry.tip.clone()))
            .collect();
        for (trunk, local) in &trunks {
            let outcome = match &remote {
                None => TrunkUpdate::NoUpstream,
                Some(remote) => self.fast_forward(trunk, local, remote)?,
            };
            synced.trunks.push(TrunkSync {
                name: trunk.clone(),
                outcome,
            });
        }

        let (_, resolution) = self.resolve()?;
        let merged = self.merged_branches(&resolution)?;
        synced.merged = merged.iter().cloned().collect();

        // Rehome the branches on merged ones, onto the nearest unmerged branch (or trunk) below.
        for (name, entry) in &resolution.branches {
            let Some(parent) = &entry.parent else {
                continue;
            };
            if merged.contains(name) || !merged.contains(&parent.name) {
                continue;
            }
            let lineage = resolution.lineage(name);
            let Some(home) = lineage
                .iter()
                .skip(1)
                .find(|ancestor| !merged.contains(**ancestor))
            else {
                continue;
            };
            let restacked = self.move_branch(name, home)?;
            synced.moved.extend(restacked.moved);
            synced.conflicts.extend(restacked.conflicts);
        }

        // Archive merged branches leaf-first, so each is childless by its turn.
        let (_, resolution) = self.resolve()?;
        let mut order: Vec<&String> = merged.iter().collect();
        order.sort_by_key(|name| std::cmp::Reverse(resolution.lineage(name).len()));
        for name in order {
            if self.current_branch()?.as_deref() == Some(name.as_str()) {
                let home = resolution
                    .lineage(name)
                    .into_iter()
                    .skip(1)
                    .find(|ancestor| !merged.contains(*ancestor));
                let clean = self.git.is_worktree_clean(None)?;
                match home {
                    Some(home) if clean => self.switch(home)?,
                    _ => {
                        synced.kept.push(Skipped {
                            branch: name.clone(),
                            reason: "it's checked out".into(),
                        });
                        continue;
                    }
                }
            }
            match self.archive(name) {
                Ok(()) => synced.archived.push(name.clone()),
                Err(error @ (Error::HasChildren { .. } | Error::CheckedOutElsewhere { .. })) => {
                    synced.kept.push(Skipped {
                        branch: name.clone(),
                        reason: error.to_string(),
                    });
                }
                Err(error) => return Err(error),
            }
        }

        for (trunk, _) in &trunks {
            let restacked = self.restack(trunk)?;
            synced.moved.extend(restacked.moved);
            synced.conflicts.extend(restacked.conflicts);
            synced.blocked.extend(restacked.blocked);
        }
        Ok(synced)
    }

    /// Fast-forwards local `trunk` (at `local`) to `remote`'s copy, if that's a fast-forward.
    fn fast_forward(&self, trunk: &str, local: &str, remote: &str) -> Result<TrunkUpdate, Error> {
        let Some(upstream) = self
            .git
            .ref_value(&format!("refs/remotes/{remote}/{trunk}"))?
        else {
            return Ok(TrunkUpdate::NoUpstream);
        };
        Ok(if upstream == local {
            TrunkUpdate::UpToDate
        } else if self.git.is_ancestor(local, &upstream)? {
            let update = RefUpdate {
                name: format!("refs/heads/{trunk}"),
                old: Some(local.into()),
                new: Some(upstream.clone()),
            };
            let description = format!("sync {trunk} with {remote}");
            self.journal.transact(
                &*self.git,
                OperationKind::Command,
                &description,
                None,
                vec![update],
            )?;
            TrunkUpdate::FastForwarded {
                from: local.into(),
                to: upstream,
            }
        } else if self.git.is_ancestor(&upstream, local)? {
            TrunkUpdate::Ahead
        } else {
            TrunkUpdate::Diverged
        })
    }

    /// Branches whose work is in their root trunk: merged, rebased, or squashed in.
    fn merged_branches(&self, resolution: &Resolution) -> Result<BTreeSet<String>, Error> {
        let mut candidates = Vec::new();
        for (name, entry) in &resolution.branches {
            let (Some(parent), Role::Branch) = (&entry.parent, entry.role) else {
                continue;
            };
            let Some(root) = resolution.lineage(name).last().map(|root| root.to_string()) else {
                continue;
            };
            if resolution.branches[&root].role != Role::Trunk {
                continue;
            }
            candidates.push((
                name.clone(),
                entry.tip.clone(),
                parent.offshoot.clone(),
                root,
            ));
        }
        // Each candidate's trunk commits since it forked, with patch-ids computed in one pass.
        let mut ranges: HashMap<String, Vec<String>> = HashMap::new();
        for (name, tip, _, root) in &candidates {
            let trunk_tip = &resolution.branches[root].tip;
            let fork = self.git.merge_base(tip, trunk_tip)?.unwrap_or_default();
            ranges.insert(
                name.clone(),
                self.git.commits_excluding(trunk_tip, &[fork])?,
            );
        }
        let all: BTreeSet<&String> = ranges.values().flatten().collect();
        let trunk_patches = self
            .git
            .patch_ids(&all.into_iter().cloned().collect::<Vec<_>>())?;

        let mut merged = BTreeSet::new();
        for (name, tip, offshoot, root) in &candidates {
            let trunk_tip = &resolution.branches[root].tip;
            let patches: BTreeSet<&String> = ranges[name]
                .iter()
                .filter_map(|commit| trunk_patches.get(commit))
                .collect();
            let own = self
                .git
                .commits_excluding(tip, std::slice::from_ref(offshoot))?;
            let is_merged = if own.is_empty() {
                // No commits of its own any more: merged by a merge commit or fast-forward, as long as it ever had
                // some (a brand-new branch hasn't).
                self.git.is_ancestor(tip, trunk_tip)? && self.had_own_commits(name, tip)?
            } else {
                let own_patches = self.git.patch_ids(&own)?;
                let rebased = own.iter().all(|commit| {
                    own_patches
                        .get(commit)
                        .is_some_and(|patch| patches.contains(patch))
                });
                let squashed = self
                    .git
                    .diff_patch_id(offshoot, tip)?
                    .is_some_and(|patch| patches.contains(&patch));
                rebased || squashed
            };
            if is_merged {
                merged.insert(name.clone());
            }
        }
        Ok(merged)
    }

    /// Whether `branch` (now at `tip`) has ever pointed past the commit it was created at, per its reflog.
    fn had_own_commits(&self, branch: &str, tip: &str) -> Result<bool, Error> {
        let reflog = self.git.reflog(&format!("refs/heads/{branch}"))?;
        let Some(created) = reflog.first() else {
            return Ok(false);
        };
        Ok(created.new != tip || reflog.len() > 1)
    }
}

/// The result of [`crate::Workspace::sync`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Synced {
    /// The remote fetched from and synced with, if any.
    pub remote: Option<String>,
    pub trunks: Vec<TrunkSync>,
    /// Branches found merged into their trunk.
    pub merged: Vec<String>,
    /// Branches moved: off merged branches, then by the final restack.
    pub moved: Vec<Moved>,
    /// Merged branches archived.
    pub archived: Vec<String>,
    /// Merged branches left alone, and why.
    pub kept: Vec<Skipped>,
    pub conflicts: Vec<Conflict>,
    pub blocked: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TrunkSync {
    pub name: String,
    pub outcome: TrunkUpdate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TrunkUpdate {
    FastForwarded {
        from: String,
        to: String,
    },
    UpToDate,
    /// The local trunk has commits the remote doesn't; left as it is.
    Ahead,
    /// Local and remote have both moved on; left as it is.
    Diverged,
    /// No remote copy to sync with.
    NoUpstream,
}
