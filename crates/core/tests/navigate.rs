//! Stepping through stacks, switching, and creating branches.

mod common;

use common::*;
use stack_core::{Error, Step};
use stack_testkit::Fixture;

/// ```text
/// develop
/// └─ a
///    └─ b
///       ├─ c (limb)
///       │  └─ d
///       └─ s
/// ```
fn stacks() -> Fixture {
    let fixture = repo();
    grow(&fixture, "a", "develop");
    grow(&fixture, "b", "a");
    grow(&fixture, "s", "b");
    grow(&fixture, "c", "b");
    workspace(&fixture).add_trunk("c").unwrap();
    grow(&fixture, "d", "c");
    fixture
}

fn step(fixture: &Fixture, from: &str, step: Step) -> Result<String, Error> {
    workspace(fixture).step(from, step)
}

#[test]
fn down_and_up() {
    let fixture = stacks();
    assert_eq!(step(&fixture, "b", Step::Down(1)).unwrap(), "a");
    assert_eq!(step(&fixture, "b", Step::Down(2)).unwrap(), "develop");
    assert_eq!(step(&fixture, "develop", Step::Up(2)).unwrap(), "b");
    assert_eq!(step(&fixture, "c", Step::Up(1)).unwrap(), "d");
    assert!(matches!(
        step(&fixture, "develop", Step::Down(1)),
        Err(Error::NoParent { .. })
    ));
    assert!(matches!(
        step(&fixture, "d", Step::Up(1)),
        Err(Error::NoChildren { .. })
    ));
    let forked = step(&fixture, "b", Step::Up(1));
    assert!(
        matches!(forked, Err(Error::SeveralChildren { ref children, .. }) if children == &["c", "s"]),
        "{forked:?}"
    );
}

#[test]
fn top_and_bottom() {
    let fixture = stacks();
    assert_eq!(step(&fixture, "c", Step::Top).unwrap(), "d");
    assert_eq!(step(&fixture, "d", Step::Top).unwrap(), "d");
    assert!(matches!(
        step(&fixture, "a", Step::Top),
        Err(Error::SeveralChildren { .. })
    ));
    assert_eq!(step(&fixture, "s", Step::Bottom).unwrap(), "a");
    assert_eq!(step(&fixture, "d", Step::Bottom).unwrap(), "c");
    assert_eq!(step(&fixture, "c", Step::Bottom).unwrap(), "c");
    assert!(matches!(
        step(&fixture, "develop", Step::Bottom),
        Err(Error::IsTrunk { .. })
    ));
}

#[test]
fn switch_uses_git_and_refuses_to_lose_changes() {
    let fixture = stacks();
    workspace(&fixture).switch("a").unwrap();
    assert_eq!(fixture.git(&["symbolic-ref", "--short", "HEAD"]), "a");

    fixture.write("a.txt", "local change");
    let before = fixture.snapshot();
    assert!(matches!(
        workspace(&fixture).switch("develop"),
        Err(Error::Git(_))
    ));
    fixture.assert_unchanged(&before);
}

#[test]
fn create_makes_a_child_of_the_current_branch() {
    let fixture = stacks();
    fixture.git(&["switch", "--quiet", "s"]);

    workspace(&fixture).create("t", None, false).unwrap();

    assert_eq!(fixture.git(&["symbolic-ref", "--short", "HEAD"]), "t");
    assert_eq!(parent(&tree(&fixture), "t").unwrap().name, "s");
}

#[test]
fn create_with_a_message_commits_tracked_changes() {
    let fixture = stacks();
    fixture.git(&["switch", "--quiet", "s"]);
    fixture.write("s.txt", "changed");

    workspace(&fixture)
        .create("t", Some("feat: t"), true)
        .unwrap();

    assert_eq!(fixture.git(&["log", "-1", "--format=%s", "t"]), "feat: t");
    assert_eq!(fixture.git(&["status", "--porcelain"]), "");
    assert_eq!(shape(&tree(&fixture)), "develop(a(b(c(d) s(t))))");
}
