//! Resolving restack conflicts with git's rebase, then carrying on.

mod common;

use common::*;
use stack_core::{Conflict, Error, ResolveOutcome};
use stack_testkit::Fixture;

/// `develop` ← `a` (adds a.txt) ← `b`, then `develop` adds a clashing a.txt. HEAD on `develop`.
fn clashing() -> Fixture {
    let fixture = repo();
    grow(&fixture, "a", "develop");
    grow(&fixture, "b", "a");
    fixture.git(&["switch", "--quiet", "develop"]);
    fixture.commit("a.txt", "clash", "feat: clash");
    fixture
}

fn first_conflict(fixture: &Fixture) -> Conflict {
    let restacked = workspace(fixture).restack("develop").unwrap();
    restacked.conflicts.into_iter().next().expect("a conflict")
}

fn rebasing(fixture: &Fixture) -> bool {
    fixture.path().join(".git/rebase-merge").exists()
}

#[test]
fn resolve_stops_for_the_user_then_continue_finishes_the_restack() {
    let fixture = clashing();
    let conflict = first_conflict(&fixture);

    let outcome = workspace(&fixture)
        .resolve_conflict("develop", &conflict)
        .unwrap();
    assert_eq!(
        outcome,
        ResolveOutcome::Stopped {
            branch: "a".into(),
            paths: vec!["a.txt".into()]
        }
    );
    assert!(rebasing(&fixture));

    assert!(matches!(
        workspace(&fixture).continue_restack(),
        Err(Error::UnresolvedConflicts { .. })
    ));

    fixture.write("a.txt", "resolved");
    fixture.git(&["add", "a.txt"]);
    let ResolveOutcome::Finished(restacked) = workspace(&fixture).continue_restack().unwrap()
    else {
        panic!("expected the restack to finish");
    };
    assert_eq!(
        restacked
            .moved
            .iter()
            .map(|moved| moved.name.as_str())
            .collect::<Vec<_>>(),
        ["b"]
    );
    assert!(!rebasing(&fixture));
    assert_eq!(workspace(&fixture).resolving().unwrap(), None);
    assert_eq!(
        fixture.git(&["log", "--format=%s", "develop..b"]),
        "feat: b\nfeat: a"
    );
    assert_eq!(fixture.git(&["show", "b:a.txt"]), "resolved");
}

#[test]
fn abort_gives_up_and_leaves_the_branch_as_it_was() {
    let fixture = clashing();
    let a = tip(&fixture, "a");
    let conflict = first_conflict(&fixture);
    workspace(&fixture)
        .resolve_conflict("develop", &conflict)
        .unwrap();

    let aborted = workspace(&fixture).abort_restack().unwrap();

    assert_eq!(aborted.branch, "a");
    assert!(!rebasing(&fixture));
    assert_eq!(tip(&fixture, "a"), a);
    assert!(matches!(
        workspace(&fixture).continue_restack(),
        Err(Error::NothingToContinue)
    ));
}

#[test]
fn several_conflicting_commits_stop_one_at_a_time() {
    let fixture = repo();
    grow(&fixture, "a", "develop");
    fixture.commit("a.txt", "a, again", "feat: a again");
    fixture.git(&["switch", "--quiet", "develop"]);
    fixture.commit("a.txt", "clash", "feat: clash");
    let conflict = first_conflict(&fixture);
    workspace(&fixture)
        .resolve_conflict("develop", &conflict)
        .unwrap();

    fixture.write("a.txt", "first");
    fixture.git(&["add", "a.txt"]);
    assert!(matches!(
        workspace(&fixture).continue_restack().unwrap(),
        ResolveOutcome::Stopped { .. }
    ));

    fixture.write("a.txt", "second");
    fixture.git(&["add", "a.txt"]);
    assert!(matches!(
        workspace(&fixture).continue_restack().unwrap(),
        ResolveOutcome::Finished(_)
    ));
    assert_eq!(fixture.git(&["show", "a:a.txt"]), "second");
}

#[test]
fn refuses_to_start_on_a_dirty_worktree_or_twice() {
    let fixture = clashing();
    let conflict = first_conflict(&fixture);
    fixture.write("base.txt", "wip");
    assert!(matches!(
        workspace(&fixture).resolve_conflict("develop", &conflict),
        Err(Error::DirtyWorktree { .. })
    ));
    assert!(!rebasing(&fixture));

    fixture.git(&["checkout", "--quiet", "--", "base.txt"]);
    workspace(&fixture)
        .resolve_conflict("develop", &conflict)
        .unwrap();
    assert!(matches!(
        workspace(&fixture).resolve_conflict("develop", &conflict),
        Err(Error::AlreadyResolving { .. })
    ));
}

#[test]
fn a_pinned_branch_moved_through_a_conflict_keeps_its_pin_on_the_new_parent() {
    let fixture = repo();
    grow(&fixture, "a", "develop");
    grow(&fixture, "b", "a");
    workspace(&fixture).pin("b", None).unwrap();
    fixture.git(&["switch", "--quiet", "--create", "x", "develop"]);
    fixture.commit("b.txt", "clash", "feat: x");
    let restacked = workspace(&fixture).move_branch("b", "x").unwrap();
    let conflict = restacked.conflicts[0].clone();
    workspace(&fixture)
        .resolve_conflict("b", &conflict)
        .unwrap();

    fixture.write("b.txt", "resolved");
    fixture.git(&["add", "b.txt"]);
    workspace(&fixture).continue_restack().unwrap();

    let b = parent(&tree(&fixture), "b").unwrap();
    assert_eq!(
        (b.name.as_str(), b.source, b.contradicted),
        ("x", stack_core::Source::Pinned, false)
    );
}
