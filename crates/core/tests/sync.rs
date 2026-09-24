//! Syncing after work lands on the remote.

mod common;

use common::*;
use stack_core::TrunkUpdate;
use stack_testkit::Fixture;

/// `develop` (trunk, on `origin`) ← `a` (two commits) ← `b`. HEAD on `develop`.
fn stacked() -> Fixture {
    let fixture = repo();
    fixture.add_bare_remote("origin");
    fixture.git(&["push", "--quiet", "origin", "develop"]);
    grow(&fixture, "a", "develop");
    fixture.commit("a2.txt", "a2", "feat: a2");
    grow(&fixture, "b", "a");
    fixture.git(&["switch", "--quiet", "develop"]);
    fixture
}

/// Lands `a` on origin's develop the way `how` merges it, leaving local develop behind.
fn land_on_origin(fixture: &Fixture, how: &str) {
    fixture.git(&["switch", "--quiet", "--detach", "develop"]);
    fixture.commit("other.txt", "someone else's work", "feat: other");
    match how {
        "squash" => {
            fixture.git(&["merge", "--quiet", "--squash", "a"]);
            fixture.git(&["commit", "--quiet", "--message", "feat: a (#1)"]);
        }
        "rebase" => {
            fixture.git(&["cherry-pick", "develop..a"]);
        }
        "merge" => {
            fixture.git(&["merge", "--quiet", "--no-ff", "--no-edit", "a"]);
        }
        _ => unreachable!(),
    }
    fixture.git(&["push", "--quiet", "origin", "HEAD:develop"]);
    fixture.git(&["switch", "--quiet", "develop"]);
}

fn branches(fixture: &Fixture) -> String {
    fixture.git(&["for-each-ref", "--format=%(refname:short)", "refs/heads/"])
}

#[test]
fn a_squash_merged_branch_is_archived_and_its_children_rehomed() {
    let fixture = stacked();
    land_on_origin(&fixture, "squash");

    let synced = workspace(&fixture).sync(None, true).unwrap();

    assert!(
        matches!(synced.trunks[0].outcome, TrunkUpdate::FastForwarded { .. }),
        "{synced:?}"
    );
    assert_eq!(
        (synced.merged.as_slice(), synced.archived.as_slice()),
        (["a".to_owned()].as_slice(), ["a".to_owned()].as_slice())
    );
    assert_eq!(branches(&fixture), "b\ndevelop");
    assert_eq!(
        fixture.git(&["log", "--format=%s", "develop..b"]),
        "feat: b"
    );
    assert_eq!(shape(&tree(&fixture)), "develop(b)");
    assert!(synced.conflicts.is_empty());
}

#[test]
fn rebase_merged_and_merge_committed_branches_are_found_too() {
    for how in ["rebase", "merge"] {
        let fixture = stacked();
        land_on_origin(&fixture, how);

        let synced = workspace(&fixture).sync(None, true).unwrap();

        assert_eq!(synced.merged, ["a"], "{how}");
        assert_eq!(
            fixture.git(&["log", "--format=%s", "develop..b"]),
            "feat: b",
            "{how}"
        );
    }
}

#[test]
fn a_brand_new_branch_is_not_mistaken_for_merged() {
    let fixture = stacked();
    fixture.git(&["branch", "fresh", "develop"]);
    land_on_origin(&fixture, "squash");

    let synced = workspace(&fixture).sync(None, true).unwrap();

    assert!(!synced.merged.contains(&"fresh".to_owned()));
    assert!(branches(&fixture).contains("fresh"));
}

#[test]
fn a_merged_branch_you_are_on_is_left_for_its_parent_then_archived() {
    let fixture = stacked();
    land_on_origin(&fixture, "squash");
    fixture.git(&["switch", "--quiet", "a"]);

    let synced = workspace(&fixture).sync(None, true).unwrap();

    assert_eq!(synced.archived, ["a"]);
    assert_eq!(fixture.git(&["symbolic-ref", "--short", "HEAD"]), "develop");
}

#[test]
fn a_diverged_trunk_is_left_alone() {
    let fixture = stacked();
    land_on_origin(&fixture, "squash");
    fixture.commit("local.txt", "local", "feat: local only");

    let synced = workspace(&fixture).sync(None, true).unwrap();

    assert_eq!(synced.trunks[0].outcome, TrunkUpdate::Diverged);
    assert!(synced.merged.is_empty());
}

#[test]
fn nothing_to_do_changes_nothing_but_the_fetch() {
    let fixture = stacked();
    workspace(&fixture).restack("develop").unwrap();
    let before = fixture.snapshot();

    let synced = workspace(&fixture).sync(None, false).unwrap();

    assert!(synced.merged.is_empty() && synced.moved.is_empty() && synced.archived.is_empty());
    assert_eq!(synced.trunks[0].outcome, TrunkUpdate::UpToDate);
    fixture.assert_unchanged(&before);
}
