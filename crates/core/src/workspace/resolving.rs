//! Resolving restack conflicts with git's own rebase, then carrying on.

use serde::Deserialize;

use super::*;

/// The store key of a restack waiting on `stack continue`.
const RESOLVING: &str = "resolving";

impl Workspace {
    /// Starts resolving `conflict` (from restacking `target`) with git's own rebase: checks the conflicting branch
    /// out and runs `git rebase --onto <parent> <offshoot> <branch>`, so the usual tools (and IDEs) handle the
    /// conflict. Once it's resolved and staged, [`Self::continue_restack`] finishes the rebase and restacks `target`
    /// again, moving everything on the branch along. If git resolves it unaided, that happens straight away.
    ///
    /// The rebase itself isn't in the op log; `git reflog` has it.
    ///
    /// # Errors
    /// - [`Error::AlreadyResolving`] if another restack is waiting.
    /// - [`Error::DirtyWorktree`], or [`Error::CheckedOutElsewhere`] for the conflicting branch.
    pub fn resolve_conflict(
        &self,
        target: &str,
        conflict: &Conflict,
    ) -> Result<ResolveOutcome, Error> {
        if let Some(waiting) = self.resolving()? {
            return Err(Error::AlreadyResolving {
                branch: waiting.branch,
            });
        }
        if !self.git.is_worktree_clean(None)? {
            let branch = self.current_branch()?.unwrap_or_else(|| "HEAD".into());
            return Err(Error::DirtyWorktree { branch });
        }
        if self.git.checked_out_elsewhere()?.contains(&conflict.branch) {
            return Err(Error::CheckedOutElsewhere {
                branch: conflict.branch.clone(),
            });
        }
        let waiting = Resolving {
            target: target.into(),
            branch: conflict.branch.clone(),
            onto: conflict.onto.clone(),
        };
        let json = serde_json::to_string(&waiting).expect("state serialises");
        self.journal
            .with_store(|store| store.set_state(RESOLVING, Some(&json)))?;
        if self
            .git
            .rebase(&conflict.onto, &conflict.offshoot, &conflict.branch)?
        {
            self.finish_resolving(waiting)
        } else {
            Ok(ResolveOutcome::Stopped {
                branch: waiting.branch,
                paths: self.git.unmerged_paths()?,
            })
        }
    }

    /// Carries on after the conflict is resolved and staged: `git rebase --continue` (if the rebase is still going),
    /// then restacks the original target again.
    ///
    /// # Errors
    /// [`Error::NothingToContinue`], or [`Error::UnresolvedConflicts`] if the conflict isn't resolved yet.
    pub fn continue_restack(&self) -> Result<ResolveOutcome, Error> {
        let waiting = self.resolving()?.ok_or(Error::NothingToContinue)?;
        if self.git.rebase_in_progress()? && !self.git.rebase_continue()? {
            return Ok(ResolveOutcome::Stopped {
                branch: waiting.branch,
                paths: self.git.unmerged_paths()?,
            });
        }
        self.finish_resolving(waiting)
    }

    /// Gives up on the waiting restack: aborts the rebase if it's still going. Branches already restacked stay
    /// restacked (undo reverts them).
    ///
    /// # Errors
    /// [`Error::NothingToContinue`].
    pub fn abort_restack(&self) -> Result<Resolving, Error> {
        let waiting = self.resolving()?.ok_or(Error::NothingToContinue)?;
        if self.git.rebase_in_progress()? {
            self.git.rebase_abort()?;
        }
        self.journal
            .with_store(|store| store.set_state(RESOLVING, None))?;
        Ok(waiting)
    }

    /// The restack waiting on `stack continue`, if any.
    pub fn resolving(&self) -> Result<Option<Resolving>, Error> {
        let Some(json) = self
            .journal
            .query(|store| store.state_value(RESOLVING))?
            .flatten()
        else {
            return Ok(None);
        };
        serde_json::from_str(&json)
            .map(Some)
            .map_err(|error| Error::store(error.to_string()))
    }

    fn finish_resolving(&self, waiting: Resolving) -> Result<ResolveOutcome, Error> {
        self.journal
            .with_store(|store| store.set_state(RESOLVING, None))?;
        // A pinned branch that git just moved (e.g. by `stack move`) keeps its pin, on the new parent.
        let metadata = Metadata::load(&*self.git)?;
        if metadata
            .links
            .get(&waiting.branch)
            .is_some_and(|stored| stored.value.pinned)
        {
            self.pin(&waiting.branch, Some(&waiting.onto))?;
        }
        Ok(ResolveOutcome::Finished(self.restack(&waiting.target)?))
    }
}

/// A restack waiting for a conflict to be resolved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resolving {
    /// The branch the restack started from, restacked again on continue.
    pub target: String,
    /// The branch being rebased by git.
    pub branch: String,
    /// The parent it's being rebased onto.
    pub onto: String,
}

/// Where resolving a restack conflict got to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ResolveOutcome {
    /// git stopped at a conflict: resolve `paths`, `git add` them, then continue.
    Stopped { branch: String, paths: Vec<String> },
    /// The rebase finished and the restack carried on.
    Finished(Restacked),
}
