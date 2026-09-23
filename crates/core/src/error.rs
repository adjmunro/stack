use std::path::PathBuf;

type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// Errors returned by `stack-core`.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// No git repository was found at or above `path`.
    #[error("not a git repository (or any parent): {}", path.display())]
    NotARepository { path: PathBuf, source: BoxError },

    /// The underlying git backend failed.
    #[error("git: {0}")]
    Git(BoxError),
}

impl Error {
    pub(crate) fn git(error: impl Into<BoxError>) -> Self {
        Self::Git(error.into())
    }
}
