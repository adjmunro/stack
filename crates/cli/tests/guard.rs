mod common;

use std::path::PathBuf;
use std::process::Output;

use common::{stack, stdout};
use stack_testkit::Fixture;

/// `develop` (trunk, pushed) ← `feat/a`, with `origin` and the guard installed. HEAD on `feat/a`.
fn guarded() -> (Fixture, PathBuf) {
    let fixture = Fixture::new();
    fixture.commit("base.txt", "base", "feat: base");
    stack(&fixture, &["trunk", "add", "develop"]);
    let remote = fixture.add_bare_remote("origin");
    fixture.git(&["push", "--quiet", "origin", "develop"]);
    fixture.git(&["switch", "--quiet", "--create", "feat/a"]);
    fixture.commit("a.txt", "a", "feat: a");
    let installed = stdout(&stack(&fixture, &["guard", "install"]));
    assert!(
        installed.starts_with("Installed the push guard at "),
        "{installed}"
    );
    (fixture, remote)
}

/// Runs `git push` as an agent would: no one at a terminal to confirm anything.
fn push(fixture: &Fixture, args: &[&str]) -> Output {
    fixture
        .command("git")
        .env("STACK_GUARD_PROMPT", "never")
        .arg("push")
        .args(args)
        .output()
        .unwrap()
}

fn remote_branches(fixture: &Fixture, remote: &std::path::Path) -> String {
    fixture.git_in(
        remote,
        &["for-each-ref", "--format=%(refname:short)", "refs/heads/"],
    )
}

#[test]
fn same_name_pushes_go_through() {
    let (fixture, remote) = guarded();

    assert!(
        push(&fixture, &["--quiet", "origin", "feat/a"])
            .status
            .success()
    );
    assert_eq!(remote_branches(&fixture, &remote), "develop\nfeat/a");
}

#[test]
fn pushing_under_another_name_is_refused() {
    let (fixture, remote) = guarded();

    let output = push(
        &fixture,
        &["origin", "feat/a:feat/renamed", "HEAD:feat/other"],
    );

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("stack guard: this would push feat/a to origin/feat/renamed"),
        "{stderr}"
    );
    assert!(
        stderr.contains("push refused by the stack guard"),
        "{stderr}"
    );
    assert_eq!(remote_branches(&fixture, &remote), "develop");
}

#[test]
fn protected_branches_are_refused_without_a_human() {
    let (fixture, remote) = guarded();
    fixture.git(&["config", "--add", "stack.protect", "release"]);

    for args in [
        ["origin", "feat/a:develop"],
        ["origin", "HEAD:release"],
        ["origin", ":develop"],
    ] {
        let output = push(&fixture, &args);
        assert!(!output.status.success(), "{args:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("protected origin/"),
            "{args:?}"
        );
    }
    assert_eq!(remote_branches(&fixture, &remote), "develop");
    assert_eq!(
        stdout(&stack(&fixture, &["guard", "status"])),
        "The push guard is installed\nProtected: develop, release\n"
    );
}

#[test]
fn stack_push_passes_the_guard() {
    let (fixture, remote) = guarded();

    assert_eq!(
        stdout(&stack(&fixture, &["push"])),
        "Pushed feat/a to origin (new)\n"
    );
    assert_eq!(remote_branches(&fixture, &remote), "develop\nfeat/a");
}

#[test]
fn an_existing_hook_is_chained_and_restored() {
    let fixture = Fixture::new();
    fixture.commit("base.txt", "base", "feat: base");
    fixture.add_bare_remote("origin");
    let hook = fixture.path().join(".git/hooks/pre-push");
    let ran = fixture.scratch_path("ran");
    std::fs::write(&hook, format!("#!/bin/sh\ncat > '{}'\n", ran.display())).unwrap();
    std::fs::set_permissions(&hook, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();

    let installed = stdout(&stack(&fixture, &["guard", "install"]));
    assert!(
        installed.contains("(your existing pre-push hook runs after it)"),
        "{installed}"
    );

    fixture.git(&["switch", "--quiet", "--create", "feat/b"]);
    assert!(
        push(&fixture, &["--quiet", "origin", "feat/b"])
            .status
            .success()
    );
    assert!(
        std::fs::read_to_string(&ran)
            .unwrap()
            .starts_with("refs/heads/feat/b "),
        "chained hook got the ref lines"
    );

    assert_eq!(
        stdout(&stack(&fixture, &["guard", "uninstall"])),
        "Removed the push guard\n"
    );
    assert!(
        std::fs::read_to_string(&hook)
            .unwrap()
            .starts_with("#!/bin/sh\ncat > ")
    );
    assert_eq!(
        stdout(&stack(&fixture, &["guard", "status"])),
        "The push guard is not installed\nProtected: nothing\n"
    );
}
