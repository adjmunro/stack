use std::path::Path;

use crate::git::{GitRepo, GixRepo};
use crate::{Error, Status};

/// Entry point for all `stack` operations on one repository.
pub struct Workspace {
    git: Box<dyn GitRepo>,
}

impl Workspace {
    /// Opens the repository containing `path`, searching parent directories like `git` does.
    ///
    /// # Errors
    /// [`Error::NotARepository`] if no repository is found.
    pub fn discover(path: impl AsRef<Path>) -> Result<Self, Error> {
        Ok(Self {
            git: Box::new(GixRepo::discover(path.as_ref())?),
        })
    }

    /// Reports the repository's current state. Read-only.
    ///
    /// # Errors
    /// [`Error::Git`] if the repository can't be read.
    pub fn status(&self) -> Result<Status, Error> {
        Ok(Status {
            head: self.git.head()?,
        })
    }
}
