mod common;

use common::{stack, stderr, stdout};
use stack_testkit::Fixture;

/// `develop` (a trunk) ← `feat/a` ← `feat/b`, then `develop` advances. HEAD on `feat/b`.
fn advanced() -> Fixture {
    let fixture = Fixture::new();
    fixture.commit("base.txt", "base", "feat: base");
    stack(&fixture, &["trunk", "add", "develop"]);
    fixture.git(&["switch", "--quiet", "--create", "feat/a"]);
    fixture.commit("a.txt", "a", "feat: a");
    fixture.git(&["switch", "--quiet", "--create", "feat/b"]);
    fixture.commit("b.txt", "b", "feat: b");
    fixture.git(&["switch", "--quiet", "develop"]);
    fixture.commit("later.txt", "later", "feat: later");
    fixture.git(&["switch", "--quiet", "feat/b"]);
    fixture
}

#[test]
fn restacks_from_the_trunk_and_reports_each_branch() {
    let fixture = advanced();

    assert_eq!(
        stdout(&stack(&fixture, &["restack", "develop"])),
        "Restacked feat/a onto develop (1 commit)\nRestacked feat/b onto feat/a (1 commit)\n"
    );
    assert_eq!(
        stdout(&stack(&fixture, &["restack", "develop"])),
        "Everything is up to date.\n"
    );
    assert_eq!(
        stdout(&stack(&fixture, &["undo"])),
        "Undid #2: restack develop\n"
    );
}

#[test]
fn defaults_to_the_current_branch() {
    let fixture = advanced();
    fixture.git(&["switch", "--quiet", "feat/a"]);

    assert_eq!(
        stdout(&stack(&fixture, &["restack"])),
        "Restacked feat/a onto develop (1 commit)\nRestacked feat/b onto feat/a (1 commit)\n"
    );
}

#[test]
fn conflict_explains_how_to_finish_and_fails() {
    let fixture = advanced();
    let offshoot = fixture.git(&["rev-parse", "--short=7", "develop~1"]);
    fixture.git(&["switch", "--quiet", "develop"]);
    fixture.commit("a.txt", "clash", "feat: clash");

    let output = stack(&fixture, &["restack", "develop", "--no-resolve"]);

    let commit = fixture.git(&["rev-parse", "--short=7", "feat/a"]);
    assert_eq!(
        String::from_utf8(output.stdout.clone()).unwrap(),
        format!(
            "Conflict: feat/a \"feat: a\" ({commit}) conflicts with develop in a.txt\n\
             Left feat/a and the branches on it as they were. To finish:\n  \
             git rebase --onto develop {offshoot} feat/a\n  \
             stack restack feat/a\n"
        )
    );
    assert_eq!(stderr(&output), "error: restack stopped at a conflict\n");
}

#[test]
fn following_the_conflict_instructions_finishes_the_restack() {
    let fixture = advanced();
    let offshoot = fixture.git(&["rev-parse", "develop~1"]);
    fixture.git(&["switch", "--quiet", "develop"]);
    fixture.commit("a.txt", "clash", "feat: clash");
    stack(&fixture, &["restack", "develop", "--no-resolve"]);

    let rebase = fixture
        .command("git")
        .args([
            "rebase", "--quiet", "--onto", "develop", &offshoot, "feat/a",
        ])
        .output()
        .unwrap();
    assert!(
        !rebase.status.success(),
        "rebase should stop at the conflict"
    );
    fixture.write("a.txt", "resolved");
    fixture.git(&["add", "a.txt"]);
    fixture
        .command("git")
        .env("GIT_EDITOR", "true")
        .args(["rebase", "--continue"])
        .output()
        .unwrap();

    assert_eq!(
        stdout(&stack(&fixture, &["restack", "feat/a"])),
        "Restacked feat/b onto feat/a (1 commit)\n"
    );
    let log = fixture.git(&["log", "--format=%s", "develop..feat/b"]);
    assert_eq!(log, "feat: b\nfeat: a");
    assert_eq!(fixture.git(&["show", "feat/b:a.txt"]), "resolved");
}

#[test]
fn move_takes_descendants_along() {
    let fixture = advanced();
    fixture.git(&["switch", "--quiet", "--create", "feat/x", "develop"]);
    fixture.commit("x.txt", "x", "feat: x");
    fixture.git(&["switch", "--quiet", "feat/a"]);

    assert_eq!(
        stdout(&stack(&fixture, &["move", "--onto", "feat/x"])),
        "Restacked feat/a onto feat/x (1 commit)\nRestacked feat/b onto feat/a (1 commit)\n"
    );
    assert_eq!(
        stderr(&stack(&fixture, &["move", "feat/x", "--onto", "feat/b"])),
        "error: feat/b is feat/x or stacked on it; that would create a cycle\n"
    );
}

