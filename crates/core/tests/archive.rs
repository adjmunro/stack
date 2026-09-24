//! Archiving and restoring branches.

mod common;

use common::*;
use stack_core::{Archived, Error};
use stack_testkit::Fixture;

/// `develop` ← `a` ← `b`, `develop` ← `old`. HEAD on `develop`.
fn stacks() -> Fixture {
    let fixture = repo();
    grow(&fixture, "a", "develop");
    grow(&fixture, "b", "a");
    grow(&fixture, "old", "develop");
    fixture.git(&["switch", "--quiet", "develop"]);
    fixture
}

#[test]
fn archive_moves_the_branch_out_of_sight_but_keeps_its_commits() {
    let fixture = stacks();
    let old = tip(&fixture, "old");
    let before = fixture.snapshot();

    workspace(&fixture).archive("old").unwrap();

    assert_eq!(fixture.git(&["branch", "--list", "old"]), "");
    assert_eq!(
        workspace(&fixture).archived().unwrap(),
        [Archived {
            name: "old".into(),
            commit: old.clone()
        }]
    );
    assert_eq!(shape(&tree(&fixture)), "develop(a(b))");
    let diff = before.diff(&fixture.snapshot());
    let refs: Vec<&str> = diff.refs.keys().map(String::as_str).collect();
    assert_eq!(refs, ["refs/heads/old", "refs/stack/archive/old"]);
    assert!(diff.objects_added.is_empty(), "{diff}");

    fixture.git(&["gc", "--quiet", "--prune=now"]);
    assert_eq!(fixture.git(&["cat-file", "-t", &old]), "commit");
}

#[test]
fn unarchive_and_undo_bring_it_back() {
    let fixture = stacks();
    let before = fixture.snapshot();
    workspace(&fixture).archive("old").unwrap();

    workspace(&fixture).unarchive("old").unwrap();
    assert_eq!(shape(&tree(&fixture)), "develop(a(b) old)");

    workspace(&fixture).undo().unwrap();
    workspace(&fixture).undo().unwrap();
    let diff = before.diff(&fixture.snapshot());
    assert!(diff.refs.is_empty(), "{diff}");
    fixture.git(&["fsck", "--no-progress"]);
}

#[test]
fn restack_ignores_archived_branches_and_keeps_their_records() {
    let fixture = stacks();
    workspace(&fixture).pin("old", None).unwrap();
    workspace(&fixture).archive("old").unwrap();
    fixture.commit("later.txt", "later", "feat: later");

    let restacked = workspace(&fixture).restack("develop").unwrap();

    assert!(restacked.moved.iter().all(|moved| moved.name != "old"));
    assert_ne!(
        fixture.git(&["for-each-ref", "refs/stack/branches/old"]),
        ""
    );
}

#[test]
fn refusals_change_nothing() {
    let fixture = stacks();
    workspace(&fixture).archive("old").unwrap();
    fixture.git(&["branch", "old", "develop"]);
    fixture.git(&["switch", "--quiet", "b"]);
    let before = fixture.snapshot();
    let workspace = workspace(&fixture);

    assert!(matches!(
        workspace.archive("a"),
        Err(Error::HasChildren { .. })
    ));
    assert!(matches!(
        workspace.archive("develop"),
        Err(Error::IsTrunk { .. })
    ));
    assert!(matches!(
        workspace.archive("b"),
        Err(Error::DeletesCheckedOutBranch { .. })
    ));
    assert!(matches!(
        workspace.archive("old"),
        Err(Error::AlreadyArchived { .. })
    ));
    assert!(matches!(
        workspace.archive("nope"),
        Err(Error::UnknownBranch { .. })
    ));
    assert!(matches!(
        workspace.unarchive("old"),
        Err(Error::BranchExists { .. })
    ));
    assert!(matches!(
        workspace.unarchive("nope"),
        Err(Error::NotArchived { .. })
    ));
    let diff = before.diff(&fixture.snapshot());
    assert!(diff.refs.is_empty() && diff.worktree.is_empty(), "{diff}");
}
