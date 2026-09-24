use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::git::{Branch, Checkout, GitRepo, GixRepo, PushRef, RefUpdate};
use crate::journal::Journal;
use crate::metadata::{self, Link, Mark, Metadata};
use crate::resolve::Resolution;
use crate::restack::Mode;
use crate::{
    Archived, CommitReview, Error, FollowPosition, FollowerSync, Following, Head, Landed,
    LostCommit, MarkKind, Moved, Operation, OperationKind, Outcome, Parent, PreviewedMove,
    PushOutcome, Pushed, PushedBranch, Recovered, RestackPreview, Restacked, Role, Scope, Status,
    Step, SyncOutcome, Tree, Worktree,
};

/// Entry point for all `stack` operations on one repository.
///
/// Queries never write. Mutations are compare-and-swap (each fails without changes if the metadata it read was
/// changed concurrently) and journalled in the op log, so they can be undone and survive a crash part-way through.
pub struct Workspace {
    git: Box<dyn GitRepo>,
    journal: Journal,
    recovered: Vec<Recovered>,
}

/// The environment `stack` runs `git` subprocesses with.
///
/// Only `git` subprocesses (merges, commits, working-tree updates, config lookups) use it; the in-process git
/// library still reads this process's environment and the user's git config.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Environment {
    /// This process's environment, minus variables that would redirect `git` to another repository or index.
    #[default]
    Inherit,
    /// Exactly these variables and no others: e.g. a GUI supplying the user's login-shell environment, or tests
    /// sealing off the developer's git config.
    Exactly(Vec<(OsString, OsString)>),
}

/// The result of [`Workspace::add_trunk`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Marked {
    pub name: String,
    pub outcome: Outcome,
    /// [`Role::Trunk`], or [`Role::Limb`] if the branch is stacked on a regular branch.
    pub role: Role,
    /// The branch it is stacked on, for a limb.
    pub parent: Option<String>,
}

/// The result of [`Workspace::pin`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Pinned {
    pub outcome: Outcome,
    pub parent: String,
}

/// A worktree's key in the follower registry: its canonical path, so different spellings of it match.
fn worktree_key(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_owned())
        .to_string_lossy()
        .into_owned()
}

/// The store key for a commit's marks: its patch-id, or the commit itself if it has none (merges, empty commits).
fn mark_key(commit: &str, patch_ids: &std::collections::HashMap<String, String>) -> String {
    match patch_ids.get(commit) {
        Some(patch) => format!("patch:{patch}"),
        None => format!("commit:{commit}"),
    }
}

impl Workspace {
    /// Opens the repository containing `path`, searching parent directories like `git` does, and resolves any
    /// operation a previous run left interrupted (see [`Self::recovered`]).
    ///
    /// # Errors
    /// [`Error::NotARepository`] if no repository is found.
    pub fn discover(path: impl AsRef<Path>) -> Result<Self, Error> {
        Self::discover_with(path, Environment::Inherit)
    }

    /// As [`Self::discover`], running `git` with `environment` instead of this process's.
    ///
    /// # Errors
    /// [`Error::NotARepository`] if no repository is found.
    pub fn discover_with(path: impl AsRef<Path>, environment: Environment) -> Result<Self, Error> {
        let git = GixRepo::discover(path.as_ref(), environment)?;
        let journal = Journal::new(git.common_dir().join("stack").join("stack.db"));
        let recovered = journal.recover(&git)?;
        Ok(Self {
            git: Box::new(git),
            journal,
            recovered,
        })
    }

    /// Interrupted operations resolved when this workspace was opened.
    pub fn recovered(&self) -> &[Recovered] {
        &self.recovered
    }

    /// Reverts the latest command that isn't already undone. Returns that command.
    ///
    /// # Errors
    /// [`Error::NothingToUndo`], or [`Error::UndoConflict`] if a ref it changed has changed again since.
    pub fn undo(&self) -> Result<Operation, Error> {
        self.journal.undo(&*self.git)
    }

    /// Re-applies the most recently undone command, as long as no command has run since. Returns that command.
    ///
    /// # Errors
    /// [`Error::NothingToRedo`], or [`Error::UndoConflict`] if a ref it changed has changed again since.
    pub fn redo(&self) -> Result<Operation, Error> {
        self.journal.redo(&*self.git)
    }

