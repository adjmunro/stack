use std::path::Path;

use serde::Serialize;

use crate::git::{GitRepo, GixRepo, RefUpdate};
use crate::metadata::{self, Link, Mark, Metadata};
use crate::resolve::Resolution;
use crate::{Error, Head, Outcome, Role, Status, Tree};

/// Entry point for all `stack` operations on one repository.
///
/// Queries never write. Mutations are compare-and-swap: each fails without changes if the metadata it read was
/// changed concurrently.
pub struct Workspace {
    git: Box<dyn GitRepo>,
}

/// The result of [`Workspace::add_trunk`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Marked {
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
        self.require_branch(name)?;
        if let Some(mark) = metadata.marks.get(name) {
            let parent = resolution.branches[name]
                .parent
                .as_ref()
                .map(|parent| parent.name.clone());
            return Ok(Marked {
                outcome: Outcome::Unchanged,
                role: mark.value.role,
                parent,
            });
        }
        let parent = resolution.branches[name]
            .parent
            .as_ref()
            .map(|parent| parent.name.clone())
            .filter(|parent| !metadata.marks.contains_key(parent));
        let role = if parent.is_some() {
            Role::Limb
        } else {
            Role::Trunk
        };
        let blob = self.git.write_blob(&metadata::encode(&Mark { role }))?;
        let outcome = self.update(format!("{}{name}", metadata::TRUNKS), None, Some(blob))?;
        Ok(Marked {
            outcome,
            role,
            parent,
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
        self.update(
            format!("{}{name}", metadata::TRUNKS),
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
        let outcome = self.update(format!("{}{branch}", metadata::BRANCHES), old, Some(blob))?;
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
        self.update(
            format!("{}{branch}", metadata::BRANCHES),
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
        name: String,
        old: Option<String>,
        new: Option<String>,
    ) -> Result<Outcome, Error> {
        self.git.update_refs(&[RefUpdate { name, old, new }])?;
        Ok(Outcome::Changed)
    }
}
