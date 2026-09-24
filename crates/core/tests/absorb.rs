//! Absorbing staged hunks into the commits that last touched their lines.

mod common;

use common::*;
use stack_core::Error;
use stack_testkit::Fixture;

const LINES: &str = "one\ntwo\nthree\nfour\nfive\n";

/// `develop` (base.txt) ← `a` (a.txt) ← `b`: commit b1 adds f1.txt, commit b2 adds f2.txt. HEAD on `b`.
fn stack() -> Fixture {
    let fixture = repo();
    fixture.git(&["switch", "--quiet", "--create", "a", "develop"]);
    fixture.commit("a.txt", LINES, "feat: a");
    fixture.git(&["switch", "--quiet", "--create", "b"]);
    fixture.commit("f1.txt", LINES, "feat: b1");
    fixture.commit("f2.txt", LINES, "feat: b2");
    fixture
}

fn stage(fixture: &Fixture, path: &str, contents: &str) {
    fixture.write(path, contents);
    fixture.git(&["add", path]);
}

#[test]
fn each_hunk_goes_into_the_commit_that_last_changed_its_lines() {
    let fixture = stack();
    stage(&fixture, "f1.txt", "one\nTWO\nthree\nfour\nfive\n");
    stage(&fixture, "f2.txt", "one\ntwo\nthree\nFOUR\nfive\n");
    stage(&fixture, "a.txt", "ONE\ntwo\nthree\nfour\nfive\n");

    let absorbed = workspace(&fixture).absorb().unwrap();

    assert_eq!(absorbed.absorbed.len(), 3);
    assert!(absorbed.left.is_empty(), "{:?}", absorbed.left);
    assert_eq!(
        fixture.git(&["show", "b~1:f1.txt"]).lines().nth(1),
        Some("TWO")
    );
    assert_eq!(
        fixture.git(&["show", "b:f2.txt"]).lines().nth(3),
        Some("FOUR")
    );
    assert_eq!(
        fixture.git(&["show", "a:a.txt"]).lines().next(),
        Some("ONE")
    );
    assert_eq!(
        fixture.git(&["log", "--format=%s", "develop..b"]),
        "feat: b2\nfeat: b1\nfeat: a"
    );
    assert_eq!(fixture.git(&["status", "--porcelain"]), "");
}

#[test]
fn what_cannot_be_placed_stays_staged() {
    let fixture = stack();
    stage(&fixture, "f1.txt", "one\nTWO\nthree\nfour\nfive\nsix\n");
    stage(&fixture, "base.txt", "changed on develop's line");
    stage(&fixture, "new.txt", "brand new");

    let absorbed = workspace(&fixture).absorb().unwrap();

    assert_eq!(absorbed.absorbed.len(), 1);
    let mut reasons: Vec<(String, String)> = absorbed
        .left
        .iter()
        .map(|hunk| (hunk.path.clone(), hunk.reason.clone().unwrap()))
        .collect();
    reasons.sort();
    assert_eq!(
        reasons,
        [
            (
                "base.txt".to_owned(),
                "changes lines from outside this stack".to_owned()
            ),
            ("f1.txt".to_owned(), "only adds lines".to_owned()),
            ("new.txt".to_owned(), "only adds lines".to_owned()),
        ]
    );
    let staged = fixture.git(&["diff", "--cached", "--name-only"]);
    assert_eq!(staged, "base.txt\nf1.txt\nnew.txt");
    assert_eq!(
        fixture
            .git(&["diff", "--cached", "f1.txt"])
            .lines()
            .filter(|line| line.starts_with('+') && !line.starts_with("+++"))
            .collect::<Vec<_>>(),
        ["+six"]
    );
}

#[test]
fn undo_back_to_the_first_absorb_restores_the_staged_changes() {
    let fixture = stack();
    stage(&fixture, "f1.txt", "one\nTWO\nthree\nfour\nfive\n");
    stage(&fixture, "a.txt", "ONE\ntwo\nthree\nfour\nfive\nextra\n");
    let before = fixture.snapshot();
    let absorbed = workspace(&fixture).absorb().unwrap();
    let first = workspace(&fixture)
        .oplog(10)
        .unwrap()
        .into_iter()
        .rev()
        .find(|operation| operation.description.starts_with("amend"))
        .unwrap();
    assert_eq!(absorbed.amended.len(), 2);

    workspace(&fixture).undo_to(first.id).unwrap();

    let diff = before.diff(&fixture.snapshot());
    assert!(
        diff.refs.keys().all(|name| name == "refs/stack/keep")
            && diff.index.is_empty()
            && diff.worktree.is_empty(),
        "{diff}"
    );
}

#[test]
fn needs_staged_changes() {
    let fixture = stack();
    assert!(matches!(
        workspace(&fixture).absorb(),
        Err(Error::NothingStaged)
    ));
}

#[test]
fn hunks_in_one_file_can_go_to_different_commits() {
    let fixture = repo();
    fixture.git(&["switch", "--quiet", "--create", "b", "develop"]);
    fixture.commit("f.txt", LINES, "feat: add f");
    fixture.commit("f.txt", "one\ntwo\nthree\nfour\nFIVE\n", "feat: shout five");
    stage(&fixture, "f.txt", "one\nTwo\nthree\nfour\nFive!\n");

    let absorbed = workspace(&fixture).absorb().unwrap();

    assert_eq!(absorbed.amended.len(), 2, "{:?}", absorbed.left);
    assert_eq!(
        fixture.git(&["show", "b~1:f.txt"]),
        "one\nTwo\nthree\nfour\nfive"
    );
    assert_eq!(
        fixture.git(&["show", "b:f.txt"]),
        "one\nTwo\nthree\nfour\nFive!"
    );
    assert_eq!(fixture.git(&["status", "--porcelain"]), "");
}
