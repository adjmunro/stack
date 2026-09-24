//! Worktrees, followers, and landing.

use super::*;

/// A worktree's key in the follower registry: its canonical path, so different spellings of it match.
fn worktree_key(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_owned())
        .to_string_lossy()
        .into_owned()
}

impl Workspace {
    /// Where [`Self::add_worktree`] puts a worktree for `branch` by default: a hidden sibling of the main worktree,
    /// `.<repo>-<branch>` (with `/` in the branch name as `-`).
    ///
    /// # Errors
    /// [`Error::Git`] for a repository without a main worktree (bare).
    pub fn default_worktree_path(&self, branch: &str) -> Result<PathBuf, Error> {
        let main = self
            .git
            .worktrees()?
            .into_iter()
            .next()
            .ok_or_else(|| Error::git("no main worktree"))?;
        let repo = main
            .path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let parent = main
            .path
            .parent()
            .ok_or_else(|| Error::git("main worktree has no parent directory"))?;
        Ok(parent.join(format!(".{repo}-{}", branch.replace('/', "-"))))
    }

    /// Adds a worktree for `branch` at `path` (default: [`Self::default_worktree_path`]). If `branch` is already
    /// checked out somewhere, the new worktree follows it instead: detached at its tip, and moved along by
    /// [`Self::sync_followers`].
    ///
    /// # Errors
    /// [`Error::UnknownBranch`], [`Error::PathExists`], or [`Error::Git`] if `git worktree add` fails.
    pub fn add_worktree(&self, branch: &str, path: Option<&Path>) -> Result<Worktree, Error> {
        let tip = self
            .git
            .branch_tip(branch)?
            .ok_or_else(|| Error::UnknownBranch {
                name: branch.into(),
            })?;
        let path = match path {
            Some(path) => path.to_owned(),
            None => self.default_worktree_path(branch)?,
        };
        if path.exists() {
            return Err(Error::PathExists { path });
        }
        let checked_out = self
            .git
            .worktrees()?
            .iter()
            .any(|worktree| worktree.branch.as_deref() == Some(branch));
        if checked_out {
            self.git.add_worktree(&path, None, Some(&tip))?;
            let key = worktree_key(&path);
            self.journal
                .with_store(|store| store.set_follower(&key, branch))?;
        } else {
            self.git.add_worktree(&path, Some(branch), None)?;
        }
        let canonical = path.canonicalize().map_err(Error::git)?;
        self.worktrees()?
            .into_iter()
            .find(|worktree| {
                worktree
                    .path
                    .canonicalize()
                    .is_ok_and(|other| other == canonical)
            })
            .ok_or_else(|| Error::git("the new worktree isn't listed"))
    }

    /// Every worktree, the main one first, with what followers follow and how they stand.
    pub fn worktrees(&self) -> Result<Vec<Worktree>, Error> {
        let followers = self.followers()?;
        self.git
            .worktrees()?
            .into_iter()
            .map(|info| {
                let follows = match followers
                    .iter()
                    .find(|(path, _)| *path == worktree_key(&info.path))
                {
                    Some((_, branch)) if info.branch.is_none() => Some(Following {
                        position: self.follow_position(branch, info.head.as_deref())?,
                        branch: branch.clone(),
                    }),
                    _ => None,
                };
                Ok(Worktree {
                    path: info.path,
                    head: info.head,
                    branch: info.branch,
                    follows,
                    current: info.current,
                })
            })
            .collect()
    }

