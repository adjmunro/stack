mod common;

use common::{stack, stdout};
use stack_testkit::Fixture;

#[test]
fn mark_review_and_unmark() {
    let fixture = Fixture::new();
    fixture.commit("base.txt", "base", "feat: base");
    stack(&fixture, &["trunk", "add", "develop"]);
    fixture.git(&["switch", "--quiet", "--create", "feat/a"]);
    let first = fixture.commit("a.txt", "a", "feat: a");
    fixture.commit("a2.txt", "a2", "feat: a2");
    let short = |commit: &str| commit[..7].to_owned();
    let head = fixture.git(&["rev-parse", "HEAD"]);

    assert_eq!(
        stdout(&stack(&fixture, &["mark"])),
        "Marked 1 commit reviewed\n"
    );
    assert_eq!(
        stdout(&stack(
            &fixture,
            &["mark", &first, "--flagged", "--note", "naming"]
        )),
        "Marked 1 commit flagged\n"
    );
    assert_eq!(
        stdout(&stack(&fixture, &["review"])),
        format!(
            "{} feat: a2 [reviewed]\n{} feat: a [flagged: naming]\n",
            short(&head),
            short(&first)
        )
    );
    assert_eq!(
        stdout(&stack(
            &fixture,
            &["mark", "--branch", "feat/a", "--tested"]
        )),
        "Marked 2 commits tested\n"
    );
    assert_eq!(
        stdout(&stack(&fixture, &["unmark", &first, "--kind", "flagged"])),
        "Removed 1 mark\n"
    );
    assert_eq!(
        stdout(&stack(&fixture, &["review", "feat/a"])),
        format!(
            "{} feat: a2 [reviewed, tested]\n{} feat: a [tested]\n",
            short(&head),
            short(&first)
        )
    );
}

#[test]
fn lost_lists_amended_away_commits_with_a_restore_hint() {
    let fixture = Fixture::new();
    fixture.commit("base.txt", "base", "feat: base");
    fixture.git(&["switch", "--quiet", "--create", "feat/a"]);
    let old = fixture.commit("a.txt", "a", "feat: a");
    fixture.git(&[
        "commit",
        "--quiet",
        "--amend",
        "--message",
        "feat: a, reworded",
    ]);

    let output = stdout(&stack(&fixture, &["lost"]));

    let mut lines = output.lines();
    let first = lines.next().unwrap();
    assert!(
        first.starts_with(&format!("{} feat: a (feat/a, ", &old[..7])),
        "{output}"
    );
    assert!(first.ends_with(": commit (amend))"), "{output}");
    assert_eq!(
        lines.next(),
        Some("Restore one with: git branch <name> <commit>")
    );
}

#[test]
fn delta_shows_only_the_real_difference() {
    let fixture = Fixture::new();
    fixture.commit("base.txt", "base", "feat: base");
    stack(&fixture, &["trunk", "add", "develop"]);
    fixture.git(&["switch", "--quiet", "--create", "a"]);
    fixture.commit("feature.txt", "the feature", "feat: feature");
    fixture.git(&["switch", "--quiet", "--create", "x", "develop"]);
    fixture.commit("noise.txt", "noise", "chore: noise");
    fixture.git(&["switch", "--quiet", "--create", "b"]);
    fixture.git(&["cherry-pick", "a"]);
    fixture.commit("extra.txt", "extra", "feat: extra");

    let net = stdout(&stack(&fixture, &["delta", "a", "b"]));
    assert!(
        net.contains("+++ b/extra.txt") && !net.contains("noise.txt"),
        "{net}"
    );

    let commits = stdout(&stack(&fixture, &["delta", "a", "--commits"]));
    assert!(
        commits.contains("feat: feature") && commits.contains("feat: extra"),
        "{commits}"
    );
}

#[test]
fn lint_reports_and_fails() {
    let fixture = Fixture::new();
    fixture.commit("base.txt", "base", "feat: base");
    stack(&fixture, &["trunk", "add", "develop"]);
    fixture.git(&["switch", "--quiet", "--create", "a"]);
    fixture.commit("a.txt", "a", "feat: a");
    assert_eq!(stdout(&stack(&fixture, &["lint"])), "a's commits pass\n");

    let wip = fixture.commit("b.txt", "b", "WIP");
    let output = stack(&fixture, &["lint"]);
    assert_eq!(
        String::from_utf8(output.stdout.clone()).unwrap(),
        format!("{} WIP (doesn't match stack.lint.pattern)\n", &wip[..7])
    );
    assert!(!output.status.success());
}
