//! Undo, redo, and the op log.

mod common;

use common::*;
use stack_core::{Error, OperationKind, OperationState, Source};
use stack_testkit::Fixture;

fn descriptions(fixture: &Fixture) -> Vec<String> {
    let operations = workspace(fixture).oplog(10).unwrap();
    operations
        .iter()
        .map(|operation| {
            let undone = if operation.undone { " (undone)" } else { "" };
            format!("{}{undone}", operation.description)
        })
        .collect()
}

#[test]
fn undo_reverts_exactly_the_last_command() {
    let fixture = repo();
    grow(&fixture, "a", "develop");
    let before = fixture.snapshot();
    workspace(&fixture).pin("a", None).unwrap();

    let undone = workspace(&fixture).undo().unwrap();

    assert_eq!(
        (undone.description.as_str(), undone.undone),
        ("pin a on develop", true)
    );
    let diff = before.diff(&fixture.snapshot());
    // Only the op log and the keep tree (now holding the pin's blob, for redo) differ.
    assert_eq!(diff.refs.keys().collect::<Vec<_>>(), ["refs/stack/keep"]);
    assert!(
        diff.head.is_none() && diff.index.is_empty() && diff.worktree.is_empty() && !diff.config,
        "{diff}"
    );
    assert_eq!(
        parent(&tree(&fixture), "a").unwrap().source,
        Source::Derived
    );
}

#[test]
fn undo_and_redo_walk_back_and_forth_in_order() {
    let fixture = repo();
    grow(&fixture, "a", "develop");
    grow(&fixture, "b", "a");
    let workspace = workspace(&fixture);
    workspace.pin("a", None).unwrap();
    workspace.pin("b", None).unwrap();

    assert_eq!(workspace.undo().unwrap().description, "pin b on a");
    assert_eq!(workspace.undo().unwrap().description, "pin a on develop");
    assert_eq!(workspace.redo().unwrap().description, "pin a on develop");
    assert_eq!(workspace.redo().unwrap().description, "pin b on a");
    assert!(matches!(workspace.redo(), Err(Error::NothingToRedo)));

    let tree = tree(&fixture);
    assert_eq!(parent(&tree, "a").unwrap().source, Source::Pinned);
    assert_eq!(parent(&tree, "b").unwrap().source, Source::Pinned);
    assert_eq!(
        descriptions(&fixture),
        [
            "redo #3: pin b on a",
            "redo #2: pin a on develop",
            "undo #2: pin a on develop",
            "undo #3: pin b on a",
            "pin b on a",
            "pin a on develop",
            "trunk add develop",
        ]
    );
}

#[test]
fn a_new_command_clears_redo() {
    let fixture = repo();
    grow(&fixture, "a", "develop");
    let workspace = workspace(&fixture);
    workspace.pin("a", None).unwrap();
    workspace.undo().unwrap();
    grow(&fixture, "b", "develop");
    workspace.pin("b", None).unwrap();

    assert!(matches!(workspace.redo(), Err(Error::NothingToRedo)));
    assert_eq!(workspace.undo().unwrap().description, "pin b on develop");
    // The earlier undo still stands; the next undo goes past it.
    assert_eq!(workspace.undo().unwrap().description, "trunk add develop");
    assert!(matches!(workspace.undo(), Err(Error::NothingToUndo)));
}

#[test]
fn undo_restores_metadata_after_gc() {
    let fixture = repo();
    grow(&fixture, "a", "develop");
    workspace(&fixture).pin("a", None).unwrap();
    workspace(&fixture).unpin("a").unwrap();
    fixture.git(&["gc", "--quiet", "--prune=now"]);

    workspace(&fixture).undo().unwrap();

    assert_eq!(parent(&tree(&fixture), "a").unwrap().source, Source::Pinned);
    fixture.git(&["fsck", "--no-progress"]);
}

#[test]
fn undo_refuses_to_overwrite_changes_made_outside_stack() {
    let fixture = repo();
    grow(&fixture, "a", "develop");
    workspace(&fixture).pin("a", None).unwrap();
    fixture.git(&["update-ref", "-d", "refs/stack/branches/a"]);
    let before = fixture.snapshot();

    let result = workspace(&fixture).undo();

    assert!(
        matches!(result, Err(Error::UndoConflict { ref reference }) if reference == "refs/stack/branches/a")
    );
    fixture.assert_unchanged(&before);
}

#[test]
fn nothing_to_undo_without_an_op_log_writes_nothing() {
    let fixture = Fixture::new();
    fixture.commit("base.txt", "base", "feat: base");
    let before = fixture.snapshot();

    assert!(matches!(
        workspace(&fixture).undo(),
        Err(Error::NothingToUndo)
    ));
    assert!(matches!(
        workspace(&fixture).redo(),
        Err(Error::NothingToRedo)
    ));
    assert!(workspace(&fixture).oplog(10).unwrap().is_empty());
    fixture.assert_unchanged(&before);
}

#[test]
fn oplog_records_kinds_states_and_changes() {
    let fixture = repo();
    grow(&fixture, "a", "develop");
    workspace(&fixture).pin("a", None).unwrap();
    workspace(&fixture).undo().unwrap();

    let operations = workspace(&fixture).oplog(10).unwrap();

    let [undo, pin, trunk] = operations.as_slice() else {
        panic!("{operations:?}")
    };
    assert_eq!(
        (undo.kind, undo.target, undo.state),
        (OperationKind::Undo, Some(pin.id), OperationState::Done)
    );
    assert_eq!((pin.kind, pin.undone), (OperationKind::Command, true));
    assert_eq!(trunk.changes.len(), 1);
    assert_eq!(trunk.changes[0].name, "refs/stack/trunks/develop");
    assert_eq!(trunk.changes[0].old, None);
}

#[test]
fn undo_to_takes_the_repository_back_to_before_a_command() {
    let fixture = repo();
    grow(&fixture, "a", "develop");
    grow(&fixture, "b", "a");
    fixture.git(&["switch", "--quiet", "develop"]);
    fixture.commit("later.txt", "later", "feat: later");
    let before = fixture.snapshot();
    let workspace = workspace(&fixture);
    workspace.pin("a", None).unwrap();
    workspace.pin("b", None).unwrap();
    workspace.restack("develop").unwrap();
    let first = workspace
        .oplog(10)
        .unwrap()
        .into_iter()
        .find(|operation| operation.description == "pin a on develop")
        .unwrap();

    let undone = workspace.undo_to(first.id).unwrap();

    let descriptions: Vec<&str> = undone
        .iter()
        .map(|operation| operation.description.as_str())
        .collect();
    assert_eq!(
        descriptions,
        ["restack develop", "pin b on a", "pin a on develop"]
    );
    let diff = before.diff(&fixture.snapshot());
    assert!(
        diff.refs.keys().all(|name| name == "refs/stack/keep"),
        "{diff}"
    );
    assert!(matches!(
        workspace.undo_to(first.id),
        Err(Error::NothingToUndo)
    ));
}