#[test]
fn check_previews_without_changing_anything() {
    let fixture = advanced();
    let before = fixture.snapshot();

    assert_eq!(
        stdout(&stack(&fixture, &["check"])),
        "feat/a restacks cleanly onto develop (1 commit)\nfeat/b restacks cleanly onto feat/a (1 commit)\n"
    );
    assert!(before.diff(&fixture.snapshot()).refs.is_empty());

    fixture.git(&["switch", "--quiet", "develop"]);
    fixture.commit("a.txt", "clash", "feat: clash");
    let output = stack(&fixture, &["check"]);
    let commit = fixture.git(&["rev-parse", "--short=7", "feat/a"]);
    assert_eq!(
        String::from_utf8(output.stdout.clone()).unwrap(),
        format!(
            "Conflict: feat/a \"feat: a\" ({commit}) conflicts with develop in a.txt\n\
             feat/b can't be checked until the conflict below it is resolved\n"
        )
    );
    assert_eq!(stderr(&output), "error: a restack would conflict\n");
}

#[test]
fn tree_check_marks_conflicts_and_blocked_branches() {
    let fixture = advanced();
    fixture.git(&["switch", "--quiet", "develop"]);
    fixture.commit("a.txt", "clash", "feat: clash");

    let tree = stdout(&stack(&fixture, &["tree", "--check"]));

    assert!(
        tree.contains("feat/a") && tree.contains("(needs restack) (restack conflicts in a.txt)"),
        "{tree}"
    );
    assert!(tree.contains("(restack blocked below)"), "{tree}");
}

#[test]
fn a_conflict_starts_git_s_rebase_and_continue_finishes() {
    let fixture = advanced();
    fixture.git(&["switch", "--quiet", "develop"]);
    fixture.commit("a.txt", "clash", "feat: clash");

    let output = stack(&fixture, &["restack", "develop"]);

    let commit = fixture.git(&["rev-parse", "--short=7", "feat/a"]);
    assert_eq!(
        String::from_utf8(output.stdout.clone()).unwrap(),
        format!(
            "Conflict: feat/a \"feat: a\" ({commit}) conflicts with develop in a.txt\n\
             Resolving feat/a: fix the conflicts in a.txt, `git add` them, then run `stack continue` (or `stack abort`)\n"
        )
    );
    assert!(stdout(&stack(&fixture, &["status"])).ends_with(
        "Restacking develop: waiting on a conflict in feat/a (`stack continue` or `stack abort`)\n"
    ));
    assert_eq!(
        stderr(&stack(&fixture, &["continue"])),
        "error: resolve the conflicts in a.txt and `git add` them first\n"
    );

    fixture.write("a.txt", "resolved");
    fixture.git(&["add", "a.txt"]);
    assert_eq!(
        stdout(&stack(&fixture, &["continue"])),
        "Restacked feat/b onto feat/a (1 commit)\n"
    );
    assert_eq!(
        fixture.git(&["log", "--format=%s", "develop..feat/b"]),
        "feat: b\nfeat: a"
    );
    assert_eq!(
        stderr(&stack(&fixture, &["continue"])),
        "error: no restack is waiting to continue\n"
    );
}

#[test]
fn abort_stops_resolving() {
    let fixture = advanced();
    fixture.git(&["switch", "--quiet", "develop"]);
    fixture.commit("a.txt", "clash", "feat: clash");
    stack(&fixture, &["restack", "develop"]);

    assert!(stdout(&stack(&fixture, &["abort"])).starts_with("Stopped resolving feat/a"));
    assert!(!fixture.path().join(".git/rebase-merge").exists());
}

#[test]
fn chain_and_multi_move() {
    let fixture = advanced();
    fixture.git(&["switch", "--quiet", "--create", "feat/x", "develop"]);
    fixture.commit("x.txt", "x", "feat: x");

    let chained = stdout(&stack(&fixture, &["chain", "feat/x", "feat/a"]));
    assert!(
        chained.starts_with("Restacked feat/a onto feat/x (1 commit)\n"),
        "{chained}"
    );

    let moved = stdout(&stack(
        &fixture,
        &["move", "feat/a", "feat/b", "--onto", "develop"],
    ));
    assert!(
        moved.contains("Restacked feat/b onto develop (1 commit)"),
        "{moved}"
    );
}

#[test]
fn amend_into_a_parent_branch() {
    let fixture = advanced();
    fixture.write("a.txt", "a, fixed");
    fixture.git(&["add", "a.txt"]);
    let old = fixture.git(&["rev-parse", "--short=7", "feat/a"]);

    let output = stdout(&stack(&fixture, &["amend", "--into", "feat/a"]));

    let new = fixture.git(&["rev-parse", "--short=7", "feat/a"]);
    assert_eq!(
        output,
        format!("Amended {old} into {new} on feat/a\nRestacked feat/b onto feat/a (1 commit)\n")
    );
    assert_eq!(fixture.git(&["show", "feat/b:a.txt"]), "a, fixed");
}