    /// The most recent operations, newest first.
    pub fn oplog(&self, limit: usize) -> Result<Vec<Operation>, Error> {
        self.journal.recent(limit)
    }

    /// Creates an empty git repository at `path` with `git init`.
    ///
    /// # Errors
    /// [`Error::Git`] if `git init` fails.
    pub fn create_repository(path: impl AsRef<Path>) -> Result<(), Error> {
        crate::git::create_repository(path.as_ref())
    }

    /// Sets `stack` up: marks each of `trunks` as a trunk, or the [suggested trunk](Self::suggested_trunk) if
    /// `trunks` is empty. Returns one result per trunk; empty if no trunk could be suggested.
    ///
    /// # Errors
    /// As [`Self::add_trunk`].
    pub fn init(&self, trunks: &[&str]) -> Result<Vec<Marked>, Error> {
        let suggested = if trunks.is_empty() {
            self.suggested_trunk()?
        } else {
            None
        };
        let trunks = trunks
            .iter()
            .map(|trunk| (*trunk).to_owned())
            .chain(suggested);
        trunks.map(|trunk| self.add_trunk(&trunk)).collect()
    }

    /// The branch most likely to be the trunk: `origin`'s default branch if it exists locally; else the current
    /// branch if it is `main`, `master`, `develop`, or `trunk`; else the first of those that exists; else the current
    /// branch. `None` if HEAD is detached and none of those exist.
    pub fn suggested_trunk(&self) -> Result<Option<String>, Error> {
        const CONVENTIONAL: [&str; 4] = ["main", "master", "develop", "trunk"];
        if let Some(branch) = self.git.remote_default_branch("origin")? {
            return Ok(Some(branch));
        }
        let current = self.current_branch()?;
        if let Some(current) = current
            .as_ref()
            .filter(|current| CONVENTIONAL.contains(&current.as_str()))
        {
            return Ok(Some(current.clone()));
        }
        for name in CONVENTIONAL {
            if self.git.branch_tip(name)?.is_some() {
                return Ok(Some(name.to_owned()));
            }
        }
        Ok(current)
    }

    /// Reports the repository's current state.
    pub fn status(&self) -> Result<Status, Error> {
        Ok(Status {
            head: self.git.head()?,
        })
    }

    /// Every trunk and the branches stacked on it, resolved from the commit graph and recorded parents.
    pub fn tree(&self) -> Result<Tree, Error> {
        let resolution = self.resolve()?.1;
        Ok(resolution.tree(self.current_branch()?.as_deref()))
    }

