mod common;

use common::{stack, stderr, stdout};
use stack_testkit::Fixture;

/// `develop` (trunk) ← `feat/a`, checked out in the main worktree.
fn repo() -> Fixture {
    let fixture = Fixture::new();
    fixture.commit("base.txt", "base", "feat: base");
    stack(&fixture, &["trunk", "add", "develop"]);
    fixture.git(&["switch", "--quiet", "--create", "feat/a"]);
    fixture.commit("a.txt", "a", "feat: a");
    fixture
}

fn stack_in(fixture: &Fixture, dir: &std::path::Path, args: &[&str]) -> std::process::Output {
    fixture
        .command(env!("CARGO_BIN_EXE_stack"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn follower_lifecycle() {
    let fixture = repo();
    let path = fixture
        .path()
        .parent()
        .unwrap()
        .canonicalize()
        .unwrap()
        .join(".repo-feat-a");

    let added = stdout(&stack(&fixture, &["worktree", "add", "feat/a"]));
    assert!(
        added.starts_with("Created follower of feat/a at "),
        "{added}"
    );
    let list = stdout(&stack(&fixture, &["worktree", "list"]));
    assert!(list.contains("following feat/a (up to date)"), "{list}");

    fixture.git_in(
        &path,
        &[
            "commit",
            "--quiet",
            "--allow-empty",
            "--message",
            "feat: from the follower",
        ],
    );
    let list = stdout(&stack(&fixture, &["worktree", "list"]));
    assert!(
        list.contains("following feat/a (has commits; run `stack land`)"),
        "{list}"
    );

    let landed = stack_in(&fixture, &path, &["land"]);
    assert!(
        stdout(&landed).starts_with("Landed 1 commit on feat/a; moved "),
        "{landed:?}"
    );
    assert_eq!(
        fixture.git(&["log", "-1", "--format=%s", "feat/a"]),
        "feat: from the follower"
    );

    fixture.commit("more.txt", "more", "feat: more");
    let synced = stdout(&stack(&fixture, &["worktree", "sync"]));
    assert!(synced.starts_with("Moved follower "), "{synced}");
}

#[test]
fn restack_moves_followers_and_notes_it() {
    let fixture = repo();
    stack(&fixture, &["worktree", "add", "feat/a"]);
    fixture.git(&["switch", "--quiet", "develop"]);
    fixture.commit("later.txt", "later", "feat: later");

    let output = stack(&fixture, &["restack", "develop"]);

    assert!(
        stderr_of(&output).starts_with("note: Moved follower "),
        "{output:?}"
    );
}

fn stderr_of(output: &std::process::Output) -> String {
    String::from_utf8(output.stderr.clone()).unwrap()
}

#[test]
fn land_outside_a_follower_explains_itself() {
    let fixture = repo();
    assert_eq!(
        stderr(&stack(&fixture, &["land"])),
        "error: this worktree isn't detached; land from a follower worktree\n"
    );
}
