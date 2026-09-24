//! Journalled ref transactions: record intent, apply one atomic ref transaction, mark complete. Also undo and redo.
//!
//! Objects that a transaction stops referencing from `refs/stack/` are added to the [`KEEP`] tree first, so undo can
//! restore them after `git gc`.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

use crate::git::{Checkout, GitRepo, RefUpdate};
use crate::metadata;
use crate::store::{Record, Store};
use crate::{
    Error, Operation, OperationKind, OperationState, Recovered, RecoveryOutcome, RefChange,
};

/// A tree of every metadata blob the op log may need to restore.
pub(crate) const KEEP: &str = "refs/stack/keep";

pub(crate) struct Journal {
    path: PathBuf,
    store: Mutex<Option<Store>>,
}

impl Journal {
    /// A journal stored at `path`. Nothing is created until the first transaction.
    pub(crate) fn new(path: PathBuf) -> Self {
        Self {
            path,
            store: Mutex::new(None),
        }
    }

    /// Resolves interrupted operations by comparing each ref with its recorded old and new values.
    pub(crate) fn recover(&self, git: &dyn GitRepo) -> Result<Vec<Recovered>, Error> {
        let Some(mut guard) = self.store(false)? else {
            return Ok(Vec::new());
        };
        let store = guard.as_mut().expect("store opened");
        let mut recovered = Vec::new();
        for record in store.pending()? {
            let outcome = resolve_pending(store, git, &record)?;
            recovered.push(Recovered {
                operation: record.id,
                description: record.description,
                outcome,
            });
        }
        Ok(recovered)
    }

    /// Applies `updates` atomically as a journalled operation. Returns its id.
    ///
    /// If the checked-out branch moves, the index and working tree move first (see [`GitRepo::checkout`]), and back
    /// again if the refs then fail to update.
    ///
    /// # Errors
    /// - [`Error::DirtyWorktree`] or [`Error::DeletesCheckedOutBranch`] if the checked-out branch is involved.
    /// - [`Error::CheckedOutElsewhere`] if a branch it moves is checked out in another worktree.
    /// - Whatever the checkout or ref transaction returns; the operation is then marked done or failed to match.
    pub(crate) fn transact(
        &self,
        git: &dyn GitRepo,
        kind: OperationKind,
        description: &str,
        target: Option<i64>,
        updates: Vec<RefUpdate>,
    ) -> Result<i64, Error> {
        self.transact_with(git, kind, description, target, updates, None)
    }

    /// As [`Self::transact`], but `holder` names another worktree that has a branch checked out: instead of refusing
    /// to move that branch, its worktree's files move with it (carrying local changes that don't clash).
    pub(crate) fn transact_with(
        &self,
        git: &dyn GitRepo,
        kind: OperationKind,
        description: &str,
        target: Option<i64>,
        mut updates: Vec<RefUpdate>,
        holder: Option<(&str, Checkout)>,
    ) -> Result<i64, Error> {
        let checkout = match holder {
            Some((branch, checkout)) => {
                plan_checkout(git, &updates, Some(branch))?;
                Some(checkout)
            }
            None => plan_checkout(git, &updates, None)?,
        };
        updates.extend(keep_update(git, &updates)?);
        let mut guard = self.store(true)?.expect("store created");
        let store = guard.as_mut().expect("store opened");
        let id = store.begin(kind, description, target, &updates, checkout.as_ref())?;
        if let Some(checkout) = &checkout {
            if let Err(error) =
                git.checkout(checkout.worktree.as_deref(), &checkout.from, &checkout.to)
            {
                store.finish(id, OperationState::Failed)?;
                return Err(error);
            }
        }
        match git.update_refs(&updates, &format!("stack: {description}")) {
            Ok(()) => {
                store.finish(id, OperationState::Done)?;
                Ok(id)
            }
            Err(error) => {
                let record = Record {
                    id,
                    kind,
                    description: description.to_owned(),
                    target,
                    state: OperationState::Pending,
                    undone: false,
                    started_at: 0,
                    updates,
                    checkout,
                };
                resolve_pending(store, git, &record)?;
                Err(error)
            }
        }
    }

