mod common;

use common::*;
use stack_core::{Error, Outcome, Source};

#[test]
fn pin_records_the_resolved_parent_in_one_ref() {
    let fixture = repo();
    let a = grow(&fixture, "a", "develop");
    grow(&fixture, "b", "a");
    let before = fixture.snapshot();

    assert_eq!(
        workspace(&fixture).pin("b", None).unwrap().outcome,
        Outcome::Changed
    );

    let diff = before.diff(&fixture.snapshot());
    assert_eq!(
        diff.refs.keys().collect::<Vec<_>>(),
        ["refs/stack/branches/b"]
    );
    assert_eq!(diff.objects_added.len(), 1);
    assert!(
        diff.head.is_none() && diff.index.is_empty() && diff.worktree.is_empty() && !diff.config
    );
    assert_eq!(
        fixture.git(&["cat-file", "-p", "refs/stack/branches/b"]),
        format!(r#"{{"version":1,"parent":"a","offshoot":"{a}","pinned":true}}"#)
    );
    assert_eq!(parent(&tree(&fixture), "b").unwrap().source, Source::Pinned);
}

#[test]
fn pinning_past_a_branch_overrides_the_graph_and_is_flagged() {
    let fixture = repo();
    grow(&fixture, "a", "develop");
    grow(&fixture, "b", "a");

    workspace(&fixture).pin("b", Some("develop")).unwrap();

    let tree = tree(&fixture);
    assert_eq!(shape(&tree), "develop(a b)");
    let b = parent(&tree, "b").unwrap();
    assert_eq!(
        (b.source, b.contradicted, b.offshoot),
        (Source::Pinned, true, tip(&fixture, "develop"))
    );
}

#[test]
fn pin_fixes_a_same_commit_tie_break_without_a_flag() {
    let fixture = repo();
    grow(&fixture, "older", "develop");
    fixture.git(&["branch", "newer", "older"]);
    grow(&fixture, "child", "newer");

    workspace(&fixture).pin("child", Some("newer")).unwrap();

    let tree = tree(&fixture);
    assert_eq!(shape(&tree), "develop(older(newer(child)))");
    assert!(!parent(&tree, "child").unwrap().contradicted);
}

#[test]
fn pinning_again_is_unchanged() {
    let fixture = repo();
    grow(&fixture, "a", "develop");
    workspace(&fixture).pin("a", None).unwrap();
    let before = fixture.snapshot();

    assert_eq!(
        workspace(&fixture)
            .pin("a", Some("develop"))
            .unwrap()
            .outcome,
        Outcome::Unchanged
    );
    fixture.assert_unchanged(&before);
}

#[test]
fn invalid_pins_change_nothing() {
    let fixture = repo();
    grow(&fixture, "a", "develop");
    grow(&fixture, "b", "a");
    fixture.git(&["switch", "--quiet", "--orphan", "pages"]);
    fixture.commit("index.html", "hi", "feat: pages");
    let before = fixture.snapshot();
    let workspace = workspace(&fixture);

    assert!(matches!(
        workspace.pin("a", Some("b")),
        Err(Error::Cycle { .. })
    ));
    assert!(matches!(
        workspace.pin("a", Some("a")),
        Err(Error::Cycle { .. })
    ));
    assert!(matches!(
        workspace.pin("develop", Some("a")),
        Err(Error::IsTrunk { .. })
    ));
    assert!(matches!(
        workspace.pin("a", Some("nope")),
        Err(Error::UnknownBranch { .. })
    ));
    assert!(matches!(
        workspace.pin("a", Some("pages")),
        Err(Error::Unrelated { .. })
    ));
    assert!(matches!(
        workspace.pin("pages", None),
        Err(Error::NoParent { .. })
    ));
    for name in ["../escape", "a..b", "trailing/", "with space", "lock.lock"] {
        let result = workspace.pin(name, None);
        assert!(
            matches!(result, Err(Error::InvalidRefName { .. })),
            "{name}: {result:?}"
        );
    }
    fixture.assert_unchanged(&before);
}

#[test]
fn unpin_deletes_only_the_record() {
    let fixture = repo();
    grow(&fixture, "a", "develop");
    workspace(&fixture).pin("a", None).unwrap();
    let before = fixture.snapshot();

    assert_eq!(workspace(&fixture).unpin("a").unwrap(), Outcome::Changed);

    let diff = before.diff(&fixture.snapshot());
    assert_eq!(
        diff.refs.keys().collect::<Vec<_>>(),
        ["refs/stack/branches/a"]
    );
    assert!(diff.objects_added.is_empty() && diff.objects_removed.is_empty());
    assert_eq!(
        parent(&tree(&fixture), "a").unwrap().source,
        Source::Derived
    );
}

#[test]
fn unpin_rejects_unpinned_records() {
    let fixture = repo();
    let a = grow(&fixture, "a", "develop");
    grow(&fixture, "b", "a");
    record(&fixture, "b", "a", &a, false);

    assert!(matches!(
        workspace(&fixture).unpin("b"),
        Err(Error::NotPinned { .. })
    ));
    assert!(matches!(
        workspace(&fixture).unpin("a"),
        Err(Error::NotPinned { .. })
    ));
}
