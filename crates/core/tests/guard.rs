//! The push guard's rules.

mod common;

use common::*;
use stack_core::GuardViolation;

const ZERO: &str = "0000000000000000000000000000000000000000";

#[test]
fn rules_for_each_kind_of_push_line() {
    let fixture = repo();
    grow(&fixture, "a", "develop");
    fixture.git(&["config", "--add", "stack.protect", "release"]);
    let a = tip(&fixture, "a");
    let workspace = workspace(&fixture);
    let check = |line: String| workspace.check_push(&line).unwrap();

    // Same name, via the ref or via HEAD on that branch: fine.
    assert!(check(format!("refs/heads/a {a} refs/heads/a {ZERO}\n")).is_empty());
    assert!(check(format!("HEAD {a} refs/heads/a {ZERO}\n")).is_empty());
    // Tags and deleting unprotected branches: fine.
    assert!(check(format!("refs/tags/v1 {a} refs/tags/v1 {ZERO}\n")).is_empty());
    assert!(check(format!("(delete) {ZERO} refs/heads/old {a}\n")).is_empty());
    // Another name, or a bare commit: flagged.
    assert_eq!(
        check(format!("refs/heads/a {a} refs/heads/b {ZERO}\n")),
        [GuardViolation::NameMismatch {
            local: "a".into(),
            remote: "b".into()
        }]
    );
    assert_eq!(
        check(format!("{a} {a} refs/heads/b {ZERO}\n")),
        [GuardViolation::NameMismatch {
            local: a.clone(),
            remote: "b".into()
        }]
    );
    // Trunks and configured branches: flagged, pushes and deletions alike.
    assert_eq!(
        check(format!(
            "refs/heads/develop {a} refs/heads/develop {ZERO}\n(delete) {ZERO} refs/heads/release {a}\n"
        )),
        [
            GuardViolation::Protected {
                branch: "develop".into(),
                deleting: false
            },
            GuardViolation::Protected {
                branch: "release".into(),
                deleting: true
            },
        ]
    );
    assert_eq!(
        workspace.protected_branches().unwrap(),
        ["develop", "release"]
    );
}

#[test]
fn install_and_uninstall_leave_no_trace() {
    let fixture = repo();
    let before = fixture.snapshot();
    let workspace = workspace(&fixture);

    let installed = workspace
        .install_guard(std::path::Path::new("/usr/local/bin/stack"))
        .unwrap();
    assert!(!installed.chained && workspace.guard_installed().unwrap());
    let script = std::fs::read_to_string(&installed.hook).unwrap();
    assert!(
        script.contains("'/usr/local/bin/stack' guard check-push"),
        "{script}"
    );

    assert!(workspace.uninstall_guard().unwrap());
    assert!(!workspace.uninstall_guard().unwrap());
    fixture.assert_unchanged(&before);
}
