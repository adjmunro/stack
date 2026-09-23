use std::collections::{BTreeMap, BTreeSet};

use stack_core::{Error, Node, Outcome, Tree, Workspace};
use stack_testkit::{Fixture, SnapshotDiff};

fn workspace(fixture: &Fixture) -> Workspace {
    Workspace::discover(fixture.path()).unwrap()
}

/// `develop` ← `feat/a` ← `feat/b`, each with one commit, with HEAD on `develop`. Nothing tracked.
fn stacked() -> Fixture {
    let fixture = Fixture::new();
    fixture.commit("base.txt", "base", "feat: base");
    fixture.git(&["switch", "--quiet", "--create", "feat/a"]);
    fixture.commit("a.txt", "a", "feat: a");
    fixture.git(&["switch", "--quiet", "--create", "feat/b"]);
    fixture.commit("b.txt", "b", "feat: b");
    fixture.git(&["switch", "--quiet", "develop"]);
    fixture
}

/// [`stacked`] with `develop` as a trunk and both branches tracked.
fn tracked() -> Fixture {
    let fixture = stacked();
    let workspace = workspace(&fixture);
    workspace.add_trunk("develop").unwrap();
    workspace.track("feat/a", "develop").unwrap();
    workspace.track("feat/b", "feat/a").unwrap();
    fixture
}

fn tip(fixture: &Fixture, branch: &str) -> String {
    fixture.git(&["rev-parse", &format!("refs/heads/{branch}")])
}

fn cat(fixture: &Fixture, reference: &str) -> String {
    fixture.git(&["cat-file", "-p", reference])
}

/// Whether a mutation only set `reference` from `old` to a blob, adding at most that blob (identical blobs are shared).
fn only_ref_set(diff: &SnapshotDiff, reference: &str, old: Option<String>) -> bool {
    let Some((before, Some(new))) = diff.refs.get(reference).cloned() else {
        return false;
    };
    let objects_added = if diff.objects_added.is_empty() {
        BTreeSet::new()
    } else {
        BTreeSet::from([new.clone()])
    };
    before == old
        && *diff
            == SnapshotDiff {
                refs: BTreeMap::from([(reference.into(), (before, Some(new)))]),
                objects_added,
                ..SnapshotDiff::default()
            }
}

fn names(nodes: &[Node]) -> Vec<&str> {
    nodes.iter().map(|node| node.name.as_str()).collect()
}

mod trunks {
    use super::*;

    #[test]
    fn add_writes_exactly_one_blob_ref() {
        let fixture = stacked();
        let before = fixture.snapshot();

        assert_eq!(
            workspace(&fixture).add_trunk("develop").unwrap(),
            Outcome::Changed
        );

        let diff = before.diff(&fixture.snapshot());
        assert!(
            only_ref_set(&diff, "refs/stack/trunks/develop", None),
            "{diff}"
        );
        assert_eq!(
            cat(&fixture, "refs/stack/trunks/develop"),
            r#"{"version":1}"#
        );
    }

    #[test]
    fn adding_twice_is_unchanged() {
        let fixture = stacked();
        workspace(&fixture).add_trunk("develop").unwrap();
        let before = fixture.snapshot();

        assert_eq!(
            workspace(&fixture).add_trunk("develop").unwrap(),
            Outcome::Unchanged
        );
        fixture.assert_unchanged(&before);
    }

    #[test]
    fn add_requires_existing_branch() {
        let fixture = stacked();
        let before = fixture.snapshot();

        assert!(matches!(
            workspace(&fixture).add_trunk("nope"),
            Err(Error::UnknownBranch { .. })
        ));
        fixture.assert_unchanged(&before);
    }

    #[test]
    fn add_rejects_tracked_branch() {
        let fixture = tracked();
        let before = fixture.snapshot();

        assert!(matches!(
            workspace(&fixture).add_trunk("feat/a"),
            Err(Error::IsTracked { .. })
        ));
        fixture.assert_unchanged(&before);
    }

    #[test]
    fn remove_refuses_while_branches_are_stacked_on_it() {
        let fixture = tracked();
        let before = fixture.snapshot();

        let error = workspace(&fixture).remove_trunk("develop").unwrap_err();
        assert!(
            matches!(error, Error::HasChildren { ref children, .. } if children == &["feat/a"])
        );
        fixture.assert_unchanged(&before);
    }

    #[test]
    fn remove_deletes_only_the_trunk_ref() {
        let fixture = stacked();
        workspace(&fixture).add_trunk("develop").unwrap();
        let before = fixture.snapshot();

        assert_eq!(
            workspace(&fixture).remove_trunk("develop").unwrap(),
            Outcome::Changed
        );

        let diff = before.diff(&fixture.snapshot());
        assert_eq!(
            diff.refs.keys().collect::<Vec<_>>(),
            ["refs/stack/trunks/develop"]
        );
        assert_eq!(diff.refs["refs/stack/trunks/develop"].1, None);
        assert_eq!(
            diff,
            SnapshotDiff {
                refs: diff.refs.clone(),
                ..SnapshotDiff::default()
            }
        );
    }

