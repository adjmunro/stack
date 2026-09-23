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

    /// `name` is neither a trunk nor tracked, so it can't be a parent.
    #[error("{name} is not a trunk or tracked branch; add it as a trunk or track it first")]
    UntrackedParent { name: String },

    /// `name` is a trunk, which has no parent.
    #[error("{name} is a trunk")]
    IsTrunk { name: String },

    /// `name` is tracked, so it can't also be a trunk.
    #[error("{name} is tracked; untrack it first")]
    IsTracked { name: String },

    #[error("{name} is not tracked")]
    NotTracked { name: String },

    #[error("{name} is not a trunk")]
    NotTrunk { name: String },

    /// Making `parent` the parent of `branch` would create a cycle.
    #[error("{parent} is {branch} or stacked on it; that would create a cycle")]
    Cycle { branch: String, parent: String },

    /// `name` still has tracked children, which would be left without a parent.
    #[error("{name} has tracked children: {}", children.join(", "))]
    HasChildren { name: String, children: Vec<String> },

    /// `branch` and `parent` share no history.
    #[error("{branch} and {parent} have no common ancestor")]
    Unrelated { branch: String, parent: String },

    /// A ref under `refs/stack/` holds data `stack` can't read.
    #[error("corrupt metadata in {reference}: {reason}")]
    CorruptMetadata { reference: String, reason: String },

    /// The underlying git backend failed.
    #[error("git: {0}")]
    Git(BoxError),
}

impl Error {
    pub(crate) fn git(error: impl Into<BoxError>) -> Self {
        Self::Git(error.into())
    }
}
