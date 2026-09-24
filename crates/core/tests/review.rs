//! Review marks: personal notes on commits that follow the change, not the commit id.

mod common;

use common::*;
use stack_core::{Error, MarkKind};
use stack_testkit::Fixture;

/// `develop` ← `a` (one commit), then `develop` advances. HEAD on `a`.
fn branch() -> Fixture {
    let fixture = repo();
    grow(&fixture, "a", "develop");
    fixture.git(&["switch", "--quiet", "develop"]);
    fixture.commit("later.txt", "later", "feat: later");
    fixture.git(&["switch", "--quiet", "a"]);
    fixture
}

fn kinds(fixture: &Fixture, branch: &str) -> Vec<Vec<MarkKind>> {
    let review = workspace(fixture).review(branch).unwrap();
    review
        .iter()
        .map(|commit| commit.marks.iter().map(|mark| mark.kind).collect())
        .collect()
}

#[test]
fn marks_follow_the_change_through_a_restack() {
    let fixture = branch();
    workspace(&fixture)
        .mark("a", MarkKind::Reviewed, None)
        .unwrap();
    let before = tip(&fixture, "a");

    workspace(&fixture).restack("develop").unwrap();

    assert_ne!(tip(&fixture, "a"), before);
    assert_eq!(kinds(&fixture, "a"), [[MarkKind::Reviewed]]);
}

#[test]
fn marks_follow_a_message_only_amend_but_lapse_when_the_change_changes() {
    let fixture = branch();
    workspace(&fixture)
        .mark("HEAD", MarkKind::Tested, None)
        .unwrap();

    fixture.git(&[
        "commit",
        "--quiet",
        "--amend",
        "--message",
        "feat: a, reworded",
    ]);
    assert_eq!(kinds(&fixture, "a"), [[MarkKind::Tested]]);

    fixture.write("a.txt", "different");
    fixture.git(&["commit", "--quiet", "--all", "--amend", "--no-edit"]);
    assert_eq!(kinds(&fixture, "a"), [Vec::<MarkKind>::new()]);
}

#[test]
fn flags_carry_notes_and_unmark_removes_by_kind_or_all() {
    let fixture = branch();
    let workspace = workspace(&fixture);
    workspace.mark("a", MarkKind::Reviewed, None).unwrap();
    workspace
        .mark("a", MarkKind::Flagged, Some("check the empty case"))
        .unwrap();

    let review = workspace.review("a").unwrap();
    assert_eq!(review[0].summary, "feat: a");
    let flag = review[0]
        .marks
        .iter()
        .find(|mark| mark.kind == MarkKind::Flagged)
        .unwrap();
    assert_eq!(flag.note.as_deref(), Some("check the empty case"));

    assert_eq!(workspace.unmark("a", Some(MarkKind::Flagged)).unwrap(), 1);
    assert_eq!(kinds(&fixture, "a"), [[MarkKind::Reviewed]]);
    assert_eq!(workspace.unmark("a", None).unwrap(), 1);
    assert_eq!(workspace.unmark("a", None).unwrap(), 0);
}

#[test]
fn review_lists_only_the_branch_s_own_commits_newest_first() {
    let fixture = branch();
    fixture.commit("a2.txt", "a2", "feat: a2");

    let review = workspace(&fixture).review("a").unwrap();

    let summaries: Vec<&str> = review
        .iter()
        .map(|commit| commit.summary.as_str())
        .collect();
    assert_eq!(summaries, ["feat: a2", "feat: a"]);
}

#[test]
fn empty_commits_keep_their_marks_by_id() {
    let fixture = branch();
    fixture.git(&[
        "commit",
        "--quiet",
        "--allow-empty",
        "--message",
        "chore: empty",
    ]);

    workspace(&fixture)
        .mark("HEAD", MarkKind::Reviewed, None)
        .unwrap();

    assert_eq!(kinds(&fixture, "a")[0], [MarkKind::Reviewed]);
}

#[test]
fn marking_touches_only_the_local_store() {
    let fixture = branch();
    let before = fixture.snapshot();

    assert!(workspace(&fixture).review("a").unwrap()[0].marks.is_empty());
    fixture.assert_unchanged(&before);

    workspace(&fixture)
        .mark("a", MarkKind::Reviewed, None)
        .unwrap();
    let diff = before.diff(&fixture.snapshot());
    assert!(
        diff.refs.is_empty() && diff.objects_added.is_empty(),
        "{diff}"
    );
    assert_eq!(diff.stack.iter().collect::<Vec<_>>(), ["stack.db"]);
}

#[test]
fn unknown_revisions_and_branches() {
    let fixture = branch();
    assert!(matches!(
        workspace(&fixture).mark("nope", MarkKind::Reviewed, None),
        Err(Error::UnknownRevision { .. })
    ));
    assert!(matches!(
        workspace(&fixture).review("nope"),
        Err(Error::UnknownBranch { .. })
    ));
}
