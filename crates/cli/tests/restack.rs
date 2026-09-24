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

    let output = stack(&fixture, &["restack", "develop"]);

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
    assert_eq!(
        stderr(&output),
        "error: restack stopped at a conflict in feat/a\n"
    );
}

#[test]
fn following_the_conflict_instructions_finishes_the_restack() {
    let fixture = advanced();
    let offshoot = fixture.git(&["rev-parse", "develop~1"]);
    fixture.git(&["switch", "--quiet", "develop"]);
    fixture.commit("a.txt", "clash", "feat: clash");
    stack(&fixture, &["restack", "develop"]);

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
