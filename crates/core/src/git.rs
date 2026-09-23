//! The `GitRepo` port and its adapters.

use std::path::Path;

use crate::{Error, Head};

/// All git access from the core goes through this trait.
pub(crate) trait GitRepo: Send + Sync {
    fn head(&self) -> Result<Head, Error>;
}

/// [`GitRepo`] backed by `gix`.
pub(crate) struct GixRepo {
    repo: gix::ThreadSafeRepository,
}

impl GixRepo {
    pub(crate) fn discover(path: &Path) -> Result<Self, Error> {
        let repo =
            gix::ThreadSafeRepository::discover(path).map_err(|source| Error::NotARepository {
                path: path.to_owned(),
                source: source.into(),
            })?;
        Ok(Self { repo })
    }
}

impl GitRepo for GixRepo {
    fn head(&self) -> Result<Head, Error> {
        let repo = self.repo.to_thread_local();
        let head = repo.head().map_err(Error::git)?;
        let commit = if head.is_unborn() {
            None
        } else {
            Some(repo.head_id().map_err(Error::git)?.to_string())
        };
        Ok(match (head.referent_name(), commit) {
            (Some(name), commit) => Head::Branch {
                name: name.shorten().to_string(),
                commit,
            },
            (None, Some(commit)) => Head::Detached { commit },
            (None, None) => unreachable!("an unborn HEAD always names a branch"),
        })
    }
}
