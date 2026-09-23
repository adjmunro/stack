//! Parents derived from the commit graph and reconciled with recorded parents.

mod common;

use common::*;
use stack_core::{Role, Source};
use stack_testkit::Fixture;

mod derivation {
    use super::*;

    #[test]
    fn nests_a_linear_stack_without_writing() {
        let fixture = repo();
        grow(&fixture, "a", "develop");
        grow(&fixture, "b", "a");
        let before = fixture.snapshot();

        let tree = tree(&fixture);

        assert_eq!(shape(&tree), "develop(a(b))");
        let b = parent(&tree, "b").unwrap();
        assert_eq!(
            (b.source, b.offshoot, b.needs_restack),
            (Source::Derived, tip(&fixture, "a"), false)
        );
        fixture.assert_unchanged(&before);
    }

    #[test]
    fn siblings_share_a_parent() {
        let fixture = repo();
        grow(&fixture, "a", "develop");
        grow(&fixture, "b", "a");
        grow(&fixture, "c", "a");
        grow(&fixture, "d", "develop");

        assert_eq!(shape(&tree(&fixture)), "develop(a(b c) d)");
    }

    #[test]
    fn advanced_trunk_leaves_branch_needing_restack() {
        let fixture = repo();
        let fork = tip(&fixture, "develop");
        grow(&fixture, "a", "develop");
        fixture.git(&["switch", "--quiet", "develop"]);
        fixture.commit("later.txt", "later", "feat: later");

        let a = parent(&tree(&fixture), "a").unwrap();
        assert_eq!(
            (a.name.as_str(), a.offshoot, a.needs_restack),
            ("develop", fork, true)
        );
    }

    #[test]
    fn branches_merged_into_a_trunk_never_become_parents() {
        let fixture = repo();
        grow(&fixture, "old", "develop");
        fixture.git(&["switch", "--quiet", "develop"]);
        fixture.git(&["merge", "--quiet", "--ff-only", "old"]);
        grow(&fixture, "new", "old");

        assert_eq!(shape(&tree(&fixture)), "develop(new old)");
    }

    #[test]
    fn newer_branch_on_the_same_commit_is_the_child() {
        let fixture = repo();
        grow(&fixture, "older", "develop");
        fixture.git(&["branch", "newer", "older"]);

        assert_eq!(shape(&tree(&fixture)), "develop(older(newer))");
    }

    #[test]
    fn child_of_same_commit_branches_picks_the_oldest() {
        let fixture = repo();
        grow(&fixture, "older", "develop");
        fixture.git(&["branch", "newer", "older"]);
        grow(&fixture, "child", "newer");

        assert_eq!(shape(&tree(&fixture)), "develop(older(child newer))");
    }

    #[test]
    fn parallel_copies_with_identical_patches_never_steal_a_child() {
        let fixture = repo();
        grow(&fixture, "a", "develop");
        grow(&fixture, "c", "a");
        fixture.git(&["switch", "--quiet", "--create", "copy", "develop"]);
        fixture.git(&["cherry-pick", "a"]);
        record(&fixture, "c", "a", &tip(&fixture, "a"), false);

        assert_eq!(shape(&tree(&fixture)), "develop(a(c) copy)");
        fixture.git(&["update-ref", "-d", "refs/stack/branches/c"]);
        assert_eq!(shape(&tree(&fixture)), "develop(a(c) copy)");
    }

    #[test]
    fn unrelated_history_is_unattached() {
        let fixture = repo();
        fixture.git(&["switch", "--quiet", "--orphan", "pages"]);
        fixture.commit("index.html", "hi", "feat: pages");

        assert_eq!(shape(&tree(&fixture)), "develop | ~pages");
    }

    #[test]
    fn without_trunks_every_branch_is_unattached() {
        let fixture = Fixture::new();
        fixture.commit("base.txt", "base", "feat: base");
        grow(&fixture, "a", "develop");

        assert_eq!(shape(&tree(&fixture)), "~a | ~develop");
    }