    /// Reverts the latest command that isn't undone. Returns that command.
    ///
    /// # Errors
    /// [`Error::NothingToUndo`], or [`Error::UndoConflict`] if a ref it changed has changed again since.
    pub(crate) fn undo(&self, git: &dyn GitRepo) -> Result<Operation, Error> {
        let target = self
            .read(|store| store.undo_target())?
            .flatten()
            .ok_or(Error::NothingToUndo)?;
        let updates = revertible(&target)
            .map(|update| RefUpdate {
                name: update.name.clone(),
                old: update.new.clone(),
                new: update.old.clone(),
            })
            .collect();
        self.reapply(git, OperationKind::Undo, &target, updates)
    }

    /// Re-applies the most recently undone command, if no command has run since. Returns that command.
    ///
    /// # Errors
    /// [`Error::NothingToRedo`], or [`Error::UndoConflict`] if a ref it changed has changed again since.
    pub(crate) fn redo(&self, git: &dyn GitRepo) -> Result<Operation, Error> {
        let target = self
            .read(|store| store.redo_target())?
            .flatten()
            .ok_or(Error::NothingToRedo)?;
        let updates = revertible(&target).cloned().collect();
        self.reapply(git, OperationKind::Redo, &target, updates)
    }

    /// Runs `write` against the store, creating it if needed. For local state outside the op log (review marks).
    pub(crate) fn with_store<T>(
        &self,
        write: impl FnOnce(&Store) -> Result<T, Error>,
    ) -> Result<T, Error> {
        let guard = self.store(true)?.expect("store created");
        write(guard.as_ref().expect("store opened"))
    }

    /// Runs `query` against the store if it exists.
    pub(crate) fn query<T>(
        &self,
        query: impl FnOnce(&Store) -> Result<T, Error>,
    ) -> Result<Option<T>, Error> {
        self.read(query)
    }

    /// The most recent operations, newest first.
    pub(crate) fn recent(&self, limit: usize) -> Result<Vec<Operation>, Error> {
        let records = self.read(|store| store.recent(limit))?.unwrap_or_default();
        Ok(records.into_iter().map(operation).collect())
    }

    fn reapply(
        &self,
        git: &dyn GitRepo,
        kind: OperationKind,
        target: &Record,
        updates: Vec<RefUpdate>,
    ) -> Result<Operation, Error> {
        for update in &updates {
            if git.ref_value(&update.name)? != update.old {
                return Err(Error::UndoConflict {
                    reference: update.name.clone(),
                });
            }
        }
        let verb = if kind == OperationKind::Undo {
            "undo"
        } else {
            "redo"
        };
        let description = format!("{verb} #{}: {}", target.id, target.description);
        self.transact(git, kind, &description, Some(target.id), updates)?;
        let mut target = operation(target.clone());
        target.undone = kind == OperationKind::Undo;
        Ok(target)
    }

    fn read<T>(&self, query: impl FnOnce(&Store) -> Result<T, Error>) -> Result<Option<T>, Error> {
        match self.store(false)? {
            Some(guard) => Ok(Some(query(guard.as_ref().expect("store opened"))?)),
            None => Ok(None),
        }
    }

    /// The open store, opening it first if it exists (or `create` is set). `None` if there is no store.
    fn store(&self, create: bool) -> Result<Option<MutexGuard<'_, Option<Store>>>, Error> {
        let mut guard = self
            .store
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if guard.is_none() && (create || self.path.exists()) {
            *guard = Some(Store::open(&self.path)?);
        }
        Ok(guard.is_some().then_some(guard))
    }
}

/// The move of this worktree needed because `updates` move its checked-out branch, if they do. Refuses branches
/// checked out in other worktrees, except `held_elsewhere`, whose move the caller arranges.
fn plan_checkout(
    git: &dyn GitRepo,
    updates: &[RefUpdate],
    held_elsewhere: Option<&str>,
) -> Result<Option<Checkout>, Error> {
    let mut elsewhere = git.checked_out_elsewhere()?;
    elsewhere.retain(|branch| Some(branch.as_str()) != held_elsewhere);
    if let Some(update) = updates.iter().find(|update| {
        update
            .name
            .strip_prefix("refs/heads/")
            .is_some_and(|branch| elsewhere.iter().any(|other| other == branch))
    }) {
        let branch = update.name.trim_start_matches("refs/heads/").to_owned();
        return Err(Error::CheckedOutElsewhere { branch });
    }
    let crate::Head::Branch {
        name,
        commit: Some(commit),
    } = git.head()?
    else {
        return Ok(None);
    };
    let Some(update) = updates
        .iter()
        .find(|update| update.name == format!("refs/heads/{name}"))
    else {
        return Ok(None);
    };
    let Some(new) = &update.new else {
        return Err(Error::DeletesCheckedOutBranch { branch: name });
    };
    if update.old.as_deref() != Some(commit.as_str()) || *new == commit {
        return Ok(None);
    }
    if !git.is_worktree_clean(None)? {
        return Err(Error::DirtyWorktree { branch: name });
    }
    Ok(Some(Checkout {
        worktree: None,
        from: commit,
        to: new.clone(),
    }))
}

