//! Moving a branch onto a different parent.

mod common;

use common::*;
use stack_core::{Error, Source};
use stack_testkit::Fixture;

/// `develop` ← `a` ← `b` ← `c`, and `develop` ← `x`. HEAD on `develop`.
fn stacks() -> Fixture {
    let fixture = repo();
    grow(&fixture, "a", "develop");
    grow(&fixture, "b", "a");
    grow(&fixture, "c", "b");
    grow(&fixture, "x", "develop");
    fixture.git(&["switch", "--quiet", "develop"]);
    fixture
}

fn subjects(fixture: &Fixture, range: &str) -> String {
    fixture.git(&["log", "--format=%s", range])
}

#[test]
fn moves_a_branch_and_its_descendants_onto_another_parent() {
    let fixture = stacks();
    let a = tip(&fixture, "a");

    let restacked = workspace(&fixture).move_branch("b", "x").unwrap();

    let moved: Vec<(&str, &str)> = restacked
        .moved
        .iter()
        .map(|moved| (moved.name.as_str(), moved.onto.as_str()))
        .collect();
    assert_eq!(moved, [("b", "x"), ("c", "b")]);
    assert_eq!(
        subjects(&fixture, "develop..c"),
        "feat: c\nfeat: b\nfeat: x"
    );
    assert_eq!(tip(&fixture, "a"), a);
    let tree = tree(&fixture);
    assert_eq!(shape(&tree), "develop(a x(b(c)))");
    assert_eq!(parent(&tree, "b").unwrap().source, Source::Recorded);
    assert_eq!(
        workspace(&fixture).oplog(1).unwrap()[0].description,
        "move b onto x"
    );
}

#[test]
fn moving_onto_a_trunk_unstacks_the_branch() {
    let fixture = stacks();

    workspace(&fixture).move_branch("b", "develop").unwrap();

    assert_eq!(shape(&tree(&fixture)), "develop(a b(c) x)");
    assert_eq!(subjects(&fixture, "develop..b"), "feat: b");
}

#[test]
fn a_pin_follows_the_move() {
    let fixture = stacks();
    workspace(&fixture).pin("b", None).unwrap();

    workspace(&fixture).move_branch("b", "x").unwrap();

    let b = parent(&tree(&fixture), "b").unwrap();
    assert_eq!(
        (b.name.as_str(), b.source, b.contradicted),
        ("x", Source::Pinned, false)
    );
}

#[test]
fn undo_moves_it_back() {
    let fixture = stacks();
    let before = fixture.snapshot();
    workspace(&fixture).move_branch("b", "x").unwrap();

    workspace(&fixture).undo().unwrap();

    let diff = before.diff(&fixture.snapshot());
    assert!(
        diff.refs.keys().all(|name| name == "refs/stack/keep"),
        "{diff}"
    );
    assert_eq!(shape(&tree(&fixture)), "develop(a(b(c)) x)");
}

#[test]
fn invalid_moves_change_nothing() {
    let fixture = stacks();
    let before = fixture.snapshot();
    let workspace = workspace(&fixture);

    assert!(matches!(
        workspace.move_branch("b", "c"),
        Err(Error::Cycle { .. })
    ));
    assert!(matches!(
        workspace.move_branch("b", "b"),
        Err(Error::Cycle { .. })
    ));
    assert!(matches!(
        workspace.move_branch("develop", "x"),
        Err(Error::IsTrunk { .. })
    ));
    assert!(matches!(
        workspace.move_branch("b", "nope"),
        Err(Error::UnknownBranch { .. })
    ));
    assert!(matches!(
        workspace.move_branch("nope", "x"),
        Err(Error::UnknownBranch { .. })
    ));
    fixture.assert_unchanged(&before);
}

#[test]
fn moving_several_branches_onto_one_parent_makes_them_siblings() {
    let fixture = stacks();

    workspace(&fixture)
        .move_branches(&["b", "c"], "develop")
        .unwrap();

    assert_eq!(shape(&tree(&fixture)), "develop(a b c x)");
    assert_eq!(subjects(&fixture, "develop..c"), "feat: c");
}

#[test]
fn chaining_lines_branches_up_in_order() {
    let fixture = stacks();

    workspace(&fixture).chain(&["x", "a"]).unwrap();

    assert_eq!(shape(&tree(&fixture)), "develop(x(a(b(c))))");
    assert_eq!(
        subjects(&fixture, "develop..c"),
        "feat: c\nfeat: b\nfeat: a\nfeat: x"
    );
}

#[test]
fn a_chain_that_would_loop_is_refused() {
    let fixture = stacks();
    let before = fixture.snapshot();

    assert!(matches!(
        workspace(&fixture).chain(&["b", "a"]),
        Err(Error::Cycle { .. })
    ));
    fixture.assert_unchanged(&before);
}
