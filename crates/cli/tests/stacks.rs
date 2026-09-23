mod common;

use common::{stack, stderr, stdout};
use stack_testkit::Fixture;

/// `develop` ← `feat/a` ← `feat/b`, plus `feat/c` on `develop`. HEAD on `feat/c`. Nothing tracked.
fn repo() -> Fixture {
    let fixture = Fixture::new();
    fixture.commit("base.txt", "base", "feat: base");
    fixture.git(&["switch", "--quiet", "--create", "feat/a"]);
    fixture.commit("a.txt", "a", "feat: a");
    fixture.git(&["switch", "--quiet", "--create", "feat/b"]);
    fixture.commit("b.txt", "b", "feat: b");
    fixture.git(&["switch", "--quiet", "--create", "feat/c", "develop"]);
    fixture.commit("c.txt", "c", "feat: c");
    fixture
}

fn short(fixture: &Fixture, branch: &str) -> String {
    fixture.git(&["rev-parse", "--short=7", branch])
}

#[test]
fn builds_and_renders_a_tree() {
    let fixture = repo();

    assert_eq!(
        stdout(&stack(&fixture, &["trunk", "add", "develop"])),
        "Added trunk develop\n"
    );
    assert_eq!(
        stdout(&stack(
            &fixture,
            &["track", "feat/a", "--parent", "develop"]
        )),
        "Tracked feat/a on develop\n"
    );
    assert_eq!(
        stdout(&stack(&fixture, &["track", "feat/b", "-p", "feat/a"])),
        "Tracked feat/b on feat/a\n"
    );
    // Defaults to the current branch.
    assert_eq!(
        stdout(&stack(&fixture, &["track", "-p", "develop"])),
        "Tracked feat/c on develop\n"
    );

    let [develop, a, b, c] =
        ["develop", "feat/a", "feat/b", "feat/c"].map(|branch| short(&fixture, branch));
    assert_eq!(
        stdout(&stack(&fixture, &["tree"])),
        format!(
            "develop {develop}\n\
             ├─ feat/a {a}\n\
             │  └─ feat/b {b}\n\
             └─ * feat/c {c}\n"
        )
    );
}

#[test]
fn repeated_commands_report_no_change() {
    let fixture = repo();
    stack(&fixture, &["trunk", "add", "develop"]);
    stack(&fixture, &["track", "-p", "develop"]);
    let before = fixture.snapshot();

    assert_eq!(
        stdout(&stack(&fixture, &["trunk", "add", "develop"])),
        "develop is already a trunk\n"
    );
    assert_eq!(
        stdout(&stack(&fixture, &["track", "-p", "develop"])),
        "feat/c is already tracked on develop\n"
    );
    assert_eq!(
        stdout(&stack(&fixture, &["track", "--json", "-p", "develop"])),
        "\"unchanged\"\n"
    );
    fixture.assert_unchanged(&before);
}

#[test]
fn tree_json_is_the_core_view_model() {
    let fixture = repo();
    stack(&fixture, &["trunk", "add", "develop"]);
    stack(&fixture, &["track", "-p", "develop"]);

    let json: serde_json::Value =
        serde_json::from_str(&stdout(&stack(&fixture, &["tree", "--json"]))).unwrap();

    let commit = |branch: &str| fixture.git(&["rev-parse", branch]);
    assert_eq!(
        json,
        serde_json::json!({
            "trunks": [{
                "name": "develop",
                "commit": commit("develop"),
                "current": false,
                "children": [{ "name": "feat/c", "commit": commit("feat/c"), "current": true, "children": [] }],
            }],
            "orphans": [],
        })
    );
}

#[test]
fn tree_without_trunks_explains_how_to_add_one() {
    let fixture = repo();
    assert_eq!(
        stdout(&stack(&fixture, &["tree"])),
        "No trunks. Add one with `stack trunk add <branch>`.\n"
    );
}

#[test]
fn deleted_and_orphaned_branches_are_shown() {
    let fixture = repo();
    stack(&fixture, &["trunk", "add", "develop"]);
    stack(&fixture, &["track", "feat/a", "-p", "develop"]);
    stack(&fixture, &["track", "feat/b", "-p", "feat/a"]);
    fixture.git(&["switch", "--quiet", "develop"]);
    fixture.git(&["branch", "--quiet", "-D", "feat/b"]);
    fixture.git(&["update-ref", "-d", "refs/stack/branches/feat/a"]);

    let develop = short(&fixture, "develop");
    assert_eq!(
        stdout(&stack(&fixture, &["tree"])),
        format!("* develop {develop}\n\nParent missing:\nfeat/b (deleted)\n")
    );
}

#[test]
fn errors_exit_non_zero_without_changes() {
    let fixture = repo();
    stack(&fixture, &["trunk", "add", "develop"]);
    stack(&fixture, &["track", "feat/a", "-p", "develop"]);
    stack(&fixture, &["track", "feat/b", "-p", "feat/a"]);
    let before = fixture.snapshot();

    let cases: [(&[&str], &str); 5] = [
        (
            &["untrack", "feat/a"],
            "error: feat/a has tracked children: feat/b\n",
        ),
        (
            &["trunk", "remove", "develop"],
            "error: develop has tracked children: feat/a\n",
        ),
        (
            &["track", "feat/a", "-p", "feat/b"],
            "error: feat/b is feat/a or stacked on it; that would create a cycle\n",
        ),
        (&["track", "-p", "nope"], "error: no such branch: nope\n"),
        (&["untrack"], "error: feat/c is not tracked\n"),
    ];
    for (args, message) in cases {
        assert_eq!(stderr(&stack(&fixture, args)), message, "{args:?}");
    }
    fixture.assert_unchanged(&before);
}

#[test]
fn detached_head_needs_an_explicit_branch() {
    let fixture = repo();
    stack(&fixture, &["trunk", "add", "develop"]);
    fixture.git(&["switch", "--quiet", "--detach"]);

    assert_eq!(
        stderr(&stack(&fixture, &["track", "-p", "develop"])),
        "error: HEAD is detached; name a branch\n"
    );
}

#[test]
fn untrack_and_remove_trunk_undo_setup() {
    let fixture = repo();
    let before = fixture.snapshot();
    stack(&fixture, &["trunk", "add", "develop"]);
    stack(&fixture, &["track", "-p", "develop"]);

    assert_eq!(stdout(&stack(&fixture, &["untrack"])), "Untracked feat/c\n");
    assert_eq!(
        stdout(&stack(&fixture, &["trunk", "remove", "develop"])),
        "Removed trunk develop\n"
    );

    // Only the now-unreferenced metadata blobs remain, for git gc to collect.
    let diff = before.diff(&fixture.snapshot());
    assert!(
        diff.refs.is_empty()
            && diff.head.is_none()
            && diff.index.is_empty()
            && diff.worktree.is_empty(),
        "{diff}"
    );
    assert_eq!(diff.objects_added.len(), 2);
}