    #[test]
    fn branch_on_a_release_trunk_attaches_to_it_not_develop() {
        let fixture = repo();
        grow(&fixture, "release", "develop");
        workspace(&fixture).add_trunk("release").unwrap();
        grow(&fixture, "fix", "release");
        fixture.git(&["switch", "--quiet", "develop"]);
        fixture.commit("later.txt", "later", "feat: later");
        grow(&fixture, "feature", "develop");

        assert_eq!(shape(&tree(&fixture)), "develop(feature) | release(fix)");
    }
}

mod reconciliation {
    use super::*;

    #[test]
    fn recorded_parent_survives_its_parent_being_amended() {
        let fixture = repo();
        let old_a = grow(&fixture, "a", "develop");
        grow(&fixture, "b", "a");
        record(&fixture, "b", "a", &old_a, false);
        fixture.git(&["switch", "--quiet", "a"]);
        fixture.write("a.txt", "amended");
        fixture.git(&[
            "commit",
            "--quiet",
            "--all",
            "--amend",
            "--message",
            "feat: a (amended)",
        ]);
        let before = fixture.snapshot();

        let tree = tree(&fixture);

        assert_eq!(shape(&tree), "develop(a(b))");
        let b = parent(&tree, "b").unwrap();
        assert_eq!(
            (b.source, b.offshoot, b.needs_restack),
            (Source::Recorded, old_a, true)
        );
        fixture.assert_unchanged(&before);
    }

    /// Known gap: nothing is recorded until `stack` performs an operation, so an unrecorded link is lost when the
    /// parent is rewritten. Observed tips (SQLite) will close this.
    #[test]
    fn unrecorded_parent_rewritten_falls_back_to_trunk() {
        let fixture = repo();
        grow(&fixture, "a", "develop");
        grow(&fixture, "b", "a");
        fixture.git(&["switch", "--quiet", "a"]);
        fixture.git(&[
            "commit",
            "--quiet",
            "--amend",
            "--message",
            "feat: a (amended)",
        ]);

        assert_eq!(shape(&tree(&fixture)), "develop(a b)");
    }

    #[test]
    fn recorded_parent_survives_its_parent_advancing() {
        let fixture = repo();
        let old_a = grow(&fixture, "a", "develop");
        grow(&fixture, "b", "a");
        record(&fixture, "b", "a", &old_a, false);
        fixture.git(&["switch", "--quiet", "a"]);
        fixture.commit("a2.txt", "a2", "feat: a2");

        let b = parent(&tree(&fixture), "b").unwrap();
        assert_eq!(
            (b.name.as_str(), b.offshoot, b.needs_restack),
            ("a", old_a, true)
        );
    }

    #[test]
    fn branch_moved_elsewhere_with_git_is_rederived_and_reported() {
        let fixture = repo();
        let a = grow(&fixture, "a", "develop");
        grow(&fixture, "c", "develop");
        grow(&fixture, "b", "a");
        record(&fixture, "b", "a", &a, false);
        fixture.git(&["rebase", "--quiet", "--onto", "c", "a", "b"]);

        let tree = tree(&fixture);

        assert_eq!(shape(&tree), "develop(a c(b))");
        let b = parent(&tree, "b").unwrap();
        assert_eq!(
            (b.source, b.replaces.as_deref()),
            (Source::Derived, Some("a"))
        );
    }

    #[test]
    fn branch_moved_onto_a_child_of_its_recorded_parent_follows_the_graph() {
        let fixture = repo();
        let a = grow(&fixture, "a", "develop");
        grow(&fixture, "b", "a");
        grow(&fixture, "x", "a");
        record(&fixture, "b", "a", &a, false);
        fixture.git(&["rebase", "--quiet", "--onto", "x", "a", "b"]);

        let tree = tree(&fixture);
        assert_eq!(shape(&tree), "develop(a(x(b)))");
        assert_eq!(parent(&tree, "b").unwrap().replaces.as_deref(), Some("a"));
    }

    #[test]
    fn deleted_recorded_parent_is_replaced() {
        let fixture = repo();
        let a = grow(&fixture, "a", "develop");
        grow(&fixture, "b", "a");
        record(&fixture, "b", "a", &a, false);
        fixture.git(&["branch", "--quiet", "-D", "a"]);

        let tree = tree(&fixture);
        assert_eq!(shape(&tree), "develop(b)");
        assert_eq!(parent(&tree, "b").unwrap().replaces.as_deref(), Some("a"));
    }