/// An update to [`KEEP`] adding every metadata blob `updates` stop referencing, if any are new to it. Other refs
/// (branches, archives) hold commits, which their reflogs or the refs themselves protect.
fn keep_update(git: &dyn GitRepo, updates: &[RefUpdate]) -> Result<Option<RefUpdate>, Error> {
    let current = git.ref_value(KEEP)?;
    let mut kept: BTreeSet<String> = match &current {
        Some(tree) => git.tree_entries(tree)?.into_iter().collect(),
        None => BTreeSet::new(),
    };
    let before = kept.len();
    kept.extend(
        updates
            .iter()
            .filter(|update| {
                (update.name.starts_with(metadata::TRUNKS)
                    || update.name.starts_with(metadata::BRANCHES))
                    && update.old != update.new
            })
            .filter_map(|update| update.old.clone()),
    );
    if kept.len() == before {
        return Ok(None);
    }
    let tree = git.write_blob_tree(&kept.into_iter().collect::<Vec<_>>())?;
    Ok(Some(RefUpdate {
        name: KEEP.into(),
        old: current,
        new: Some(tree),
    }))
}

/// The updates undo and redo act on: everything but [`KEEP`], which only ever grows.
fn revertible(record: &Record) -> impl Iterator<Item = &RefUpdate> {
    record.updates.iter().filter(|update| update.name != KEEP)
}

fn resolve_pending(
    store: &mut Store,
    git: &dyn GitRepo,
    record: &Record,
) -> Result<RecoveryOutcome, Error> {
    let mut all_new = true;
    let mut all_old = true;
    for update in &record.updates {
        let value = git.ref_value(&update.name)?;
        all_new &= value == update.new;
        all_old &= value == update.old;
    }
    Ok(if all_new {
        store.finish(record.id, OperationState::Done)?;
        RecoveryOutcome::Completed
    } else if all_old {
        // The working tree may already have moved ahead of the refs; move it back.
        if let Some(checkout) = &record.checkout {
            let worktree = checkout.worktree.as_deref();
            let moved = git.index_matches(worktree, &checkout.to)?;
            if moved
                && git
                    .checkout(worktree, &checkout.to, &checkout.from)
                    .is_err()
            {
                return Ok(RecoveryOutcome::Inconsistent);
            }
        }
        store.finish(record.id, OperationState::Failed)?;
        RecoveryOutcome::RolledBack
    } else {
        RecoveryOutcome::Inconsistent
    })
}