    #[test]
    fn remove_rejects_non_trunk() {
        let fixture = stacked();
        assert!(matches!(
            workspace(&fixture).remove_trunk("develop"),
            Err(Error::NotTrunk { .. })
        ));
    }
}

mod tracking {
    use super::*;

    #[test]
    fn track_records_parent_and_fork_point() {
        let fixture = stacked();
        let fork_point = tip(&fixture, "develop");
        // Advance develop so its tip is no longer the fork point.
        fixture.commit("later.txt", "later", "feat: later");
        workspace(&fixture).add_trunk("develop").unwrap();
        let before = fixture.snapshot();

        assert_eq!(
            workspace(&fixture).track("feat/a", "develop").unwrap(),
            Outcome::Changed
        );

        let diff = before.diff(&fixture.snapshot());
        assert!(
            only_ref_set(&diff, "refs/stack/branches/feat/a", None),
            "{diff}"
        );
        assert_eq!(
            cat(&fixture, "refs/stack/branches/feat/a"),
            format!(r#"{{"version":1,"parent":"develop","base":"{fork_point}"}}"#)
        );
    }

    #[test]
    fn tracking_again_is_unchanged() {
        let fixture = tracked();
        let before = fixture.snapshot();

        assert_eq!(
            workspace(&fixture).track("feat/b", "feat/a").unwrap(),
            Outcome::Unchanged
        );
        fixture.assert_unchanged(&before);
    }

    #[test]
    fn retracking_swaps_only_that_branch_ref() {
        let fixture = tracked();
        let old = fixture.git(&["rev-parse", "refs/stack/branches/feat/b"]);
        let before = fixture.snapshot();

        assert_eq!(
            workspace(&fixture).track("feat/b", "develop").unwrap(),
            Outcome::Changed
        );

        let diff = before.diff(&fixture.snapshot());
        assert!(
            only_ref_set(&diff, "refs/stack/branches/feat/b", Some(old)),
            "{diff}"
        );
    }

    #[test]
    fn parent_must_be_trunk_or_tracked() {
        let fixture = stacked();
        workspace(&fixture).add_trunk("develop").unwrap();
        let before = fixture.snapshot();

        assert!(matches!(
            workspace(&fixture).track("feat/b", "feat/a"),
            Err(Error::UntrackedParent { .. })
        ));
        fixture.assert_unchanged(&before);
    }

    #[test]
    fn trunk_cannot_be_tracked() {
        let fixture = tracked();
        let before = fixture.snapshot();

        assert!(matches!(
            workspace(&fixture).track("develop", "feat/a"),
            Err(Error::IsTrunk { .. })
        ));
        fixture.assert_unchanged(&before);
    }

    #[test]
    fn cycles_are_rejected() {
        let fixture = tracked();
        let before = fixture.snapshot();

        assert!(matches!(
            workspace(&fixture).track("feat/a", "feat/b"),
            Err(Error::Cycle { .. })
        ));
        assert!(matches!(
            workspace(&fixture).track("feat/a", "feat/a"),
            Err(Error::Cycle { .. })
        ));
        fixture.assert_unchanged(&before);
    }

    #[test]
    fn unrelated_histories_are_rejected() {
        let fixture = stacked();
        workspace(&fixture).add_trunk("develop").unwrap();
        fixture.git(&["switch", "--quiet", "--orphan", "other"]);
        fixture.commit("other.txt", "other", "feat: other");
        let before = fixture.snapshot();

        assert!(matches!(
            workspace(&fixture).track("other", "develop"),
            Err(Error::Unrelated { .. })
        ));
        fixture.assert_unchanged(&before);
    }

    #[test]
    fn invalid_names_are_rejected_before_lookup() {
        let fixture = tracked();
        let before = fixture.snapshot();

        for name in ["../escape", "a..b", "trailing/", "with space", "lock.lock"] {
            let result = workspace(&fixture).track(name, "develop");
            assert!(
                matches!(result, Err(Error::InvalidRefName { .. })),
                "{name}: {result:?}"
            );
        }
        fixture.assert_unchanged(&before);
    }

    #[test]
    fn untrack_refuses_while_branches_are_stacked_on_it() {
        let fixture = tracked();
        let before = fixture.snapshot();

        assert!(matches!(
            workspace(&fixture).untrack("feat/a"),
            Err(Error::HasChildren { .. })
        ));
        fixture.assert_unchanged(&before);
    }

    #[test]
    fn untrack_deletes_only_the_branch_ref() {
        let fixture = tracked();
        let before = fixture.snapshot();

        assert_eq!(
            workspace(&fixture).untrack("feat/b").unwrap(),
            Outcome::Changed
        );

        let diff = before.diff(&fixture.snapshot());
        assert_eq!(
            diff.refs.keys().collect::<Vec<_>>(),
            ["refs/stack/branches/feat/b"]
        );
        assert_eq!(diff.refs["refs/stack/branches/feat/b"].1, None);
        assert_eq!(
            diff,
            SnapshotDiff {
                refs: diff.refs.clone(),
                ..SnapshotDiff::default()
            }
        );
    }

    #[test]
    fn untrack_works_after_branch_is_deleted() {
        let fixture = tracked();
        fixture.git(&["branch", "--quiet", "-D", "feat/b"]);

        assert_eq!(
            workspace(&fixture).untrack("feat/b").unwrap(),
            Outcome::Changed
        );
    }

    #[test]
    fn untrack_rejects_untracked() {
        let fixture = stacked();
        assert!(matches!(
            workspace(&fixture).untrack("feat/a"),
            Err(Error::NotTracked { .. })
        ));
    }
}

mod tree {
    use super::*;

