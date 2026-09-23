use std::collections::BTreeSet;
use std::path::Path;

use crate::git::{GitRepo, GixRepo, RefUpdate};
use crate::metadata::{self, Link, Metadata};
use crate::{Error, Head, Node, Outcome, Status, Tree};

/// Entry point for all `stack` operations on one repository.
///
/// Mutations are compare-and-swap: each fails without changes if the metadata it read was changed concurrently.
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
    pub fn status(&self) -> Result<Status, Error> {
        Ok(Status {
            head: self.git.head()?,
        })
    }

    /// Every trunk and the branches stacked on it. Read-only.
    pub fn tree(&self) -> Result<Tree, Error> {
        let metadata = Metadata::load(&*self.git)?;
        let current = match self.git.head()? {
            Head::Branch { name, .. } => Some(name),
            Head::Detached { .. } => None,
        };
        let mut builder = TreeBuilder {
            git: &*self.git,
            metadata: &metadata,
            current,
            visited: BTreeSet::new(),
        };
        let mut trunks = Vec::new();
        for name in metadata.trunks.keys() {
            trunks.push(builder.node(name)?);
        }
        // Roots first, so orphans keep their children. Anything still unvisited sits in a cycle of corrupt metadata.
        let (roots, rest): (Vec<_>, Vec<_>) = metadata.branches.iter().partition(|(_, stored)| {
            let parent = &stored.value.parent;
            !metadata.is_trunk(parent) && !metadata.branches.contains_key(parent)
        });
        let mut orphans = Vec::new();
        for (name, _) in roots.into_iter().chain(rest) {
            if !builder.visited.contains(name) {
                orphans.push(builder.node(name)?);
            }
        }
        orphans.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(Tree { trunks, orphans })
    }

    /// Makes the existing branch `name` a trunk.
    ///
    /// # Errors
    /// [`Error::UnknownBranch`], or [`Error::IsTracked`] if `name` is stacked on another branch.
    pub fn add_trunk(&self, name: &str) -> Result<Outcome, Error> {
        let metadata = Metadata::load(&*self.git)?;
        self.require_branch(name)?;
        if metadata.is_trunk(name) {
            return Ok(Outcome::Unchanged);
        }
        if metadata.branches.contains_key(name) {
            return Err(Error::IsTracked { name: name.into() });
        }
        let blob = self.git.write_blob(&metadata::encode_trunk())?;
        self.update(format!("{}{name}", metadata::TRUNKS), None, Some(blob))
    }

    /// Stops treating `name` as a trunk. The branch itself is untouched.
    ///
    /// # Errors
    /// [`Error::NotTrunk`], or [`Error::HasChildren`] while branches are stacked on it.
    pub fn remove_trunk(&self, name: &str) -> Result<Outcome, Error> {
        let metadata = Metadata::load(&*self.git)?;
        let stored = metadata
            .trunks
            .get(name)
            .ok_or_else(|| Error::NotTrunk { name: name.into() })?;
        require_childless(&metadata, name)?;
        self.update(
            format!("{}{name}", metadata::TRUNKS),
            Some(stored.id.clone()),
            None,
        )
    }

    /// Records `parent` as the parent of `branch`, based at their merge base. Re-tracking changes the parent.
    ///
    /// # Errors
    /// - [`Error::UnknownBranch`] if either branch doesn't exist.
    /// - [`Error::IsTrunk`] if `branch` is a trunk.
    /// - [`Error::UntrackedParent`] if `parent` is neither a trunk nor tracked.
    /// - [`Error::Cycle`] if `parent` is `branch` or stacked on it.
    /// - [`Error::Unrelated`] if they share no history.
    pub fn track(&self, branch: &str, parent: &str) -> Result<Outcome, Error> {
        let metadata = Metadata::load(&*self.git)?;
        let tip = self.require_branch(branch)?;
        let parent_tip = self.require_branch(parent)?;
        if metadata.is_trunk(branch) {
            return Err(Error::IsTrunk {
                name: branch.into(),
            });
        }
        if !metadata.is_trunk(parent) && !metadata.branches.contains_key(parent) {
            return Err(Error::UntrackedParent {
                name: parent.into(),
            });
        }
        if metadata.lineage(parent).contains(&branch) {
            return Err(Error::Cycle {
                branch: branch.into(),
                parent: parent.into(),
            });
        }
        let base = self
            .git
            .merge_base(&tip, &parent_tip)?
            .ok_or_else(|| Error::Unrelated {
                branch: branch.into(),
                parent: parent.into(),
            })?;
        let link = Link {
            parent: parent.into(),
            base,
        };
        let old = metadata.branches.get(branch);
        if old.is_some_and(|stored| stored.value == link) {
            return Ok(Outcome::Unchanged);
        }
        let blob = self.git.write_blob(&metadata::encode_link(&link))?;
        self.update(
            format!("{}{branch}", metadata::BRANCHES),
            old.map(|stored| stored.id.clone()),
            Some(blob),
        )
    }

    /// Forgets `branch`'s parent. The branch itself is untouched, and needn't still exist.
    ///
    /// # Errors
    /// [`Error::NotTracked`], or [`Error::HasChildren`] while branches are stacked on it.
    pub fn untrack(&self, branch: &str) -> Result<Outcome, Error> {
        let metadata = Metadata::load(&*self.git)?;
        let stored = metadata
            .branches
            .get(branch)
            .ok_or_else(|| Error::NotTracked {
                name: branch.into(),
            })?;
        require_childless(&metadata, branch)?;
        self.update(
            format!("{}{branch}", metadata::BRANCHES),
            Some(stored.id.clone()),
            None,
        )
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

struct TreeBuilder<'a> {
    git: &'a dyn GitRepo,
    metadata: &'a Metadata,
    current: Option<String>,
    visited: BTreeSet<String>,
}

impl TreeBuilder<'_> {
    fn node(&mut self, name: &str) -> Result<Node, Error> {
        self.visited.insert(name.to_owned());
        let mut children = Vec::new();
        for child in self.metadata.children(name) {
            if !self.visited.contains(&child) {
                children.push(self.node(&child)?);
            }
        }
        Ok(Node {
            name: name.to_owned(),
            commit: self.git.branch_tip(name)?,
            current: self.current.as_deref() == Some(name),
            children,
        })
    }
}

fn require_childless(metadata: &Metadata, name: &str) -> Result<(), Error> {
    let children = metadata.children(name);
    if children.is_empty() {
        Ok(())
    } else {
        Err(Error::HasChildren {
            name: name.into(),
            children,
        })
    }
}
