//! Finding commits no ref reaches any more.

mod common;

use common::*;
use stack_testkit::Fixture;

fn lost(fixture: &Fixture) -> Vec<(String, String)> {
    let found = workspace(fixture).lost(20).unwrap();
    found
        .into_iter()
        .map(|lost| (lost.summary, lost.how.split(':').next().unwrap().to_owned()))
        .collect()
}

#[test]
fn the_version_before_an_amend_is_lost() {
    let fixture = repo();
    grow(&fixture, "a", "develop");
    fixture.git(&[
        "commit",
        "--quiet",
        "--amend",
        "--message",
        "feat: a, reworded",
    ]);

    assert_eq!(
        lost(&fixture),
        [("feat: a".to_owned(), "commit (amend)".to_owned())]
    );
}

#[test]
fn work_left_by_a_hard_reset_is_lost_and_only_the_newest_of_a_chain_is_listed() {
    let fixture = repo();
    grow(&fixture, "a", "develop");
    fixture.commit("a2.txt", "a2", "feat: a2");
    fixture.git(&["reset", "--quiet", "--hard", "develop"]);

    assert_eq!(
        lost(&fixture),
        [("feat: a2".to_owned(), "reset".to_owned())]
    );
}

#[test]
fn a_deleted_branch_you_had_checked_out_is_lost() {
    let fixture = repo();
    grow(&fixture, "gone", "develop");
    fixture.git(&["switch", "--quiet", "develop"]);
    fixture.git(&["branch", "--quiet", "-D", "gone"]);

    let found = workspace(&fixture).lost(20).unwrap();

    assert_eq!(found.len(), 1);
    assert_eq!(
        (found[0].summary.as_str(), found[0].seen_on.as_str()),
        ("feat: gone", "HEAD")
    );
}

#[test]
fn commits_any_ref_still_reaches_are_not_lost() {
    let fixture = repo();
    grow(&fixture, "a", "develop");
    grow(&fixture, "b", "develop");
    fixture.git(&["tag", "kept", "a"]);
    fixture.git(&["switch", "--quiet", "develop"]);
    fixture.git(&["branch", "--quiet", "-D", "a"]);
    workspace(&fixture).archive("b").unwrap();
    let before = fixture.snapshot();

    assert!(lost(&fixture).is_empty());
    fixture.assert_unchanged(&before);
}

#[test]
fn restoring_a_lost_commit_removes_it_from_the_list() {
    let fixture = repo();
    grow(&fixture, "a", "develop");
    fixture.git(&[
        "commit",
        "--quiet",
        "--amend",
        "--message",
        "feat: a, reworded",
    ]);
    let old = workspace(&fixture).lost(20).unwrap()[0].commit.clone();

    fixture.git(&["branch", "rescued", &old]);

    assert!(lost(&fixture).is_empty());
}