    #[test]
    fn nests_branches_under_trunks() {
        let fixture = tracked();
        fixture.git(&["switch", "--quiet", "feat/a"]);
        let before = fixture.snapshot();

        let tree = workspace(&fixture).tree().unwrap();

        let leaf = |name: &str| Node {
            name: name.into(),
            commit: Some(tip(&fixture, name)),
            current: false,
            children: vec![],
        };
        let a = Node {
            current: true,
            children: vec![leaf("feat/b")],
            ..leaf("feat/a")
        };
        let develop = Node {
            children: vec![a],
            ..leaf("develop")
        };
        assert_eq!(
            tree,
            Tree {
                trunks: vec![develop],
                orphans: vec![]
            }
        );
        fixture.assert_unchanged(&before);
    }

    #[test]
    fn empty_without_metadata() {
        let fixture = stacked();
        assert_eq!(
            workspace(&fixture).tree().unwrap(),
            Tree {
                trunks: vec![],
                orphans: vec![]
            }
        );
    }

    #[test]
    fn deleted_branch_has_no_commit() {
        let fixture = tracked();
        fixture.git(&["branch", "--quiet", "-D", "feat/b"]);

        let tree = workspace(&fixture).tree().unwrap();
        assert_eq!(tree.trunks[0].children[0].children[0].commit, None);
    }

    #[test]
    fn branches_with_missing_parent_become_orphans_with_their_children() {
        let fixture = tracked();
        fixture.git(&["switch", "--quiet", "--create", "feat/c", "feat/b"]);
        workspace(&fixture).track("feat/c", "feat/b").unwrap();
        fixture.git(&["update-ref", "-d", "refs/stack/branches/feat/a"]);

        let tree = workspace(&fixture).tree().unwrap();

        assert!(tree.trunks[0].children.is_empty());
        assert_eq!(names(&tree.orphans), ["feat/b"]);
        assert_eq!(names(&tree.orphans[0].children), ["feat/c"]);
    }

    #[test]
    fn corrupt_metadata_is_reported() {
        let fixture = tracked();
        let garbage = fixture.git_stdin(&["hash-object", "-w", "--stdin"], b"not json");
        fixture.git(&["update-ref", "refs/stack/branches/feat/a", &garbage]);
        assert!(matches!(
            workspace(&fixture).tree(),
            Err(Error::CorruptMetadata { .. })
        ));

        fixture.git(&["update-ref", "refs/stack/branches/feat/a", "develop"]);
        assert!(matches!(
            workspace(&fixture).tree(),
            Err(Error::CorruptMetadata { .. })
        ));
    }
}

mod storage {
    use super::*;

    #[test]
    fn metadata_is_not_pushed_or_cloned_by_default() {
        let fixture = tracked();
        let remote = fixture.add_bare_remote("origin");
        fixture.git(&["push", "--quiet", "origin", "--all"]);
        fixture.git(&["push", "--quiet", "origin", "--tags"]);
        let clone = fixture.scratch_path("clone");
        fixture.git(&[
            "clone",
            "--quiet",
            fixture.path().to_str().unwrap(),
            clone.to_str().unwrap(),
        ]);

        for repo in [&remote, &clone] {
            let refs = fixture.git_in(repo, &["for-each-ref", "--format=%(refname)"]);
            assert!(!refs.contains("refs/stack/"), "{}:\n{refs}", repo.display());
        }
    }

    #[test]
    fn metadata_is_invisible_to_log_all() {
        let fixture = stacked();
        let log_before = fixture.git(&["log", "--all", "--oneline"]);
        let workspace = workspace(&fixture);
        workspace.add_trunk("develop").unwrap();
        workspace.track("feat/a", "develop").unwrap();

        assert_eq!(fixture.git(&["log", "--all", "--oneline"]), log_before);
    }

    #[test]
    fn metadata_survives_gc_and_packed_refs() {
        let fixture = tracked();
        let tree = workspace(&fixture).tree().unwrap();
        fixture.git(&["gc", "--quiet", "--prune=now"]);
        fixture.git(&["pack-refs", "--all"]);

        assert_eq!(workspace(&fixture).tree().unwrap(), tree);
        // Compare-and-swap against a packed ref.
        assert_eq!(
            workspace(&fixture).untrack("feat/b").unwrap(),
            Outcome::Changed
        );
        assert_eq!(
            fixture.git(&["for-each-ref", "refs/stack/branches/feat/b"]),
            ""
        );
        fixture.git(&["fsck", "--no-progress"]);
    }
}
