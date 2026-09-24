//! Review marks.

use super::*;

/// The store key for a commit's marks: its patch-id, or the commit itself if it has none (merges, empty commits).
fn mark_key(commit: &str, patch_ids: &std::collections::HashMap<String, String>) -> String {
    match patch_ids.get(commit) {
        Some(patch) => format!("patch:{patch}"),
        None => format!("commit:{commit}"),
    }
}

impl Workspace {
    /// Marks the commit `revision` names. The mark follows the commit's change (its patch-id) through rebases and
    /// amends that leave the change alone, and lapses once the change itself changes. Merge and empty commits have no
    /// patch-id, so their marks stay with the exact commit. Replaces an existing mark of the same kind.
    ///
    /// Marks are personal: they live in the local store, not in refs, and aren't in the op log.
    ///
    /// # Errors
    /// [`Error::UnknownRevision`].
    pub fn mark(&self, revision: &str, kind: MarkKind, note: Option<&str>) -> Result<(), Error> {
        let key = self.mark_key(revision)?;
        self.journal
            .with_store(|store| store.set_mark(&key, kind, note))
    }

    /// Removes the marks of `kind` (or all marks) from the commit `revision` names. Returns how many were removed.
    ///
    /// # Errors
    /// [`Error::UnknownRevision`].
    pub fn unmark(&self, revision: &str, kind: Option<MarkKind>) -> Result<usize, Error> {
        let key = self.mark_key(revision)?;
        Ok(self
            .journal
            .query(|store| store.clear_marks(&key, kind))?
            .unwrap_or(0))
    }

    /// `branch`'s own commits (those after its offshoot, or its whole history if it has no parent), newest first,
    /// with their marks.
    ///
    /// # Errors
    /// [`Error::UnknownBranch`].
    pub fn review(&self, branch: &str) -> Result<Vec<CommitReview>, Error> {
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
        let commits = self.git.commits_excluding(&entry.tip, &hidden)?;
        let patch_ids = self.git.patch_ids(&commits)?;
        commits
            .into_iter()
            .map(|commit| {
                let key = mark_key(&commit, &patch_ids);
                let marks = self
                    .journal
                    .query(|store| store.marks(&key))?
                    .unwrap_or_default();
                Ok(CommitReview {
                    summary: self.git.commit(&commit)?.summary,
                    commit,
                    marks,
                })
            })
            .collect()
    }

    fn mark_key(&self, revision: &str) -> Result<String, Error> {
        let commit = self.git.resolve_commit(revision)?;
        let patch_ids = self.git.patch_ids(std::slice::from_ref(&commit))?;
        Ok(mark_key(&commit, &patch_ids))
    }
}