    #[test]
    fn pinned_parent_holds_when_contradicted() {
        let fixture = repo();
        grow(&fixture, "a", "develop");
        grow(&fixture, "c", "develop");
        grow(&fixture, "b", "a");
        workspace(&fixture).pin("b", None).unwrap();
        fixture.git(&["rebase", "--quiet", "--onto", "c", "a", "b"]);

        let tree = tree(&fixture);
        assert_eq!(shape(&tree), "develop(a(b) c)");
        let b = parent(&tree, "b").unwrap();
        assert_eq!((b.source, b.contradicted), (Source::Pinned, true));
    }

    #[test]
    fn recorded_cycles_fall_back_to_the_graph() {
        let fixture = repo();
        let a = grow(&fixture, "a", "develop");
        fixture.git(&["branch", "b", "a"]);
        record(&fixture, "a", "b", &a, false);
        record(&fixture, "b", "a", &a, false);

        assert_eq!(shape(&tree(&fixture)), "develop(a(b))");
    }
}

mod trunks {
    use super::*;

    #[test]
    fn add_writes_exactly_one_blob_ref() {
        let fixture = Fixture::new();
        fixture.commit("base.txt", "base", "feat: base");
        let before = fixture.snapshot();

        let marked = workspace(&fixture).add_trunk("develop").unwrap();

        assert_eq!((marked.role, marked.parent), (Role::Trunk, None));
        let diff = before.diff(&fixture.snapshot());
        assert_eq!(
            diff.refs.keys().collect::<Vec<_>>(),
            ["refs/stack/trunks/develop"]
        );
        assert_eq!(diff.objects_added.len(), 1);
        assert!(
            diff.head.is_none()
                && diff.index.is_empty()
                && diff.worktree.is_empty()
                && !diff.config
        );
        assert_eq!(
            fixture.git(&["cat-file", "-p", "refs/stack/trunks/develop"]),
            r#"{"version":1,"role":"trunk"}"#
        );
    }

    #[test]
    fn adding_twice_is_unchanged() {
        let fixture = repo();
        let before = fixture.snapshot();

        assert_eq!(
            workspace(&fixture).add_trunk("develop").unwrap().outcome,
            stack_core::Outcome::Unchanged
        );
        fixture.assert_unchanged(&before);
    }

    #[test]
    fn checkpoint_on_a_stack_branch_becomes_a_limb() {
        let fixture = repo();
        grow(&fixture, "a", "develop");
        grow(&fixture, "rel", "a");
        grow(&fixture, "d", "rel");

        let marked = workspace(&fixture).add_trunk("rel").unwrap();

        assert_eq!(
            (marked.role, marked.parent.as_deref()),
            (Role::Limb, Some("a"))
        );
        assert_eq!(shape(&tree(&fixture)), "develop(a(rel(d)))");
    }

    #[test]
    fn branch_directly_on_a_trunk_becomes_a_trunk() {
        let fixture = repo();
        grow(&fixture, "release", "develop");

        let marked = workspace(&fixture).add_trunk("release").unwrap();

        assert_eq!((marked.role, marked.parent), (Role::Trunk, None));
        assert_eq!(shape(&tree(&fixture)), "develop | release");
    }

    #[test]
    fn remove_deletes_only_the_mark_and_branches_reattach() {
        let fixture = repo();
        grow(&fixture, "release", "develop");
        workspace(&fixture).add_trunk("release").unwrap();
        grow(&fixture, "fix", "release");
        let before = fixture.snapshot();

        assert_eq!(
            workspace(&fixture).remove_trunk("release").unwrap(),
            Role::Trunk
        );

        let diff = before.diff(&fixture.snapshot());
        assert_eq!(
            diff.refs.keys().collect::<Vec<_>>(),
            ["refs/stack/trunks/release"]
        );
        assert!(diff.objects_added.is_empty() && diff.objects_removed.is_empty());
        assert_eq!(shape(&tree(&fixture)), "develop(release(fix))");
    }

    #[test]
    fn remove_rejects_unmarked() {
        let fixture = repo();
        grow(&fixture, "a", "develop");
        assert!(matches!(
            workspace(&fixture).remove_trunk("a"),
            Err(stack_core::Error::NotTrunk { .. })
        ));
    }
}
