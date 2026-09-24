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

    /// There's no remote to push to: none is configured, or there are several and none was chosen.
    #[error("no remote to push to; add one, or choose one with --remote")]
    NoRemote,

    #[error("no such remote: {name}")]
    UnknownRemote { name: String },

    /// `branch` tracks a remote branch with a different name, so pushing it to its own name would publish it
    /// somewhere unexpected.
    #[error(
        "{branch} tracks {upstream}, which has a different name; push it with git, or change its upstream"
    )]
    UpstreamMismatch { branch: String, upstream: String },

    /// `name` has no branches stacked on it.
    #[error("{name} has no branches on it")]
    NoChildren { name: String },

    /// `name` has several branches stacked on it, so "up" is ambiguous.
    #[error("{name} has several branches on it: {}; choose one", children.join(", "))]
    SeveralChildren { name: String, children: Vec<String> },

    /// `name` still has branches stacked on it, which would lose their parent.
    #[error("{name} has branches on it: {}; move or archive them first", children.join(", "))]
    HasChildren { name: String, children: Vec<String> },

    #[error("no such commit: {revision}")]
    UnknownRevision { revision: String },

    #[error("{} already exists", path.display())]
    PathExists { path: std::path::PathBuf },

    /// Landing needs a follower: a worktree with a detached `HEAD`.
    #[error("this worktree isn't detached; land from a follower worktree")]
    NotDetached,

    /// The current worktree follows no branch and none was named.
    #[error("this worktree follows no branch; name the branch to land on")]
    NotFollowing,

    #[error("nothing to land on {branch}")]
    NothingToLand { branch: String },

    /// The follower's commits aren't built on `branch`'s current tip (it moved on or was rewritten meanwhile).
    #[error("these commits aren't built on {branch}'s current tip; sync or rebase onto it first")]
    NotFastForward { branch: String },

    #[error("{name} is already archived")]
    AlreadyArchived { name: String },

    #[error("no archived branch called {name}")]
    NotArchived { name: String },

    /// A branch called `name` exists, so an archived one of that name can't be restored.
    #[error("a branch called {name} already exists")]
    BranchExists { name: String },

    /// The code-review host (via `gh`) failed.
    #[error("{0}")]
    Forge(String),

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
