mod common;

use std::fs;
use std::path::Path;

use common::{stack, stderr, stdout};
use stack_testkit::Fixture;

/// Runs `stack` in `dir` with the fixture's isolated environment.
fn stack_in(fixture: &Fixture, dir: &Path, args: &[&str]) -> std::process::Output {
    fixture
        .command(env!("CARGO_BIN_EXE_stack"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap()
}

fn empty_dir(fixture: &Fixture) -> std::path::PathBuf {
    let dir = fixture.scratch_path("new");
    fs::create_dir(&dir).unwrap();
    dir
}

#[test]
fn creates_a_repository_with_yes_and_marks_its_branch() {
    let fixture = Fixture::new();
    let dir = empty_dir(&fixture);

    let output = stdout(&stack_in(&fixture, &dir, &["init", "--yes"]));

    let branch = fixture.git_in(&dir, &["symbolic-ref", "--short", "HEAD"]);
    let shown = dir.canonicalize().unwrap();
    assert_eq!(
        output,
        format!(
            "Created git repository in {}\nAdded trunk {branch}\n",
            shown.display()
        )
    );
    assert_eq!(
        fixture.git_in(
            &dir,
            &["for-each-ref", "--format=%(refname)", "refs/stack/"]
        ),
        format!("refs/stack/trunks/{branch}")
    );
    assert_eq!(
        stdout(&stack_in(&fixture, &dir, &["tree"])),
        "No commits yet.\n"
    );

    fixture.git_in(
        &dir,
        &[
            "commit",
            "--quiet",
            "--allow-empty",
            "--message",
            "feat: first",
        ],
    );
    let tip = fixture.git_in(&dir, &["rev-parse", "--short=7", "HEAD"]);
    assert_eq!(
        stdout(&stack_in(&fixture, &dir, &["tree"])),
        format!("* {branch} {tip}\n")
    );
}

#[test]
fn without_a_terminal_it_refuses_to_create_a_repository() {
    let fixture = Fixture::new();
    let dir = empty_dir(&fixture);

    let error = stderr(&stack_in(&fixture, &dir, &["init"]));

    assert_eq!(
        error,
        "error: not a git repository; run `stack init --yes` to create one\n"
    );
    assert_eq!(fs::read_dir(&dir).unwrap().count(), 0);
}

#[test]
fn marks_the_conventional_trunk_once() {
    let fixture = Fixture::new();
    fixture.commit("base.txt", "base", "feat: base");
    fixture.git(&["switch", "--quiet", "--create", "feature"]);

    assert_eq!(stdout(&stack(&fixture, &["init"])), "Added trunk develop\n");
    let before = fixture.snapshot();
    assert_eq!(
        stdout(&stack(&fixture, &["init"])),
        "develop is already a trunk\n"
    );
    fixture.assert_unchanged(&before);
}

#[test]
fn prefers_the_remote_default_branch() {
    let fixture = Fixture::new();
    fixture.commit("base.txt", "base", "feat: base");
    fixture.git(&["branch", "main"]);
    fixture.add_bare_remote("origin");
    fixture.git(&["push", "--quiet", "origin", "develop", "main"]);
    fixture.git(&[
        "symbolic-ref",
        "refs/remotes/origin/HEAD",
        "refs/remotes/origin/main",
    ]);

    assert_eq!(stdout(&stack(&fixture, &["init"])), "Added trunk main\n");
}

#[test]
fn marks_explicit_trunks_and_infers_limbs() {
    let fixture = Fixture::new();
    fixture.commit("base.txt", "base", "feat: base");
    fixture.git(&["switch", "--quiet", "--create", "a"]);
    fixture.commit("a.txt", "a", "feat: a");
    fixture.git(&["switch", "--quiet", "--create", "rel"]);
    fixture.commit("rel.txt", "rel", "feat: rel");

    let output = stdout(&stack(
        &fixture,
        &["init", "--trunk", "develop", "-t", "rel"],
    ));

    assert_eq!(
        output,
        "Added trunk develop\nAdded trunk rel (stacked on a)\n"
    );
}

#[test]
fn reports_when_no_trunk_can_be_found() {
    let fixture = Fixture::new();
    fixture.commit("base.txt", "base", "feat: base");
    fixture.git(&["branch", "--quiet", "--move", "work"]);
    fixture.git(&["switch", "--quiet", "--detach"]);
    let before = fixture.snapshot();

    assert_eq!(
        stdout(&stack(&fixture, &["init"])),
        "No trunk found; add one with `stack trunk add <branch>`\n"
    );
    fixture.assert_unchanged(&before);
}
