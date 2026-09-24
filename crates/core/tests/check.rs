//! Previewing a restack.

mod common;

use common::*;
use stack_core::{Error, PreviewedMove};
use stack_testkit::Fixture;

/// `develop` ← `a` ← `b`, `develop` ← `c`; `develop` then changes `a`'s file. HEAD on `develop`.
fn stale() -> Fixture {
    let fixture = repo();
    grow(&fixture, "a", "develop");
    grow(&fixture, "b", "a");
    grow(&fixture, "c", "develop");
    fixture.git(&["switch", "--quiet", "develop"]);
    fixture.commit("a.txt", "clash", "feat: clash");
    fixture
}

fn moved(name: &str, onto: &str) -> PreviewedMove {
    PreviewedMove {
        name: name.into(),
        onto: onto.into(),
        replayed: 1,
        dropped: 0,
    }
}

#[test]
fn reports_clean_moves_conflicts_and_blocked_branches_without_changes() {
    let fixture = stale();
    let before = fixture.snapshot();

    let preview = workspace(&fixture).check(None).unwrap();

    assert_eq!(preview.clean, [moved("c", "develop")]);
    let [conflict] = preview.conflicts.as_slice() else {
        panic!("{preview:?}")
    };
    assert_eq!(
        (conflict.branch.as_str(), conflict.paths.as_slice()),
        ("a", ["a.txt".to_owned()].as_slice())
    );
    assert_eq!(preview.blocked, ["b"]);

    fixture.assert_unchanged(&before);
}

#[test]
fn a_clean_stack_previews_every_branch_parents_first() {
    let fixture = repo();
    grow(&fixture, "a", "develop");
    grow(&fixture, "b", "a");
    fixture.git(&["switch", "--quiet", "develop"]);
    fixture.commit("later.txt", "later", "feat: later");

    let preview = workspace(&fixture).check(Some("a")).unwrap();

    assert_eq!(preview.clean, [moved("a", "develop"), moved("b", "a")]);
    assert!(preview.conflicts.is_empty() && preview.blocked.is_empty());
}

#[test]
fn covers_every_trunk_by_default() {
    let fixture = repo();
    grow(&fixture, "release", "develop");
    workspace(&fixture).add_trunk("release").unwrap();
    grow(&fixture, "fix", "release");
    grow(&fixture, "feature", "develop");
    for trunk in ["develop", "release"] {
        fixture.git(&["switch", "--quiet", trunk]);
        fixture.commit(&format!("{trunk}-later.txt"), "later", "feat: later");
    }

    let preview = workspace(&fixture).check(None).unwrap();

    assert_eq!(
        preview.clean,
        [moved("feature", "develop"), moved("fix", "release")]
    );
}

#[test]
fn never_signs_even_when_signing_would_fail() {
    let fixture = stale();
    fixture.git(&["config", "gpg.format", "ssh"]);
    fixture.git(&["config", "user.signingkey", "/nonexistent/key"]);
    fixture.git(&["config", "commit.gpgsign", "true"]);

    assert_eq!(
        workspace(&fixture).check(Some("c")).unwrap().clean,
        [moved("c", "develop")]
    );
    assert!(
        workspace(&fixture).restack("c").is_err(),
        "restack does sign, and the key doesn't exist"
    );
}

#[test]
fn up_to_date_and_unknown() {
    let fixture = repo();
    grow(&fixture, "a", "develop");

    let preview = workspace(&fixture).check(None).unwrap();
    assert!(preview.clean.is_empty() && preview.conflicts.is_empty() && preview.blocked.is_empty());
    assert!(matches!(
        workspace(&fixture).check(Some("nope")),
        Err(Error::UnknownBranch { .. })
    ));
}