fn operation(record: Record) -> Operation {
    Operation {
        id: record.id,
        kind: record.kind,
        description: record.description,
        target: record.target,
        state: record.state,
        undone: record.undone,
        started_at: record.started_at,
        changes: record
            .updates
            .into_iter()
            .map(|update| RefChange {
                name: update.name,
                old: update.old,
                new: update.new,
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::path::{Path, PathBuf};

    use stack_testkit::{Fixture, Snapshot};

    use super::*;
    use crate::git::GixRepo;

    /// Delegates to gix, but "crashes" (panics) around the ref transaction.
    struct Crashing {
        inner: GixRepo,
        apply_refs: bool,
    }

    impl GitRepo for Crashing {
        fn common_dir(&self) -> PathBuf {
            self.inner.common_dir()
        }
        fn head(&self) -> Result<crate::Head, Error> {
            self.inner.head()
        }
        fn ref_value(&self, name: &str) -> Result<Option<String>, Error> {
            self.inner.ref_value(name)
        }
        fn tree_entries(&self, tree: &str) -> Result<Vec<String>, Error> {
            self.inner.tree_entries(tree)
        }
        fn write_blob_tree(&self, blobs: &[String]) -> Result<String, Error> {
            self.inner.write_blob_tree(blobs)
        }
        fn remote_default_branch(&self, remote: &str) -> Result<Option<String>, Error> {
            self.inner.remote_default_branch(remote)
        }
        fn branch_tip(&self, name: &str) -> Result<Option<String>, Error> {
            self.inner.branch_tip(name)
        }
        fn branches(&self) -> Result<Vec<crate::git::Branch>, Error> {
            self.inner.branches()
        }
        fn merge_base(&self, one: &str, two: &str) -> Result<Option<String>, Error> {
            self.inner.merge_base(one, two)
        }
        fn reachable_excluding(
            &self,
            tips: &[String],
            hidden: &[String],
        ) -> Result<Vec<String>, Error> {
            self.inner.reachable_excluding(tips, hidden)
        }
        fn patch_ids(
            &self,
            commits: &[String],
        ) -> Result<std::collections::HashMap<String, String>, Error> {
            self.inner.patch_ids(commits)
        }
        fn touches(&self, tip: &str, hidden: &[String], paths: &[String]) -> Result<bool, Error> {
            self.inner.touches(tip, hidden, paths)
        }
        fn blob_refs(&self, prefix: &str) -> Result<Vec<crate::git::BlobRef>, Error> {
            self.inner.blob_refs(prefix)
        }
        fn refs(&self, prefix: &str) -> Result<Vec<(String, String)>, Error> {
            self.inner.refs(prefix)
        }
        fn reflog(&self, reference: &str) -> Result<Vec<crate::git::ReflogEntry>, Error> {
            self.inner.reflog(reference)
        }
        fn commit_tips(&self) -> Result<Vec<String>, Error> {
            self.inner.commit_tips()
        }
        fn write_blob(&self, data: &[u8]) -> Result<String, Error> {
            self.inner.write_blob(data)
        }
        fn commit(&self, id: &str) -> Result<crate::git::CommitInfo, Error> {
            self.inner.commit(id)
        }
        fn resolve_commit(&self, revision: &str) -> Result<String, Error> {
            self.inner.resolve_commit(revision)
        }
        fn merge_trees(
            &self,
            base: &str,
            ours: &str,
            theirs: &str,
        ) -> Result<crate::git::Merge, Error> {
            self.inner.merge_trees(base, ours, theirs)
        }
        fn copy_commit(&self, original: &str, tree: &str, parent: &str) -> Result<String, Error> {
            self.inner.copy_commit(original, tree, parent)
        }
        fn is_worktree_clean(&self, worktree: Option<&Path>) -> Result<bool, Error> {
            self.inner.is_worktree_clean(worktree)
        }
        fn index_matches(&self, worktree: Option<&Path>, commit: &str) -> Result<bool, Error> {
            self.inner.index_matches(worktree, commit)
        }
        fn checkout(&self, worktree: Option<&Path>, from: &str, to: &str) -> Result<(), Error> {
            self.inner.checkout(worktree, from, to)
        }
        fn worktrees(&self) -> Result<Vec<crate::git::WorktreeInfo>, Error> {
            self.inner.worktrees()
        }
        fn add_worktree(
            &self,
            path: &Path,
            branch: Option<&str>,
            detached: Option<&str>,
        ) -> Result<(), Error> {
            self.inner.add_worktree(path, branch, detached)
        }
        fn set_detached_head(&self, worktree: &Path, commit: &str) -> Result<(), Error> {
            self.inner.set_detached_head(worktree, commit)
        }
        fn remotes(&self) -> Result<Vec<String>, Error> {
            self.inner.remotes()
        }
        fn config_value(&self, key: &str) -> Result<Option<String>, Error> {
            self.inner.config_value(key)
        }
        fn config_values(&self, key: &str) -> Result<Vec<String>, Error> {
            self.inner.config_values(key)
        }
        fn hooks_dir(&self) -> Result<PathBuf, Error> {
            self.inner.hooks_dir()
        }
        fn push(
            &self,
            remote: &str,
            branches: &[crate::git::PushRef],
        ) -> Result<Vec<crate::git::PushStatus>, Error> {
            self.inner.push(remote, branches)
        }
        fn set_upstream(&self, branch: &str, remote: &str) -> Result<(), Error> {
            self.inner.set_upstream(branch, remote)
        }
        fn quarantine(&self, objects: Option<&Path>) {
            self.inner.quarantine(objects)
        }
        fn rebase(&self, onto: &str, upstream: &str, branch: &str) -> Result<bool, Error> {
            self.inner.rebase(onto, upstream, branch)
        }
        fn rebase_in_progress(&self) -> Result<bool, Error> {
            self.inner.rebase_in_progress()
        }
        fn rebase_continue(&self) -> Result<bool, Error> {
            self.inner.rebase_continue()
        }
        fn rebase_abort(&self) -> Result<(), Error> {
            self.inner.rebase_abort()
        }
        fn unmerged_paths(&self) -> Result<Vec<String>, Error> {
            self.inner.unmerged_paths()
        }
        fn switch(&self, branch: &str, create: bool) -> Result<(), Error> {
            self.inner.switch(branch, create)
        }
        fn commit_index(&self, message: &str, all: bool) -> Result<(), Error> {
            self.inner.commit_index(message, all)
        }
        fn update_refs(&self, updates: &[RefUpdate], message: &str) -> Result<(), Error> {
            if self.apply_refs {
                self.inner.update_refs(updates, message)?;
            }
            panic!("simulated crash");
        }
    }

    fn gix(fixture: &Fixture) -> GixRepo {
        GixRepo::discover(
            &fixture.path(),
            crate::Environment::Exactly(fixture.environment()),
        )
        .unwrap()
    }

    fn journal_path(fixture: &Fixture) -> PathBuf {
        fixture.path().join(".git/stack/stack.db")
    }

    /// A fixture with a trunk mark `develop` written through the journal, so the store exists.
    fn fixture_with_store() -> (Fixture, String) {
        let fixture = Fixture::new();
        fixture.commit("base.txt", "base", "feat: base");
        let git = gix(&fixture);
        let blob = git
            .write_blob(b"{\"version\":1,\"role\":\"trunk\"}\n")
            .unwrap();
        let journal = Journal::new(journal_path(&fixture));
        let update = RefUpdate {
            name: "refs/stack/trunks/develop".into(),
            old: None,
            new: Some(blob.clone()),
        };
        journal
            .transact(
                &git,
                OperationKind::Command,
                "trunk add develop",
                None,
                vec![update],
            )
            .unwrap();
        (fixture, blob)
    }

    /// Crashes a transaction deleting the develop mark, then recovers with a fresh journal.
    fn crash_then_recover(apply_refs: bool) -> (Fixture, Snapshot, Vec<Recovered>) {
        let (fixture, blob) = fixture_with_store();
        let before = fixture.snapshot();
        let crashing = Crashing {
            inner: gix(&fixture),
            apply_refs,
        };
        let journal = Journal::new(journal_path(&fixture));
        let delete = RefUpdate {
            name: "refs/stack/trunks/develop".into(),
            old: Some(blob),
            new: None,
        };
        let crashed = catch_unwind(AssertUnwindSafe(|| {
            journal.transact(
                &crashing,
                OperationKind::Command,
                "trunk remove develop",
                None,
                vec![delete],
            )
        }));
        assert!(crashed.is_err(), "transaction should have crashed");

        let recovered = Journal::new(journal_path(&fixture))
            .recover(&gix(&fixture))
            .unwrap();
        (fixture, before, recovered)
    }

    fn ref_names(fixture: &Fixture) -> String {
        fixture.git(&["for-each-ref", "--format=%(refname)", "refs/stack/"])
    }

    #[test]
    fn crash_before_refs_change_rolls_back() {
        let (fixture, before, recovered) = crash_then_recover(false);

        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered[0].outcome, RecoveryOutcome::RolledBack);
        let diff = before.diff(&fixture.snapshot());
        // Only the journal and unreferenced objects (the keep tree) changed.
        assert!(
            diff.refs.is_empty() && diff.head.is_none() && diff.index.is_empty(),
            "{diff}"
        );
        assert_eq!(ref_names(&fixture), "refs/stack/trunks/develop");
    }

    #[test]
    fn crash_after_refs_change_completes() {
        let (fixture, _, recovered) = crash_then_recover(true);

        assert_eq!(recovered[0].outcome, RecoveryOutcome::Completed);
        assert_eq!(ref_names(&fixture), "refs/stack/keep");
        let store = Store::open(&journal_path(&fixture)).unwrap();
        assert_eq!(
            store.state(recovered[0].operation).unwrap(),
            Some(OperationState::Done)
        );
    }

    #[test]
    fn recovery_is_idempotent() {
        let (fixture, _, _) = crash_then_recover(true);
        let before = fixture.snapshot();

        assert!(
            Journal::new(journal_path(&fixture))
                .recover(&gix(&fixture))
                .unwrap()
                .is_empty()
        );
        fixture.assert_unchanged(&before);
    }

    #[test]
    fn mixed_state_is_left_pending() {
        let (fixture, blob) = fixture_with_store();
        let crashing = Crashing {
            inner: gix(&fixture),
            apply_refs: false,
        };
        let journal = Journal::new(journal_path(&fixture));
        let delete = RefUpdate {
            name: "refs/stack/trunks/develop".into(),
            old: Some(blob),
            new: None,
        };
        let _ = catch_unwind(AssertUnwindSafe(|| {
            journal.transact(
                &crashing,
                OperationKind::Command,
                "trunk remove develop",
                None,
                vec![delete],
            )
        }));
        // Someone else points the ref somewhere neither old nor new.
        fixture.git(&["update-ref", "refs/stack/trunks/develop", "develop"]);

        let recovered = Journal::new(journal_path(&fixture))
            .recover(&gix(&fixture))
            .unwrap();
        assert_eq!(recovered[0].outcome, RecoveryOutcome::Inconsistent);
        let again = Journal::new(journal_path(&fixture))
            .recover(&gix(&fixture))
            .unwrap();
        assert_eq!(again, recovered);
    }

    #[test]
    fn crash_after_moving_the_working_tree_moves_it_back() {
        let fixture = Fixture::new();
        let old = fixture.commit("base.txt", "base", "feat: base");
        fixture.git(&["switch", "--quiet", "--create", "other"]);
        let new = fixture.commit("other.txt", "other", "feat: other");
        fixture.git(&["switch", "--quiet", "develop"]);
        let before = fixture.snapshot();
        let crashing = Crashing {
            inner: gix(&fixture),
            apply_refs: false,
        };
        let journal = Journal::new(journal_path(&fixture));
        let update = RefUpdate {
            name: "refs/heads/develop".into(),
            old: Some(old),
            new: Some(new),
        };
        let crashed = catch_unwind(AssertUnwindSafe(|| {
            journal.transact(
                &crashing,
                OperationKind::Command,
                "move develop",
                None,
                vec![update],
            )
        }));
        assert!(crashed.is_err());
        assert!(
            fixture.path().join("other.txt").exists(),
            "working tree moved before the crash"
        );

        let recovered = Journal::new(journal_path(&fixture))
            .recover(&gix(&fixture))
            .unwrap();

        assert_eq!(recovered[0].outcome, RecoveryOutcome::RolledBack);
        let diff = before.diff(&fixture.snapshot());
        assert!(
            diff.refs.is_empty() && diff.index.is_empty() && diff.worktree.is_empty(),
            "{diff}"
        );
    }

    #[test]
    fn crash_after_moving_another_worktree_moves_it_back() {
        let fixture = Fixture::new();
        let old = fixture.commit("base.txt", "base", "feat: base");
        fixture.git(&["switch", "--quiet", "--create", "other"]);
        let new = fixture.commit("other.txt", "other", "feat: other");
        // develop is checked out in a second worktree; this one stays on other.
        let holder = fixture.scratch_path("holder");
        fixture.git(&[
            "worktree",
            "add",
            "--quiet",
            holder.to_str().unwrap(),
            "develop",
        ]);
        let crashing = Crashing {
            inner: gix(&fixture),
            apply_refs: false,
        };
        let journal = Journal::new(journal_path(&fixture));
        let update = RefUpdate {
            name: "refs/heads/develop".into(),
            old: Some(old.clone()),
            new: Some(new.clone()),
        };
        let checkout = Checkout {
            worktree: Some(holder.clone()),
            from: old,
            to: new,
        };
        let crashed = catch_unwind(AssertUnwindSafe(|| {
            let held = Some(("develop", checkout));
            journal.transact_with(
                &crashing,
                OperationKind::Command,
                "land",
                None,
                vec![update],
                held,
            )
        }));
        assert!(crashed.is_err());
        assert!(
            holder.join("other.txt").exists(),
            "holder moved before the crash"
        );

        let recovered = Journal::new(journal_path(&fixture))
            .recover(&gix(&fixture))
            .unwrap();

        assert_eq!(recovered[0].outcome, RecoveryOutcome::RolledBack);
        assert!(!holder.join("other.txt").exists());
        assert_eq!(fixture.git_in(&holder, &["status", "--porcelain"]), "");
    }

    #[test]
    fn no_store_means_nothing_to_recover_and_nothing_written() {
        let fixture = Fixture::new();
        let before = fixture.snapshot();

        assert!(
            Journal::new(journal_path(&fixture))
                .recover(&gix(&fixture))
                .unwrap()
                .is_empty()
        );
        assert!(!Path::new(&journal_path(&fixture)).exists());
        fixture.assert_unchanged(&before);
    }
}
