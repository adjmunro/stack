//! Commit subject rules.

mod common;

use common::*;
use stack_core::{Error, LintProblem};
use stack_testkit::Fixture;

fn branch_with(subjects: &[&str]) -> Fixture {
    let fixture = repo();
    fixture.git(&["switch", "--quiet", "--create", "a", "develop"]);
    for (index, subject) in subjects.iter().enumerate() {
        fixture.commit(&format!("{index}.txt"), "x", subject);
    }
    fixture
}

fn flagged(fixture: &Fixture) -> Vec<(String, Vec<LintProblem>)> {
    workspace(fixture)
        .lint("a")
        .unwrap()
        .into_iter()
        .map(|finding| (finding.summary, finding.problems))
        .collect()
}

#[test]
fn conventional_commits_pass_by_default() {
    let fixture = branch_with(&["feat: a", "fix(core)!: b", "chore(deps): c"]);
    assert!(flagged(&fixture).is_empty());
}

#[test]
fn flags_nonconforming_and_long_subjects() {
    let long = format!("feat: {}", "x".repeat(70));
    let fixture = branch_with(&["WIP", "fixup! feat: a", &long]);

    let findings = flagged(&fixture);

    assert_eq!(findings.len(), 3);
    assert_eq!(
        findings[0].1,
        [LintProblem::TooLong {
            length: 76,
            max: 72
        }]
    );
    assert!(matches!(
        findings[1].1.as_slice(),
        [LintProblem::NoMatch { .. }]
    ));
    assert!(matches!(
        findings[2].1.as_slice(),
        [LintProblem::NoMatch { .. }]
    ));
}

#[test]
fn rules_come_from_git_config() {
    let fixture = branch_with(&["serde: 1.0.1 -> 1.0.2", "feat: a"]);
    fixture.git(&["config", "stack.lint.pattern", r"^(\w+: |\S+: \S+ -> \S+$)"]);
    fixture.git(&["config", "stack.lint.maxLength", "10"]);

    let findings = flagged(&fixture);

    assert_eq!(
        findings,
        [(
            "serde: 1.0.1 -> 1.0.2".to_owned(),
            vec![LintProblem::TooLong {
                length: 21,
                max: 10
            }]
        )]
    );
}

#[test]
fn broken_rules_are_reported() {
    let fixture = branch_with(&["feat: a"]);
    fixture.git(&["config", "stack.lint.pattern", "("]);
    assert!(matches!(
        workspace(&fixture).lint("a"),
        Err(Error::InvalidLintRule { .. })
    ));
}
