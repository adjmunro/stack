//! Archiving branches.

use super::*;

impl Workspace {
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
}
