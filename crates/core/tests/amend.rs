//! Amending staged changes into commits other than HEAD.

mod common;

use common::*;
use stack_core::Error;
use stack_testkit::Fixture;

/// `develop` ← `a` (a.txt) ← `b` (b1.txt, then b2.txt). HEAD on `b`.
fn stack() -> Fixture {
    let fixture = repo();
    grow(&fixture, "a", "develop");
    fixture.git(&["switch", "--quiet", "--create", "b"]);
    fixture.commit("b1.txt", "b1", "feat: b1");
    fixture.commit("b2.txt", "b2", "feat: b2");
    fixture
}

fn stage(fixture: &Fixture, path: &str, contents: &str) {
    fixture.write(path, contents);
    fixture.git(&["add", path]);
}

#[test]
fn amends_an_earlier_commit_of_this_branch() {
    let fixture = stack();
    stage(&fixture, "b1.txt", "b1, fixed");
    fixture.write("scratch.txt", "untracked work");
    fixture.write("b2.txt", "unstaged edit");

    let amended = workspace(&fixture).amend_into("HEAD~1").unwrap();

    assert_eq!(amended.branch, "b");
    assert_eq!(fixture.git(&["show", "b~1:b1.txt"]), "b1, fixed");
    assert_eq!(
        fixture.git(&["log", "--format=%s", "a..b"]),
        "feat: b2\nfeat: b1"
    );
    // Staged work is now committed; unstaged and untracked work is untouched.
    assert_eq!(
        fixture.git(&["status", "--porcelain"]),
        "M b2.txt\n?? scratch.txt"
    );
    assert_eq!(
        std::fs::read_to_string(fixture.path().join("b2.txt")).unwrap(),
        "unstaged edit"
    );
}

#[test]
fn amends_a_commit_of_a_parent_branch_and_restacks_this_one() {
    let fixture = stack();
    stage(&fixture, "a.txt", "a, fixed");

    let amended = workspace(&fixture).amend_into("a").unwrap();

    assert_eq!(amended.branch, "a");
    assert_eq!(fixture.git(&["show", "a:a.txt"]), "a, fixed");
    assert_eq!(fixture.git(&["merge-base", "a", "b"]), tip(&fixture, "a"));
    assert_eq!(fixture.git(&["show", "b:a.txt"]), "a, fixed");
    assert_eq!(fixture.git(&["status", "--porcelain"]), "");
    assert_eq!(shape(&tree(&fixture)), "develop(a(b))");
}

#[test]
fn undo_puts_the_change_back_to_staged() {
    let fixture = stack();
    stage(&fixture, "a.txt", "a, fixed");
    let before = fixture.snapshot();
    workspace(&fixture).amend_into("a").unwrap();

    workspace(&fixture).undo().unwrap();

    let diff = before.diff(&fixture.snapshot());
    assert!(
        diff.refs.keys().all(|name| name == "refs/stack/keep")
            && diff.index.is_empty()
            && diff.worktree.is_empty(),
        "{diff}"
    );
    assert_eq!(fixture.git(&["status", "--porcelain"]), "M  a.txt");
}

#[test]
fn conflicts_and_bad_targets_change_nothing() {
    let fixture = stack();
    // b2 is replayed after b1; changing b2.txt inside b1 clashes with b2 adding it.
    stage(&fixture, "b2.txt", "early");
    let before = fixture.snapshot();
    let workspace = workspace(&fixture);

    assert!(matches!(
        workspace.amend_into("HEAD~1"),
        Err(Error::AmendConflict { .. })
    ));
    assert!(matches!(
        workspace.amend_into("develop"),
        Err(Error::NotInStack { .. })
    ));
    assert!(matches!(
        workspace.amend_into("nope"),
        Err(Error::UnknownRevision { .. })
    ));
    let diff = before.diff(&fixture.snapshot());
    assert!(
        diff.refs.is_empty() && diff.index.is_empty() && diff.worktree.is_empty(),
        "{diff}"
    );
}

#[test]
fn needs_something_staged_and_a_branch() {
    let fixture = stack();
    assert!(matches!(
        workspace(&fixture).amend_into("HEAD~1"),
        Err(Error::NothingStaged)
    ));
    fixture.git(&["switch", "--quiet", "--detach"]);
    stage(&fixture, "a.txt", "x");
    assert!(matches!(
        workspace(&fixture).amend_into("HEAD~1"),
        Err(Error::DetachedHead)
    ));
}
