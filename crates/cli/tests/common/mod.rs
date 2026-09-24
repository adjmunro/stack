#![allow(dead_code)]

use std::process::Output;

use stack_testkit::Fixture;

pub fn stack(fixture: &Fixture, args: &[&str]) -> Output {
    fixture
        .command(env!("CARGO_BIN_EXE_stack"))
        .args(args)
        .output()
        .unwrap()
}

/// Stdout of a successful run.
pub fn stdout(output: &Output) -> String {
    assert!(
        output.status.success(),
        "stack failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout.clone()).unwrap()
}

/// Stderr of a failed run.
pub fn stderr(output: &Output) -> String {
    assert!(!output.status.success(), "stack unexpectedly succeeded");
    String::from_utf8(output.stderr.clone()).unwrap()
}