    /// Marks the existing branch `name` as a trunk. Its role is inferred: a limb if it is stacked on a regular
    /// branch, otherwise a trunk.
    ///
    /// # Errors
    /// [`Error::UnknownBranch`].
    pub fn add_trunk(&self, name: &str) -> Result<Marked, Error> {
        let (metadata, resolution) = self.resolve()?;
        let parent = match resolution.branches.get(name) {
            Some(entry) => entry.parent.as_ref().map(|parent| parent.name.clone()),
            // A branch with no commits yet can only be the current one.
            None if self.current_branch()?.as_deref() == Some(name) => None,
            None => return Err(Error::UnknownBranch { name: name.into() }),
        };
        let marked = |outcome, role, parent| Marked {
            name: name.into(),
            outcome,
            role,
            parent,
        };
        if let Some(mark) = metadata.marks.get(name) {
            return Ok(marked(Outcome::Unchanged, mark.value.role, parent));
        }
        let parent = parent.filter(|parent| !metadata.marks.contains_key(parent));
        let role = if parent.is_some() {
            Role::Limb
        } else {
            Role::Trunk
        };
        let blob = self.git.write_blob(&metadata::encode(&Mark { role }))?;
        let reference = format!("{}{name}", metadata::TRUNKS);
        let outcome = self.update(&format!("trunk add {name}"), reference, None, Some(blob))?;
        Ok(marked(outcome, role, parent))
    }

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
    /// Merges write unreferenced tree objects, as `git merge-tree` does; no commits, refs, or signatures.
    ///
    /// # Errors
    /// [`Error::UnknownBranch`], or as [`Self::restack`]'s planning.
    pub fn check(&self, target: Option<&str>) -> Result<RestackPreview, Error> {
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

    /// The branches in `branch`'s line under `scope`, rootward first: what push covers. Never includes a trunk.
    ///
    /// # Errors
    /// [`Error::UnknownBranch`].
    pub fn line(&self, branch: &str, scope: Scope) -> Result<Vec<String>, Error> {
        let (_, resolution) = self.resolve()?;
        if !resolution.branches.contains_key(branch) {
            return Err(Error::UnknownBranch {
                name: branch.into(),
            });
        }
        Ok(crate::line::line(&resolution, branch, scope))
    }

    /// Pushes `branch`'s line (see [`Self::line`]) to the same names on `remote`, with a lease on each so a remote
    /// branch changed since the last fetch is never overwritten. Branches without an upstream get `remote`'s; others
    /// keep theirs (so pushing a backup to a second remote changes nothing locally). Without `remote`: `branch`'s
    /// upstream remote, else `origin`, else the only remote.
    ///
    /// # Errors
    /// - [`Error::UnknownBranch`], [`Error::NoRemote`], or [`Error::UnknownRemote`].
    /// - [`Error::UpstreamMismatch`] if a branch tracks a remote branch with a different name.
    /// - [`Error::Git`] if the remote can't be reached. Rejected branches are reported in the result instead.
    pub fn push(&self, branch: &str, scope: Scope, remote: Option<&str>) -> Result<Pushed, Error> {
        let branches = self.line(branch, scope)?;
        let remotes = self.git.remotes()?;
        let remote = match remote {
            Some(remote) if remotes.iter().any(|known| known == remote) => remote.to_owned(),
            Some(remote) => {
                return Err(Error::UnknownRemote {
                    name: remote.into(),
                });
            }
            None => self
                .git
                .config_value(&format!("branch.{branch}.remote"))?
                .filter(|upstream| remotes.contains(upstream))
                .or_else(|| remotes.iter().find(|known| *known == "origin").cloned())
                .or_else(|| (remotes.len() == 1).then(|| remotes[0].clone()))
                .ok_or(Error::NoRemote)?,
        };
        let mut refs = Vec::new();
        let mut without_upstream = Vec::new();
        for name in &branches {
            match self.git.config_value(&format!("branch.{name}.merge"))? {
                Some(merge) if merge != format!("refs/heads/{name}") => {
                    let upstream_remote = self
                        .git
                        .config_value(&format!("branch.{name}.remote"))?
                        .unwrap_or_default();
                    let upstream = format!(
                        "{upstream_remote}/{}",
                        merge.trim_start_matches("refs/heads/")
                    );
                    return Err(Error::UpstreamMismatch {
                        branch: name.clone(),
                        upstream,
                    });
                }
                Some(_) => {}
                None => without_upstream.push(name.clone()),
            }
            let lease = self
                .git
                .ref_value(&format!("refs/remotes/{remote}/{name}"))?;
            refs.push(PushRef {
                branch: name.clone(),
                lease,
            });
        }
        let statuses = if refs.is_empty() {
            Vec::new()
        } else {
            self.git.push(&remote, &refs)?
        };
        for status in &statuses {
            if status.outcome != PushOutcome::Rejected && without_upstream.contains(&status.branch)
            {
                self.git.set_upstream(&status.branch, &remote)?;
            }
        }
        let branches = branches
            .into_iter()
            .map(|name| {
                let status = statuses.iter().find(|status| status.branch == name);
                PushedBranch {
                    outcome: status.map_or(PushOutcome::Rejected, |status| status.outcome),
                    summary: status.map_or_else(
                        || "not reported by git".to_owned(),
                        |status| status.summary.clone(),
                    ),
                    name,
                }
            })
            .collect();
        Ok(Pushed { remote, branches })
    }

    /// The branch one `step` away from `branch` in its stack. Nothing is checked out; see [`Self::switch`].
    ///
    /// # Errors
    /// - [`Error::UnknownBranch`].
    /// - [`Error::NoParent`] stepping down from a trunk or unattached branch; [`Error::IsTrunk`] for
    ///   [`Step::Bottom`] from a trunk.
    /// - [`Error::NoChildren`] or [`Error::SeveralChildren`] stepping up.
    pub fn step(&self, branch: &str, step: Step) -> Result<String, Error> {
        let (_, resolution) = self.resolve()?;
        let entry = resolution
            .branches
            .get(branch)
            .ok_or_else(|| Error::UnknownBranch {
                name: branch.into(),
            })?;
        let parent_of = |name: &str| {
            resolution.branches[name]
                .parent
                .as_ref()
                .map(|parent| parent.name.clone())
        };
        let only_child = |name: &str| -> Result<String, Error> {
            let mut children: Vec<String> = resolution
                .branches
                .iter()
                .filter(|(_, entry)| {
                    entry
                        .parent
                        .as_ref()
                        .is_some_and(|parent| parent.name == name)
                })
                .map(|(child, _)| child.clone())
                .collect();
            match children.len() {
                0 => Err(Error::NoChildren { name: name.into() }),
                1 => Ok(children.remove(0)),
                _ => Err(Error::SeveralChildren {
                    name: name.into(),
                    children,
                }),
            }
        };
        let mut current = branch.to_owned();
        match step {
            Step::Down(count) => {
                for _ in 0..count {
                    current = parent_of(&current).ok_or_else(|| Error::NoParent {
                        name: current.clone(),
                    })?;
                }
            }
            Step::Up(count) => {
                for _ in 0..count {
                    current = only_child(&current)?;
                }
            }
            Step::Top => loop {
                match only_child(&current) {
                    Ok(child) => current = child,
                    Err(Error::NoChildren { .. }) => break,
                    Err(error) => return Err(error),
                }
            },
            Step::Bottom => {
                if entry.role == Role::Trunk {
                    return Err(Error::IsTrunk {
                        name: branch.into(),
                    });
                }
                if entry.role == Role::Branch {
                    // A limb heads its own stack, so the walk ends on it; a trunk doesn't, so it ends above it.
                    while let Some(parent) = parent_of(&current) {
                        let role = resolution.branches[&parent].role;
                        if role != Role::Trunk {
                            current = parent;
                        }
                        if role != Role::Branch {
                            break;
                        }
                    }
                }
            }
        }
        Ok(current)
    }

    /// Checks out `branch` with `git switch` (so git's safety checks and hooks apply).
    ///
    /// # Errors
    /// [`Error::Git`] if git refuses, e.g. because local changes would be overwritten.
    pub fn switch(&self, branch: &str) -> Result<(), Error> {
        self.git.switch(branch, false)
    }

    /// Creates `name` on the current branch and checks it out, then commits if `message` is given (staging changes
    /// to tracked files first if `all`). A shortcut for `git switch --create` and `git commit`: the parent is worked
    /// out automatically, and undo doesn't cover it.
    ///
    /// # Errors
    /// [`Error::Git`] if git refuses (e.g. the branch exists, or there is nothing to commit).
    pub fn create(&self, name: &str, message: Option<&str>, all: bool) -> Result<(), Error> {
        self.git.switch(name, true)?;
        if let Some(message) = message {
            self.git.commit_index(message, all)?;
        }
        Ok(())
    }

    /// Puts `branch` away: it leaves `refs/heads/` (so `git branch`, the tree, restack, and push no longer see it)
    /// for `refs/stack/archive/`, which keeps its commits from `git gc`. Journalled, so undo restores it.
    ///
    /// # Errors
    /// - [`Error::UnknownBranch`]; [`Error::IsTrunk`] for a trunk.
    /// - [`Error::HasChildren`] while branches are stacked on it.
    /// - [`Error::AlreadyArchived`] if an archived branch has the same name.
    /// - [`Error::DeletesCheckedOutBranch`] or [`Error::CheckedOutElsewhere`] if it's checked out.
    pub fn archive(&self, branch: &str) -> Result<(), Error> {
        let (_, resolution) = self.resolve()?;
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
        let children: Vec<String> = resolution
            .branches
            .iter()
            .filter(|(_, child)| {
                child
                    .parent
                    .as_ref()
                    .is_some_and(|parent| parent.name == branch)
            })
            .map(|(name, _)| name.clone())
            .collect();
        if !children.is_empty() {
            return Err(Error::HasChildren {
                name: branch.into(),
                children,
            });
        }
        let archived = format!("{}{branch}", metadata::ARCHIVE);
        if self.git.ref_value(&archived)?.is_some() {
            return Err(Error::AlreadyArchived {
                name: branch.into(),
            });
        }
        let updates = vec![
            RefUpdate {
                name: format!("refs/heads/{branch}"),
                old: Some(entry.tip.clone()),
                new: None,
            },
            RefUpdate {
                name: archived,
                old: None,
                new: Some(entry.tip.clone()),
            },
        ];
        let description = format!("archive {branch}");
        self.journal.transact(
            &*self.git,
            OperationKind::Command,
            &description,
            None,
            updates,
        )?;
        Ok(())
    }

    /// Restores an archived branch to `refs/heads/`. Its recorded parent, if any, still applies.
    ///
    /// # Errors
    /// [`Error::NotArchived`], or [`Error::BranchExists`] if a branch of that name has been created since.
    pub fn unarchive(&self, name: &str) -> Result<(), Error> {
        let archived = format!("{}{name}", metadata::ARCHIVE);
        let commit = self
            .git
            .ref_value(&archived)?
            .ok_or_else(|| Error::NotArchived { name: name.into() })?;
        if self.git.branch_tip(name)?.is_some() {
            return Err(Error::BranchExists { name: name.into() });
        }
        let updates = vec![
            RefUpdate {
                name: archived,
                old: Some(commit.clone()),
                new: None,
            },
            RefUpdate {
                name: format!("refs/heads/{name}"),
                old: None,
                new: Some(commit),
            },
        ];
        let description = format!("unarchive {name}");
        self.journal.transact(
            &*self.git,
            OperationKind::Command,
            &description,
            None,
            updates,
        )?;
        Ok(())
    }

    /// Archived branches, sorted by name.
    pub fn archived(&self) -> Result<Vec<Archived>, Error> {
        Ok(self
            .git
            .refs(metadata::ARCHIVE)?
            .into_iter()
            .map(|(name, commit)| Archived {
                name: name.trim_start_matches(metadata::ARCHIVE).to_owned(),
                commit,
            })
            .collect())
    }

    /// Marks the commit `revision` names. The mark follows the commit's change (its patch-id) through rebases and
    /// amends that leave the change alone, and lapses once the change itself changes. Merge and empty commits have no
    /// patch-id, so their marks stay with the exact commit. Replaces an existing mark of the same kind.
    ///
    /// Marks are personal: they live in the local store, not in refs, and aren't in the op log.
    ///
    /// # Errors
    /// [`Error::UnknownRevision`].
    pub fn mark(&self, revision: &str, kind: MarkKind, note: Option<&str>) -> Result<(), Error> {
        let key = self.mark_key(revision)?;
        self.journal
            .with_store(|store| store.set_mark(&key, kind, note))
    }

    /// Removes the marks of `kind` (or all marks) from the commit `revision` names. Returns how many were removed.
    ///
    /// # Errors
    /// [`Error::UnknownRevision`].
    pub fn unmark(&self, revision: &str, kind: Option<MarkKind>) -> Result<usize, Error> {
        let key = self.mark_key(revision)?;
        Ok(self
            .journal
            .query(|store| store.clear_marks(&key, kind))?
            .unwrap_or(0))
    }

    /// `branch`'s own commits (those after its offshoot, or its whole history if it has no parent), newest first,
    /// with their marks.
    ///
    /// # Errors
    /// [`Error::UnknownBranch`].
    pub fn review(&self, branch: &str) -> Result<Vec<CommitReview>, Error> {
        let (_, resolution) = self.resolve()?;
        let entry = resolution
            .branches
            .get(branch)
            .ok_or_else(|| Error::UnknownBranch {
                name: branch.into(),
            })?;
        let hidden: Vec<String> = entry
            .parent
            .iter()
            .map(|parent| parent.offshoot.clone())
            .collect();
        let commits = self.git.commits_excluding(&entry.tip, &hidden)?;
        let patch_ids = self.git.patch_ids(&commits)?;
        commits
            .into_iter()
            .map(|commit| {
                let key = mark_key(&commit, &patch_ids);
                let marks = self
                    .journal
                    .query(|store| store.marks(&key))?
                    .unwrap_or_default();
                Ok(CommitReview {
                    summary: self.git.commit(&commit)?.summary,
                    commit,
                    marks,
                })
            })
            .collect()
    }

    fn mark_key(&self, revision: &str) -> Result<String, Error> {
        let commit = self.git.resolve_commit(revision)?;
        let patch_ids = self.git.patch_ids(std::slice::from_ref(&commit))?;
        Ok(mark_key(&commit, &patch_ids))
    }

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

    /// Commits you've been on (per the `HEAD` and branch reflogs) that no ref reaches any more: e.g. the version
    /// before an amend, work left by `reset --hard`, or a deleted branch you had checked out. Only the newest commit of
    /// each lost chain is listed, newest sighting first, up to `limit`. Restore one with `git branch <name> <commit>`.
    pub fn lost(&self, limit: usize) -> Result<Vec<LostCommit>, Error> {
        let mut references = vec!["HEAD".to_owned()];
        references.extend(
            self.git
                .branches()?
                .into_iter()
                .map(|branch| format!("refs/heads/{}", branch.name)),
        );
        // The latest sighting of each commit.
        let mut sightings: BTreeMap<String, (String, String, i64)> = BTreeMap::new();
        for reference in &references {
            for entry in self.git.reflog(reference)? {
                for commit in [&entry.old, &entry.new] {
                    if commit.chars().all(|c| c == '0') {
                        continue;
                    }
                    let newer = sightings
                        .get(commit)
                        .is_none_or(|(_, _, time)| entry.time >= *time);
                    if newer {
                        sightings.insert(
                            commit.clone(),
                            (reference.clone(), entry.message.clone(), entry.time),
                        );
                    }
                }
            }
        }
        let tips = self.git.commit_tips()?;
        let candidates: Vec<String> = sightings
            .keys()
            .filter(|commit| !tips.contains(commit))
            .cloned()
            .collect();
        if candidates.is_empty() {
            return Ok(Vec::new());
        }
        // Lost: reachable from a sighting but from no ref, found in one walk.
        let lost: std::collections::BTreeSet<String> = self
            .git
            .reachable_excluding(&candidates, &tips)?
            .into_iter()
            .collect();
        // Newest of each chain: a lost sighting that no other lost sighting builds on.
        let mut found = Vec::new();
        for candidate in candidates
            .iter()
            .filter(|candidate| lost.contains(*candidate))
        {
            let mut superseded = false;
            for other in candidates
                .iter()
                .filter(|other| *other != candidate && lost.contains(*other))
            {
                if self.git.is_ancestor(candidate, other)? {
                    superseded = true;
                    break;
                }
            }
            if !superseded {
                let (seen_on, how, seen_at) = sightings[candidate].clone();
                let summary = self.git.commit(candidate)?.summary;
                found.push(LostCommit {
                    commit: candidate.clone(),
                    summary,
                    seen_on,
                    how,
                    seen_at,
                });
            }
        }
        found.sort_by(|a, b| {
            b.seen_at
                .cmp(&a.seen_at)
                .then_with(|| a.commit.cmp(&b.commit))
        });
        found.truncate(limit);
        Ok(found)
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

    /// Unmarks a trunk. The branch itself is untouched; branches on it are resolved again.
    ///
    /// # Errors
    /// [`Error::NotTrunk`].
    pub fn remove_trunk(&self, name: &str) -> Result<Role, Error> {
        let metadata = Metadata::load(&*self.git)?;
        let stored = metadata
            .marks
            .get(name)
            .ok_or_else(|| Error::NotTrunk { name: name.into() })?;
        let reference = format!("{}{name}", metadata::TRUNKS);
        self.update(
            &format!("trunk remove {name}"),
            reference,
            Some(stored.id.clone()),
            None,
        )?;
        Ok(stored.value.role)
    }

    /// Pins `parent` as the parent of `branch`, overriding the commit graph. Without `parent`, pins the currently
    /// resolved one.
    ///
    /// # Errors
    /// - [`Error::UnknownBranch`] if either branch doesn't exist.
    /// - [`Error::IsTrunk`] if `branch` is a trunk.
    /// - [`Error::NoParent`] if `parent` is omitted and `branch` has none.
    /// - [`Error::Cycle`] if `parent` is `branch` or stacked on it.
    /// - [`Error::Unrelated`] if they share no history.
    pub fn pin(&self, branch: &str, parent: Option<&str>) -> Result<Pinned, Error> {
        let (metadata, resolution) = self.resolve()?;
        let tip = self.require_branch(branch)?;
        let entry = &resolution.branches[branch];
        if entry.role == Role::Trunk {
            return Err(Error::IsTrunk {
                name: branch.into(),
            });
        }
        let parent = match parent {
            Some(parent) => parent.to_owned(),
            None => entry
                .parent
                .as_ref()
                .ok_or_else(|| Error::NoParent {
                    name: branch.into(),
                })?
                .name
                .clone(),
        };
        let parent_tip = self.require_branch(&parent)?;
        if resolution.lineage(&parent).contains(&branch) {
            return Err(Error::Cycle {
                branch: branch.into(),
                parent,
            });
        }
        let offshoot = match entry
            .parent
            .as_ref()
            .filter(|current| current.name == parent)
        {
            Some(current) => current.offshoot.clone(),
            None => self
                .git
                .merge_base(&tip, &parent_tip)?
                .ok_or_else(|| Error::Unrelated {
                    branch: branch.into(),
                    parent: parent.clone(),
                })?,
        };
        let link = Link {
            parent: parent.clone(),
            offshoot,
            pinned: true,
        };
        let old = metadata.links.get(branch);
        if old.is_some_and(|stored| stored.value == link) {
            return Ok(Pinned {
                outcome: Outcome::Unchanged,
                parent,
            });
        }
        let blob = self.git.write_blob(&metadata::encode(&link))?;
        let old = old.map(|stored| stored.id.clone());
        let reference = format!("{}{branch}", metadata::BRANCHES);
        let outcome = self.update(
            &format!("pin {branch} on {}", link.parent),
            reference,
            old,
            Some(blob),
        )?;
        Ok(Pinned { outcome, parent })
    }

    /// Removes a pin, so the parent is worked out automatically again.
    ///
    /// # Errors
    /// [`Error::NotPinned`].
    pub fn unpin(&self, branch: &str) -> Result<Outcome, Error> {
        let metadata = Metadata::load(&*self.git)?;
        let stored = metadata
            .links
            .get(branch)
            .filter(|stored| stored.value.pinned)
            .ok_or_else(|| Error::NotPinned {
                name: branch.into(),
            })?;
        let reference = format!("{}{branch}", metadata::BRANCHES);
        self.update(
            &format!("unpin {branch}"),
            reference,
            Some(stored.id.clone()),
            None,
        )
    }

    fn resolve(&self) -> Result<(Metadata, Resolution), Error> {
        let metadata = Metadata::load(&*self.git)?;
        let resolution = Resolution::resolve(&*self.git, &metadata)?;
        Ok((metadata, resolution))
    }

    fn current_branch(&self) -> Result<Option<String>, Error> {
        Ok(match self.git.head()? {
            Head::Branch { name, .. } => Some(name),
            Head::Detached { .. } => None,
        })
    }

    fn require_branch(&self, name: &str) -> Result<String, Error> {
        self.git
            .branch_tip(name)?
            .ok_or_else(|| Error::UnknownBranch { name: name.into() })
    }

    fn update(
        &self,
        description: &str,
        name: String,
        old: Option<String>,
        new: Option<String>,
    ) -> Result<Outcome, Error> {
        let updates = vec![RefUpdate { name, old, new }];
        self.journal.transact(
            &*self.git,
            OperationKind::Command,
            description,
            None,
            updates,
        )?;
        Ok(Outcome::Changed)
    }
}
