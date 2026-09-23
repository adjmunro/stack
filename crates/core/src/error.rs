use std::path::PathBuf;

type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// Errors returned by `stack-core`.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// No git repository was found at or above `path`.
    #[error("not a git repository (or any parent): {}", path.display())]
    NotARepository { path: PathBuf, source: BoxError },

    /// `name` isn't a valid ref name (e.g. a branch name containing `..`).
    #[error("invalid ref name: {name}")]
    InvalidRefName { name: String },

    /// No local branch called `name`.
    #[error("no such branch: {name}")]
    UnknownBranch { name: String },

    /// `name` is a trunk, which has no parent.
    #[error("{name} is a trunk")]
    IsTrunk { name: String },

    #[error("{name} is not a trunk")]
    NotTrunk { name: String },

    #[error("{name} is not pinned")]
    NotPinned { name: String },

    /// `name` has no parent to pin.
    #[error("{name} has no parent; name one with --parent")]
    NoParent { name: String },

    /// Making `parent` the parent of `branch` would create a cycle.
    #[error("{parent} is {branch} or stacked on it; that would create a cycle")]
    Cycle { branch: String, parent: String },

    /// `branch` and `parent` share no history.
    #[error("{branch} and {parent} have no common ancestor")]
    Unrelated { branch: String, parent: String },

    /// A ref under `refs/stack/` holds data `stack` can't read.
    #[error("corrupt metadata in {reference}: {reason}")]
    CorruptMetadata { reference: String, reason: String },

    #[error("nothing to undo")]
    NothingToUndo,

    #[error("nothing to redo")]
    NothingToRedo,

    /// `reference` changed after the operation being undone or redone, so reverting it would lose that change.
    #[error("{reference} has changed since; undo or redo would overwrite that change")]
    UndoConflict { reference: String },

    /// `branch` is checked out and has uncommitted changes to tracked files.
    #[error("{branch} is checked out with uncommitted changes; commit or stash them first")]
    DirtyWorktree { branch: String },

    #[error("{branch} is checked out; switch to another branch first")]
    DeletesCheckedOutBranch { branch: String },

    /// `branch` is checked out in another worktree, which moving it would leave out of step.
    #[error("{branch} is checked out in another worktree")]
    CheckedOutElsewhere { branch: String },

    /// `branch`'s recorded offshoot isn't in its history (e.g. it was pinned to a parent it isn't built on), so its
    /// own commits can't be told apart.
    #[error(
        "can't restack {branch}: it isn't built on {parent}; pin it to the branch it is built on"
    )]
    OffshootNotInHistory { branch: String, parent: String },

    /// Restack doesn't replay merge commits yet.
    #[error("can't restack {branch}: it contains merge commit {commit}")]
    MergeCommit { branch: String, commit: String },

    /// The op log (SQLite) failed.
    #[error("op log: {0}")]
    Store(BoxError),

    /// The underlying git backend failed.
    #[error("git: {0}")]
    Git(BoxError),
}

impl Error {
    pub(crate) fn store(error: impl Into<BoxError>) -> Self {
        Self::Store(error.into())
    }

    pub(crate) fn git(error: impl Into<BoxError>) -> Self {
        Self::Git(error.into())
    }
}
