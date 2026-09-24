mod common;

use common::{stack, stderr, stdout};
use stack_testkit::Fixture;

/// `develop` (trunk) ← `feat/a` ← `feat/b`, HEAD on `develop`.
fn repo() -> Fixture {
    let fixture = Fixture::new();
    fixture.commit("base.txt", "base", "feat: base");
    stack(&fixture, &["trunk", "add", "develop"]);
    fixture.write("a.txt", "a");
    fixture.git(&["add", "a.txt"]);
    stdout(&stack(
        &fixture,
        &["create", "feat/a", "--message", "feat: a"],
    ));
    fixture.write("b.txt", "b");
    fixture.git(&["add", "b.txt"]);
    stdout(&stack(&fixture, &["create", "feat/b", "-m", "feat: b"]));
    fixture.git(&["switch", "--quiet", "develop"]);
    fixture
}

fn head(fixture: &Fixture) -> String {
    fixture.git(&["symbolic-ref", "--short", "HEAD"])
}

#[test]
fn walks_up_and_down_the_stack() {
    let fixture = repo();

    assert_eq!(stdout(&stack(&fixture, &["up"])), "Switched to feat/a\n");
    assert_eq!(stdout(&stack(&fixture, &["top"])), "Switched to feat/b\n");
    assert_eq!(stdout(&stack(&fixture, &["top"])), "Already on feat/b\n");
    assert_eq!(
        stdout(&stack(&fixture, &["bottom"])),
        "Switched to feat/a\n"
    );
    assert_eq!(stdout(&stack(&fixture, &["down"])), "Switched to develop\n");
    assert_eq!(
        stdout(&stack(&fixture, &["up", "2"])),
        "Switched to feat/b\n"
    );
    assert_eq!(head(&fixture), "feat/b");
    assert_eq!(
        stderr(&stack(&fixture, &["up"])),
        "error: feat/b has no branches on it\n"
    );
}

#[test]
fn create_builds_a_stack_that_tracks_itself() {
    let fixture = repo();
    let tree = stdout(&stack(&fixture, &["tree"]));

    assert!(tree.contains("└─ feat/a"), "{tree}");
    assert!(tree.contains("   └─ feat/b"), "{tree}");
    assert_eq!(
        fixture.git(&["log", "--format=%s", "develop..feat/b"]),
        "feat: b\nfeat: a"
    );
}
