use std::process::Output;

use stack_testkit::Fixture;

fn stack(fixture: &Fixture, args: &[&str]) -> Output {
    fixture
        .command(env!("CARGO_BIN_EXE_stack"))
        .args(args)
        .output()
        .unwrap()
}

fn stdout(output: &Output) -> String {
    assert!(
        output.status.success(),
        "stack failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout.clone()).unwrap()
}

#[test]
fn status_prints_branch_and_short_sha() {
    let fixture = Fixture::new();
    let tip = fixture.commit("a.txt", "a", "feat: a");
    let before = fixture.snapshot();

    let output = stack(&fixture, &["status"]);

    assert_eq!(
        stdout(&output),
        format!("On branch develop ({})\n", &tip[..7])
    );
    fixture.assert_unchanged(&before);
}

#[test]
fn status_json_is_the_core_view_model() {
    let fixture = Fixture::new();
    let tip = fixture.commit("a.txt", "a", "feat: a");
    fixture.git(&["switch", "--quiet", "--detach"]);
    let before = fixture.snapshot();

    let json: serde_json::Value =
        serde_json::from_str(&stdout(&stack(&fixture, &["status", "--json"]))).unwrap();

    assert_eq!(
        json,
        serde_json::json!({ "head": { "kind": "detached", "commit": tip } })
    );
    fixture.assert_unchanged(&before);
}

#[test]
fn status_on_unborn_branch() {
    let fixture = Fixture::new();

    assert_eq!(
        stdout(&stack(&fixture, &["status"])),
        "On branch develop (no commits yet)\n"
    );
}

#[test]
fn directory_flag_changes_target_repo() {
    let fixture = Fixture::new();
    let elsewhere = tempfile::tempdir().unwrap();
    let repo = fixture.path();

    let output = fixture
        .command(env!("CARGO_BIN_EXE_stack"))
        .current_dir(elsewhere.path())
        .args(["-C".as_ref(), repo.as_os_str(), "status".as_ref()])
        .output()
        .unwrap();

    assert_eq!(stdout(&output), "On branch develop (no commits yet)\n");
}

#[test]
fn outside_a_repo_fails_with_message() {
    let fixture = Fixture::new();
    let elsewhere = tempfile::tempdir().unwrap();

    let output = fixture
        .command(env!("CARGO_BIN_EXE_stack"))
        .current_dir(elsewhere.path())
        .arg("status")
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).starts_with("error: not a git repository"));
}
