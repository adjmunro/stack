mod common;

use common::{stack, stderr, stdout};
use stack_testkit::Fixture;

/// `develop` ← `feat/a` ← `feat/b`, plus `feat/c` on `develop`. HEAD on `feat/c`. No trunks.
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
fn tree_is_derived_from_the_graph_once_a_trunk_exists() {
    let fixture = repo();
    assert_eq!(
        stdout(&stack(&fixture, &["tree"])),
        "No trunks. Add one with `stack trunk add <branch>`.\n"
    );

    assert_eq!(
        stdout(&stack(&fixture, &["trunk", "add", "develop"])),
        "Added trunk develop\n"
    );
    let before = fixture.snapshot();

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
    fixture.assert_unchanged(&before);
}

#[test]
fn tree_shows_limbs_pins_moves_and_restack_needs() {
    let fixture = repo();
    stack(&fixture, &["trunk", "add", "develop"]);
    assert_eq!(
        stdout(&stack(&fixture, &["trunk", "add", "feat/b"])),
        "Added trunk feat/b (stacked on feat/a)\n"
    );
    assert_eq!(
        stdout(&stack(&fixture, &["pin", "feat/a"])),
        "Pinned feat/a on develop\n"
    );
    fixture.git(&["switch", "--quiet", "develop"]);
    fixture.commit("later.txt", "later", "feat: later");

    let [develop, a, b, c] =
        ["develop", "feat/a", "feat/b", "feat/c"].map(|branch| short(&fixture, branch));
    assert_eq!(
        stdout(&stack(&fixture, &["tree"])),
        format!(
            "* develop {develop}\n\
             ├─ feat/a {a} [pinned] (needs restack)\n\
             │  └─ feat/b {b} [trunk]\n\
             └─ feat/c {c} (needs restack)\n"
        )
    );
}

#[test]
fn repeated_commands_report_no_change() {
    let fixture = repo();
    stack(&fixture, &["trunk", "add", "develop"]);
    stack(&fixture, &["pin"]);
    let before = fixture.snapshot();

    assert_eq!(
        stdout(&stack(&fixture, &["trunk", "add", "develop"])),
        "develop is already a trunk\n"
    );
    assert_eq!(
        stdout(&stack(&fixture, &["pin"])),
        "feat/c is already pinned on develop\n"
    );
    let json: serde_json::Value =
        serde_json::from_str(&stdout(&stack(&fixture, &["pin", "--json"]))).unwrap();
    assert_eq!(
        json,
        serde_json::json!({ "outcome": "unchanged", "parent": "develop" })
    );
    fixture.assert_unchanged(&before);
}

#[test]
fn tree_json_is_the_core_view_model() {
    let fixture = repo();
    stack(&fixture, &["trunk", "add", "develop"]);
    fixture.git(&["branch", "--quiet", "-D", "feat/b"]);
    fixture.git(&["branch", "--quiet", "-D", "feat/a"]);

    let json: serde_json::Value =
        serde_json::from_str(&stdout(&stack(&fixture, &["tree", "--json"]))).unwrap();

    let commit = |branch: &str| fixture.git(&["rev-parse", branch]);
    assert_eq!(
        json,
        serde_json::json!({
            "trunks": [{
                "name": "develop",
                "commit": commit("develop"),
                "role": "trunk",
                "current": false,
                "parent": null,
                "children": [{
                    "name": "feat/c",
                    "commit": commit("feat/c"),
                    "role": "branch",
                    "current": true,
                    "parent": {
                        "name": "develop",
                        "offshoot": commit("develop"),
                        "source": "derived",
                        "needs_restack": false,
                        "contradicted": false,
                        "replaces": null,
                    },
                    "children": [],
                }],
            }],
            "unattached": [],
        })
    );
}

#[test]
fn unattached_branches_are_listed_separately() {
    let fixture = repo();
    stack(&fixture, &["trunk", "add", "develop"]);
    fixture.git(&["switch", "--quiet", "--orphan", "pages"]);
    fixture.commit("index.html", "hi", "feat: pages");

    let output = stdout(&stack(&fixture, &["tree"]));

    let pages = short(&fixture, "pages");
    assert!(
        output.ends_with(&format!("\nUnattached:\n* pages {pages}\n")),
        "{output}"
    );
}

#[test]
fn errors_exit_non_zero_without_changes() {
    let fixture = repo();
    stack(&fixture, &["trunk", "add", "develop"]);
    let before = fixture.snapshot();

    let cases: [(&[&str], &str); 5] = [
        (
            &["pin", "feat/a", "-p", "feat/b"],
            "error: feat/b is feat/a or stacked on it; that would create a cycle\n",
        ),
        (&["pin", "-p", "nope"], "error: no such branch: nope\n"),
        (&["pin", "develop"], "error: develop is a trunk\n"),
        (&["unpin"], "error: feat/c is not pinned\n"),
        (
            &["trunk", "remove", "feat/a"],
            "error: feat/a is not a trunk\n",
        ),
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
        stderr(&stack(&fixture, &["pin"])),
        "error: HEAD is detached; name a branch\n"
    );
}

#[test]
fn unpin_and_trunk_remove_undo_setup() {
    let fixture = repo();
    let before = fixture.snapshot();
    stack(&fixture, &["trunk", "add", "develop"]);
    stack(&fixture, &["pin"]);

    assert_eq!(stdout(&stack(&fixture, &["unpin"])), "Unpinned feat/c\n");
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
