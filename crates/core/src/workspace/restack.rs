//! Restacking, previewing, and moving branches.

use super::*;

impl Workspace {
    /// Rebases `branch` and every branch leafward of it (through limbs) onto their parents' current tips, records
    /// each one's parent, and drops the records of branches that no longer exist. For a trunk, restacks every branch on it; the trunk itself never moves.
    ///
    /// All moves apply in one journalled transaction (one undo). If a commit conflicts, that branch and the branches
    /// on it are left as they were, the rest are restacked, and [`Restacked::conflict`] says how to finish by hand.
    ///
    /// # Errors
    /// - [`Error::UnknownBranch`].
    /// - [`Error::OffshootNotInHistory`] or [`Error::MergeCommit`] for a branch restack can't handle.
    /// - As [`Journal`]'s transaction: a dirty or elsewhere-checked-out branch that would move.
    pub fn restack(&self, branch: &str) -> Result<Restacked, Error> {
        let (metadata, resolution) = self.resolve()?;
        if !resolution.branches.contains_key(branch) {
            return Err(Error::UnknownBranch {
                name: branch.into(),
            });
        }
        self.apply_restack(&metadata, &resolution, branch, &format!("restack {branch}"))
    }

    /// Previews restacking `target` (or every trunk, if `None`) without changing anything: which branches would move
    /// cleanly, which would conflict, and which are blocked behind a conflict.
    ///
    /// Writes nothing: merges run against a temporary object directory, and no commit is created or signed.
    ///
    /// # Errors
    /// [`Error::UnknownBranch`], or as [`Self::restack`]'s planning.
    pub fn check(&self, target: Option<&str>) -> Result<RestackPreview, Error> {
        let objects = std::env::temp_dir().join(format!(
            "stack-check-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.as_nanos())
        ));
        std::fs::create_dir_all(&objects).map_err(Error::git)?;
        self.git.quarantine(Some(&objects));
        let preview = self.check_quarantined(target);
        self.git.quarantine(None);
        let _ = std::fs::remove_dir_all(&objects);
        preview
    }

    fn check_quarantined(&self, target: Option<&str>) -> Result<RestackPreview, Error> {
        let (_, resolution) = self.resolve()?;
        let targets: Vec<&str> = match target {
            Some(target) if !resolution.branches.contains_key(target) => {
                return Err(Error::UnknownBranch {
                    name: target.into(),
                });
            }
            Some(target) => vec![target],
            None => resolution
                .branches
                .iter()
                .filter(|(_, entry)| entry.role == Role::Trunk)
                .map(|(name, _)| name.as_str())
                .collect(),
        };
        let mut preview = RestackPreview {
            clean: Vec::new(),
            conflicts: Vec::new(),
            blocked: Vec::new(),
        };
        for target in targets {
            let plan = crate::restack::plan(&*self.git, &resolution, target, Mode::Preview)?;
            preview
                .clean
                .extend(plan.moves.into_iter().map(|moved| PreviewedMove {
                    name: moved.branch,
                    onto: moved.onto,
                    replayed: moved.replayed,
                    dropped: moved.dropped,
                }));
            preview.conflicts.extend(plan.conflicts);
            preview.blocked.extend(plan.blocked);
        }
        preview.blocked.sort();
        Ok(preview)
    }

    /// Moves `branch` onto `onto`: replays its own commits onto `onto`'s tip, restacks everything leafward of it, and
    /// records `onto` as its parent (still pinned, if it was). Otherwise as [`Self::restack`].
    ///
    /// # Errors
    /// - [`Error::UnknownBranch`] if either branch doesn't exist.
    /// - [`Error::IsTrunk`] if `branch` is a trunk; [`Error::NoParent`] if it's unattached.
    /// - [`Error::Cycle`] if `onto` is `branch` or stacked on it.
    /// - As [`Self::restack`].
    pub fn move_branch(&self, branch: &str, onto: &str) -> Result<Restacked, Error> {
        let (metadata, mut resolution) = self.resolve()?;
        let onto_entry = resolution
            .branches
            .get(onto)
            .ok_or_else(|| Error::UnknownBranch { name: onto.into() })?;
        let onto_tip = onto_entry.tip.clone();
        let entry = resolution
            .branches
            .get(branch)
            .ok_or_else(|| Error::UnknownBranch {
                name: branch.into(),
            })?;
        if entry.role == Role::Trunk {
            return Err(Error::IsTrunk {
                name: branch.into(),
            });
        }
        if resolution.lineage(onto).contains(&branch) {
            return Err(Error::Cycle {
                branch: branch.into(),
                parent: onto.into(),
            });
        }
        let parent = entry.parent.clone().ok_or_else(|| Error::NoParent {
            name: branch.into(),
        })?;
        let entry = resolution.branches.get_mut(branch).expect("checked above");
        entry.parent = Some(Parent {
            name: onto.into(),
            needs_restack: parent.offshoot != onto_tip,
            contradicted: false,
            replaces: None,
            ..parent
        });
        self.apply_restack(
            &metadata,
            &resolution,
            branch,
            &format!("move {branch} onto {onto}"),
        )
    }

