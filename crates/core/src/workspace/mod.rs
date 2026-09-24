use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::forge::{Forge, GhForge, PullRequest};
use crate::git::{Branch, Checkout, GitRepo, GixRepo, Merge, PushRef, RefUpdate};
use crate::journal::Journal;
use crate::metadata::{self, Link, Mark, Metadata};
use crate::resolve::Resolution;
use crate::restack::Mode;
use crate::{
    Archived, CommitRange, CommitReview, Conflict, Delta, Error, FollowPosition, FollowerSync,
    Following, GuardInstalled, GuardViolation, Head, Imported, ImportedParent, Landed, LostCommit,
    MarkKind, Moved, Node, Operation, OperationKind, Outcome, Parent, PreviewedMove,
    ProposalAction, Proposed, ProposedBranch, PushOutcome, Pushed, PushedBranch, Recovered,
    RestackPreview, Restacked, Role, Scope, Skipped, Status, Step, SyncOutcome, Tree, Worktree,
};

/// Entry point for all `stack` operations on one repository.
///
/// Queries never write. Mutations are compare-and-swap (each fails without changes if the metadata it read was
/// changed concurrently) and journalled in the op log, so they can be undone and survive a crash part-way through.
pub struct Workspace {
    git: Box<dyn GitRepo>,
    forge: Box<dyn Forge>,
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

mod archive;
mod delta;
mod guard;
mod import;
mod lost;
mod navigate;
mod propose;
mod push;
mod resolving;
pub use resolving::{ResolveOutcome, Resolving};
mod restack;
mod split;
mod sync;
pub use sync::{Synced, TrunkSync, TrunkUpdate};
mod review;
mod worktrees;

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
        let git = GixRepo::discover(path.as_ref(), environment.clone())?;
        let current = git
            .worktrees()?
            .into_iter()
            .find(|worktree| worktree.current);
        let workdir = current.map_or_else(|| git.common_dir(), |worktree| worktree.path);
        let forge = GhForge {
            workdir,
            environment,
        };
        let journal = Journal::new(git.common_dir().join("stack").join("stack.db"));
        let recovered = journal.recover(&git)?;
        Ok(Self {
            git: Box::new(git),
            forge: Box::new(forge),
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

    /// Undoes every command from `id` on, newest first, taking the repository back to before command `id`. Returns
    /// them in the order undone.
    ///
    /// # Errors
    /// [`Error::NothingToUndo`] if no command from `id` on is waiting to be undone; otherwise as [`Self::undo`],
    /// stopping at the first command that can't be (those already undone stay undone).
    pub fn undo_to(&self, id: i64) -> Result<Vec<Operation>, Error> {
        let mut undone = Vec::new();
        while let Some(next) = self.journal.next_undo()? {
            if next.id < id {
                break;
            }
            undone.push(self.undo()?);
        }
        if undone.is_empty() {
            return Err(Error::NothingToUndo);
        }
        Ok(undone)
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

    /// [`Self::tree`] reduced to the branches whose own commits change any of `paths` (git pathspecs), plus the
    /// branches rootward of them so the structure still reads. Trunks always stay.
    pub fn tree_touching(&self, paths: &[String]) -> Result<Tree, Error> {
        let resolution = self.resolve()?.1;
        let mut touching = std::collections::BTreeSet::new();
        for (name, entry) in &resolution.branches {
            if entry.role == Role::Trunk {
                continue;
            }
            let hidden: Vec<String> = entry
                .parent
                .iter()
                .map(|parent| parent.offshoot.clone())
                .collect();
            if self.git.touches(&entry.tip, &hidden, paths)? {
                touching.insert(name.clone());
            }
        }
        let tree = resolution.tree(self.current_branch()?.as_deref());
        let keep = |nodes: Vec<Node>| prune(nodes, &touching);
        Ok(Tree {
            trunks: tree
                .trunks
                .into_iter()
                .map(|trunk| Node {
                    children: keep(trunk.children),
                    ..trunk
                })
                .collect(),
            unattached: keep(tree.unattached),
        })
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

/// Keeps the nodes in `keep`, and any node with a kept descendant.
fn prune(nodes: Vec<Node>, keep: &std::collections::BTreeSet<String>) -> Vec<Node> {
    nodes
        .into_iter()
        .filter_map(|node| {
            let children = prune(node.children, keep);
            (keep.contains(&node.name) || !children.is_empty()).then_some(Node { children, ..node })
        })
        .collect()
}
