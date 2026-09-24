mod common;

use common::{stack, stderr, stdout};
use stack_testkit::Fixture;

/// `develop` (trunk) ← `feat/a` ← `feat/b`, HEAD on `feat/b`, with a bare `origin`.
fn repo() -> (Fixture, std::path::PathBuf) {
    let fixture = Fixture::new();
    fixture.commit("base.txt", "base", "feat: base");
    stack(&fixture, &["trunk", "add", "develop"]);
    fixture.git(&["switch", "--quiet", "--create", "feat/a"]);
    fixture.commit("a.txt", "a", "feat: a");
    fixture.git(&["switch", "--quiet", "--create", "feat/b"]);
    fixture.commit("b.txt", "b", "feat: b");
    let remote = fixture.add_bare_remote("origin");
    (fixture, remote)
}

#[test]
fn pushes_the_line_then_reports_up_to_date_then_forces_after_a_restack() {
    let (fixture, _remote) = repo();

    assert_eq!(
        stdout(&stack(&fixture, &["push"])),
        "Pushed feat/a to origin (new)\nPushed feat/b to origin (new)\n"
    );
    assert_eq!(
        stdout(&stack(&fixture, &["push", "--rootward", "feat/a"])),
        "feat/a is up to date on origin\n"
    );

    fixture.git(&["switch", "--quiet", "develop"]);
    fixture.commit("later.txt", "later", "feat: later");
    stack(&fixture, &["restack", "develop"]);
    assert_eq!(
        stdout(&stack(&fixture, &["push", "feat/b"])),
        "Pushed feat/a to origin (replaced its rewritten remote branch)\n\
         Pushed feat/b to origin (replaced its rewritten remote branch)\n"
    );
}

#[test]
fn a_rejection_fails_with_advice() {
    let (fixture, remote) = repo();
    stack(&fixture, &["push"]);
    // Someone else points the remote branch elsewhere.
    fixture.git_in(
        &remote,
        &["update-ref", "refs/heads/feat/b", "refs/heads/feat/a"],
    );
    fixture.git(&[
        "commit",
        "--quiet",
        "--amend",
        "--allow-empty",
        "--message",
        "feat: b2",
    ]);

    let output = stack(&fixture, &["push", "--leafward"]);

    assert!(
        String::from_utf8(output.stdout.clone())
            .unwrap()
            .starts_with("Rejected feat/b: [rejected]")
    );
    assert_eq!(
        stderr(&output),
        "error: some branches were rejected; fetch, restack, and push again\n"
    );
}

#[test]
fn trunk_alone_has_nothing_to_push() {
    let (fixture, _remote) = repo();
    fixture.git(&["switch", "--quiet", "develop"]);
    fixture.git(&["branch", "--quiet", "-D", "feat/a", "feat/b"]);

    assert_eq!(stdout(&stack(&fixture, &["push"])), "Nothing to push.\n");
}

#[test]
fn pr_pushes_and_opens_pull_requests_via_gh() {
    let (fixture, _remote) = repo();
    fixture.git(&["push", "--quiet", "origin", "develop"]);
    let bin = fixture.scratch_path("bin");
    let state = fixture.scratch_path("gh-state");
    std::fs::create_dir(&bin).unwrap();
    std::fs::create_dir(&state).unwrap();
    std::fs::copy(concat!(env!("CARGO_MANIFEST_DIR"), "/../core/tests/fake-gh.sh"), bin.join("gh")).unwrap();
    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap());

    let output = fixture
        .command(env!("CARGO_BIN_EXE_stack"))
        .env("PATH", path)
        .env("FAKE_GH_DIR", &state)
        .args(["pr"])
        .output()
        .unwrap();

    assert_eq!(
        stdout(&output),
        "Pushed feat/a to origin (new)\n\
         Pushed feat/b to origin (new)\n\
         Opened #1 for feat/a into develop: https://example.test/pull/1\n\
         Opened #2 for feat/b into feat/a: https://example.test/pull/2\n"
    );
}
