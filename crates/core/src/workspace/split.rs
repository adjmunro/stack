//! Splitting a branch into a stack.

use super::*;

impl Workspace {
    /// Splits `branch` into a stack by creating a branch at each of `points` (`(commit, name)`), which must be
    /// `branch`'s own commits. Auto-tracking then stacks them in commit order under `branch`. One journalled command,
    /// so undo removes them all.
    ///
    /// # Errors
    /// - [`Error::UnknownBranch`] or [`Error::UnknownRevision`].
    /// - [`Error::NotOwnCommit`] if a commit isn't one of `branch`'s own (or is its tip).
    /// - [`Error::BranchExists`] or [`Error::InvalidRefName`] for a new name.
    pub fn split(&self, branch: &str, points: &[(String, String)]) -> Result<Vec<String>, Error> {
        let (_, resolution) = self.resolve()?;
        let entry = resolution
            .branches
            .get(branch)
            .ok_or_else(|| Error::UnknownBranch {
                name: branch.into(),
            })?;
        let hidden: Vec<String> = entry
            .parent
            .iter()
            .map(|parent| parent.offshoot.clone())
            .collect();
        let own = self.git.commits_excluding(&entry.tip, &hidden)?;
        let mut updates = Vec::new();
        let mut created = Vec::new();
        for (revision, name) in points {
            let commit = self.git.resolve_commit(revision)?;
            if commit == entry.tip || !own.contains(&commit) {
                return Err(Error::NotOwnCommit {
                    branch: branch.into(),
                    revision: revision.clone(),
                });
            }
            if self.git.branch_tip(name)?.is_some() || created.contains(name) {
                return Err(Error::BranchExists { name: name.clone() });
            }
            updates.push(RefUpdate {
                name: format!("refs/heads/{name}"),
                old: None,
                new: Some(commit),
            });
            created.push(name.clone());
        }
        if !updates.is_empty() {
            let description = format!("split {branch} into {}", created.join(", "));
            self.journal.transact(
                &*self.git,
                OperationKind::Command,
                &description,
                None,
                updates,
            )?;
        }
        Ok(created)
    }
}