    /// Brings each follower worktree to its branch's tip, carrying local changes that don't clash. Followers with
    /// commits of their own (to land), or whose `HEAD` was never a version of the branch, are left alone. Forgets
    /// followers whose worktree is gone or has since checked out a branch.
    pub fn sync_followers(&self) -> Result<Vec<FollowerSync>, Error> {
        let detached: Vec<String> = self
            .git
            .worktrees()?
            .iter()
            .filter(|worktree| worktree.branch.is_none())
            .map(|worktree| worktree_key(&worktree.path))
            .collect();
        for (path, _) in self.followers()? {
            if !detached.contains(&path) {
                self.journal
                    .with_store(|store| store.remove_follower(&path))?;
            }
        }
        let branches: BTreeMap<String, Branch> = self
            .git
            .branches()?
            .into_iter()
            .map(|branch| (branch.name.clone(), branch))
            .collect();
        let mut synced = Vec::new();
        for worktree in self.worktrees()? {
            let (Some(following), Some(head)) = (&worktree.follows, &worktree.head) else {
                continue;
            };
            let skipped = |reason: &str| SyncOutcome::Skipped {
                reason: reason.into(),
            };
            let outcome = match (following.position, branches.get(&following.branch)) {
                (FollowPosition::UpToDate, _) => SyncOutcome::UpToDate,
                (FollowPosition::Ahead, _) => skipped("has commits to land"),
                (_, None) => skipped("its branch no longer exists"),
                (_, Some(branch)) => {
                    let former = branch.former.iter().any(|former| former.id == *head);
                    if !former && !self.git.is_ancestor(head, &branch.tip)? {
                        skipped("isn't at any version of its branch")
                    } else if let Err(error) =
                        self.git.checkout(Some(&worktree.path), head, &branch.tip)
                    {
                        skipped(&format!("its local changes clash: {error}"))
                    } else {
                        self.git.set_detached_head(&worktree.path, &branch.tip)?;
                        SyncOutcome::Moved {
                            from: head.clone(),
                            to: branch.tip.clone(),
                        }
                    }
                }
            };
            synced.push(FollowerSync {
                path: worktree.path,
                branch: following.branch.clone(),
                outcome,
            });
        }
        Ok(synced)
    }

    /// Lands this follower's commits on `branch` (default: the branch it follows): fast-forwards the branch to this
    /// worktree's `HEAD` and, if another worktree has the branch checked out, moves that worktree's files with it,
    /// carrying its local changes that don't clash. The main workspace keeps its checkout throughout.
    ///
    /// # Errors
    /// - [`Error::NotDetached`], [`Error::NotFollowing`], or [`Error::UnknownBranch`].
    /// - [`Error::NothingToLand`] if `HEAD` is the branch's tip.
    /// - [`Error::NotFastForward`] unless `HEAD` is built on the branch's current tip.
    /// - [`Error::Git`] if the holder's local changes clash; nothing is changed.
    pub fn land(&self, branch: Option<&str>) -> Result<Landed, Error> {
        let Head::Detached { commit: head } = self.git.head()? else {
            return Err(Error::NotDetached);
        };
        let worktrees = self.git.worktrees()?;
        let branch = match branch {
            Some(branch) => branch.to_owned(),
            None => {
                let here = worktrees
                    .iter()
                    .find(|worktree| worktree.current)
                    .map(|worktree| worktree_key(&worktree.path));
                let followers = self.followers()?;
                followers
                    .into_iter()
                    .find(|(path, _)| Some(path) == here.as_ref())
                    .map(|(_, branch)| branch)
                    .ok_or(Error::NotFollowing)?
            }
        };
        let tip = self
            .git
            .branch_tip(&branch)?
            .ok_or_else(|| Error::UnknownBranch {
                name: branch.clone(),
            })?;
        if tip == head {
            return Err(Error::NothingToLand { branch });
        }
        if !self.git.is_ancestor(&tip, &head)? {
            return Err(Error::NotFastForward { branch });
        }
        let commits = self
            .git
            .commits_excluding(&head, std::slice::from_ref(&tip))?
            .len();
        let holder = worktrees
            .iter()
            .find(|worktree| worktree.branch.as_deref() == Some(branch.as_str()));
        let updates = vec![RefUpdate {
            name: format!("refs/heads/{branch}"),
            old: Some(tip.clone()),
            new: Some(head.clone()),
        }];
        let description = format!("land {commits} commit(s) on {branch}");
        let checkout = holder.map(|holder| Checkout {
            worktree: Some(holder.path.clone()),
            from: tip.clone(),
            to: head.clone(),
        });
        let held = checkout.map(|checkout| (branch.as_str(), checkout));
        self.journal.transact_with(
            &*self.git,
            OperationKind::Command,
            &description,
            None,
            updates,
            held,
        )?;
        Ok(Landed {
            holder: holder.map(|holder| holder.path.clone()),
            branch,
            old: tip,
            new: head,
            commits,
        })
    }

    fn followers(&self) -> Result<Vec<(String, String)>, Error> {
        Ok(self
            .journal
            .query(|store| store.followers())?
            .unwrap_or_default())
    }

    fn follow_position(&self, branch: &str, head: Option<&str>) -> Result<FollowPosition, Error> {
        let (Some(tip), Some(head)) = (self.git.branch_tip(branch)?, head) else {
            return Ok(FollowPosition::Orphaned);
        };
        Ok(if tip == head {
            FollowPosition::UpToDate
        } else if self.git.is_ancestor(&tip, head)? {
            FollowPosition::Ahead
        } else {
            FollowPosition::Behind
        })
    }
}
