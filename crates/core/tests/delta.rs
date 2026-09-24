//! Comparing two branches' own changes.

mod common;

use common::*;
use stack_testkit::Fixture;

/// `develop` ← `a` (feature.txt); `develop` ← `x` (lots of unrelated noise) ← `b` (the same feature.txt change).
fn twins() -> Fixture {
    let fixture = repo();
    grow(&fixture, "a", "develop");
    fixture.commit("feature.txt", "the feature", "feat: feature");
    fixture.git(&["switch", "--quiet", "--create", "x", "develop"]);
    for index in 0..5 {
        fixture.commit(
            &format!("noise{index}.txt"),
            "noise",
            &format!("chore: noise {index}"),
        );
    }
    fixture.git(&["switch", "--quiet", "--create", "b", "x"]);
    fixture.git(&["cherry-pick", "a~1", "a"]);
    fixture
}

#[test]
fn identical_changes_on_different_bases_have_no_net_difference() {
    let fixture = twins();

    let delta = workspace(&fixture).delta("a", "b").unwrap();

    assert!(
        !fixture.git(&["diff", "--stat", "a", "b"]).is_empty(),
        "a plain diff is all noise"
    );
    let tree = delta.b_on_a_base.unwrap();
    assert_eq!(fixture.git(&["diff", "--stat", &delta.a.tip, &tree]), "");
    assert_eq!(delta.a.base, fixture.git(&["merge-base", "a", "develop"]));
    assert_eq!(delta.b.base, tip(&fixture, "x"));
}

#[test]
fn only_the_real_difference_shows() {
    let fixture = twins();
    fixture.commit("extra.txt", "extra", "feat: extra");

    let delta = workspace(&fixture).delta("a", "b").unwrap();

    let names = fixture.git(&[
        "diff",
        "--name-only",
        &delta.a.tip,
        &delta.b_on_a_base.unwrap(),
    ]);
    assert_eq!(names, "extra.txt");
}

#[test]
fn a_replay_that_conflicts_is_reported() {
    let fixture = repo();
    grow(&fixture, "a", "develop");
    fixture.git(&["switch", "--quiet", "--create", "x", "develop"]);
    fixture.commit("base.txt", "changed on x", "chore: change base");
    fixture.git(&["switch", "--quiet", "--create", "b", "x"]);
    fixture.commit("base.txt", "changed again on b", "feat: b");

    let delta = workspace(&fixture).delta("a", "b").unwrap();

    assert_eq!(
        (delta.b_on_a_base, delta.conflicts),
        (None, vec!["base.txt".to_owned()])
    );
}