    /// Moves each of `branches` onto `onto` in turn (making them siblings), as [`Self::move_branch`]. Stops at the
    /// first branch that conflicts; the result gathers every move and that conflict.
    ///
    /// # Errors
    /// As [`Self::move_branch`], for the first branch that fails; earlier moves stand (undo reverts them).
    pub fn move_branches(&self, branches: &[&str], onto: &str) -> Result<Restacked, Error> {
        let mut all = Restacked {
            outcome: Outcome::Unchanged,
            moved: Vec::new(),
            conflicts: Vec::new(),
            blocked: Vec::new(),
        };
        for branch in branches {
            let restacked = self.move_branch(branch, onto)?;
            let stop = !restacked.conflicts.is_empty();
            merge_restacks(&mut all, restacked);
            if stop {
                break;
            }
        }
        Ok(all)
    }

    /// Lines `branches` up into one stack in the given order: each moves onto the one before it, and the first stays
    /// where it is. Stops at the first conflict, as [`Self::move_branches`].
    ///
    /// # Errors
    /// [`Error::Cycle`] if a branch is stacked on a later one in the list (so the chain would loop); otherwise as
    /// [`Self::move_branch`].
    pub fn chain(&self, branches: &[&str]) -> Result<Restacked, Error> {
        let mut all = Restacked {
            outcome: Outcome::Unchanged,
            moved: Vec::new(),
            conflicts: Vec::new(),
            blocked: Vec::new(),
        };
        for pair in branches.windows(2) {
            let restacked = self.move_branch(pair[1], pair[0])?;
            let stop = !restacked.conflicts.is_empty();
            merge_restacks(&mut all, restacked);
            if stop {
                break;
            }
        }
        Ok(all)
    }

    /// Plans restacking `target` over `resolution` and applies it as one journalled command called `description`.
    fn apply_restack(
        &self,
        metadata: &Metadata,
        resolution: &Resolution,
        target: &str,
        description: &str,
    ) -> Result<Restacked, Error> {
        let plan = crate::restack::plan(&*self.git, resolution, target, Mode::Apply)?;
        let mut updates: Vec<RefUpdate> = plan
            .moves
            .iter()
            .map(|moved| RefUpdate {
                name: format!("refs/heads/{}", moved.branch),
                old: Some(moved.old.clone()),
                new: moved.new.clone(),
            })
            .collect();
        for (name, link) in &plan.links {
            let old = metadata.links.get(name);
            if old.is_some_and(|stored| stored.value == *link) {
                continue;
            }
            updates.push(RefUpdate {
                name: format!("{}{name}", metadata::BRANCHES),
                old: old.map(|stored| stored.id.clone()),
                new: Some(self.git.write_blob(&metadata::encode(link))?),
            });
        }
        // Records of branches that no longer exist (archived ones keep theirs for when they come back).
        let archived: Vec<String> = self
            .archived()?
            .into_iter()
            .map(|archived| archived.name)
            .collect();
        for (name, stored) in &metadata.links {
            if !resolution.branches.contains_key(name) && !archived.contains(name) {
                updates.push(RefUpdate {
                    name: format!("{}{name}", metadata::BRANCHES),
                    old: Some(stored.id.clone()),
                    new: None,
                });
            }
        }
        let outcome = if updates.is_empty() {
            Outcome::Unchanged
        } else {
            self.journal.transact(
                &*self.git,
                OperationKind::Command,
                description,
                None,
                updates,
            )?;
            Outcome::Changed
        };
        let moved = plan
            .moves
            .into_iter()
            .map(|moved| Moved {
                name: moved.branch,
                onto: moved.onto,
                old: moved.old,
                new: moved.new.expect("applied plans write commits"),
                replayed: moved.replayed,
                dropped: moved.dropped,
            })
            .collect();
        Ok(Restacked {
            outcome,
            moved,
            conflicts: plan.conflicts,
            blocked: plan.blocked,
        })
    }
}

/// Adds `next`'s moves and conflicts to `all`.
fn merge_restacks(all: &mut Restacked, next: Restacked) {
    if next.outcome == Outcome::Changed {
        all.outcome = Outcome::Changed;
    }
    all.moved.extend(next.moved);
    all.conflicts.extend(next.conflicts);
    all.blocked.extend(next.blocked);
}
