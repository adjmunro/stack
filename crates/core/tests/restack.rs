//! Restack: rebasing branches onto their parents' current tips.

mod common;

use std::collections::BTreeSet;

use common::*;
use stack_core::{Error, Outcome};
use stack_testkit::{Fixture, SnapshotDiff};

fn subjects(fixture: &Fixture, range: &str) -> Vec<String> {
    fixture
        .git(&["log", "--format=%s", range])
        .lines()
        .map(str::to_owned)
        .collect()
}

fn moved_names(restacked: &stack_core::Restacked) -> Vec<&str> {
    restacked
        .moved
        .iter()
        .map(|moved| moved.name.as_str())
        .collect()
}

/// `develop` ← `a` ← `b`, then `develop` advances. HEAD on `develop`.
fn advanced() -> Fixture {
    let fixture = repo();
    grow(&fixture, "a", "develop");
    grow(&fixture, "b", "a");
    fixture.git(&["switch", "--quiet", "develop"]);
    fixture.commit("later.txt", "later", "feat: later");
    fixture
}

fn refs_only(diff: &SnapshotDiff) -> bool {
    diff.head.is_none() && diff.index.is_empty() && diff.worktree.is_empty() && !diff.config
}

#[test]
fn moves_the_stack_onto_the_advanced_trunk() {
    let fixture = advanced();
    let author = fixture.git(&["log", "-1", "--format=%an <%ae> %ad", "b"]);
    let before = fixture.snapshot();

    let restacked = workspace(&fixture).restack("develop").unwrap();

    assert_eq!(restacked.outcome, Outcome::Changed);
    assert_eq!(moved_names(&restacked), ["a", "b"]);
    assert!(
        restacked
            .moved
            .iter()
            .all(|moved| (moved.replayed, moved.dropped) == (1, 0))
    );
    assert!(restacked.conflicts.is_empty() && restacked.blocked.is_empty());
    assert_eq!(subjects(&fixture, "develop..b"), ["feat: b", "feat: a"]);
    assert_eq!(
        fixture.git(&["merge-base", "a", "develop"]),
        tip(&fixture, "develop")
    );
    assert_eq!(
        fixture.git(&["log", "-1", "--format=%an <%ae> %ad", "b"]),
        author
    );
    assert_eq!(fixture.git(&["show", "b:later.txt"]), "later");

    let diff = before.diff(&fixture.snapshot());
    assert!(refs_only(&diff), "{diff}");
    let refs: Vec<&str> = diff.refs.keys().map(String::as_str).collect();
    assert_eq!(
        refs,
        [
            "refs/heads/a",
            "refs/heads/b",
            "refs/stack/branches/a",
            "refs/stack/branches/b"
        ]
    );
    let tree = tree(&fixture);
    assert_eq!(shape(&tree), "develop(a(b))");
    assert!(!parent(&tree, "b").unwrap().needs_restack);
}

#[test]
fn produces_the_same_trees_and_messages_as_git_rebase() {
    let ours = advanced();
    let theirs = advanced();
    let fork = theirs.git(&["merge-base", "a", "b"]);
    let base = theirs.git(&["merge-base", "a", "develop"]);

    workspace(&ours).restack("develop").unwrap();
    theirs.git(&["rebase", "--quiet", "--onto", "develop", &base, "a"]);
    theirs.git(&["rebase", "--quiet", "--onto", "a", &fork, "b"]);

    for branch in ["a", "b"] {
        let tree = format!("{branch}^{{tree}}");
        assert_eq!(
            ours.git(&["rev-parse", &tree]),
            theirs.git(&["rev-parse", &tree]),
            "{branch}"
        );
        assert_eq!(
            subjects(&ours, &format!("develop..{branch}")),
            subjects(&theirs, &format!("develop..{branch}"))
        );
    }
}

#[test]
fn amended_parent_is_not_duplicated_into_the_child() {
    let fixture = repo();
    grow(&fixture, "a", "develop");
    grow(&fixture, "b", "a");
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

    let restacked = workspace(&fixture).restack("a").unwrap();

    assert_eq!(moved_names(&restacked), ["b"]);
    assert_eq!(
        subjects(&fixture, "develop..b"),
        ["feat: b", "feat: a (amended)"]
    );
    assert_eq!(fixture.git(&["show", "b:a.txt"]), "amended");
}

#[test]
fn only_the_target_and_its_descendants_move() {
    let fixture = repo();
    grow(&fixture, "a", "develop");
    grow(&fixture, "b", "a");
    grow(&fixture, "c", "develop");
    fixture.git(&["switch", "--quiet", "develop"]);
    fixture.commit("later.txt", "later", "feat: later");
    let c = tip(&fixture, "c");

    let restacked = workspace(&fixture).restack("a").unwrap();

    assert_eq!(moved_names(&restacked), ["a", "b"]);
    assert_eq!(tip(&fixture, "c"), c);
}

#[test]
fn restack_passes_through_limbs() {
    let fixture = repo();
    grow(&fixture, "a", "develop");
    grow(&fixture, "rel", "a");
    workspace(&fixture).add_trunk("rel").unwrap();
    grow(&fixture, "d", "rel");
    fixture.git(&["switch", "--quiet", "develop"]);
    fixture.commit("later.txt", "later", "feat: later");

    let restacked = workspace(&fixture).restack("develop").unwrap();

    assert_eq!(moved_names(&restacked), ["a", "rel", "d"]);
    assert_eq!(
        subjects(&fixture, "develop..d"),
        ["feat: d", "feat: rel", "feat: a"]
    );
}

#[test]
fn checked_out_branch_moves_with_its_working_tree() {
    let fixture = advanced();
    fixture.git(&["switch", "--quiet", "b"]);

    workspace(&fixture).restack("develop").unwrap();

    assert_eq!(fixture.git(&["symbolic-ref", "HEAD"]), "refs/heads/b");
    assert_eq!(fixture.git(&["status", "--porcelain"]), "");
    assert_eq!(
        std::fs::read_to_string(fixture.path().join("later.txt")).unwrap(),
        "later"
    );
}

#[test]
fn moving_the_checked_out_branch_is_logged_in_head_and_branch_reflogs() {
    let fixture = advanced();
    fixture.git(&["switch", "--quiet", "b"]);

    workspace(&fixture).restack("develop").unwrap();

    let new_tip = tip(&fixture, "b");
    for reference in ["HEAD", "b"] {
        let entry = fixture.git(&["reflog", "-1", "--format=%H %gs", reference]);
        assert_eq!(
            entry,
            format!("{new_tip} stack: restack develop"),
            "{reference}"
        );
    }
    assert_eq!(
        fixture.git(&["reflog", "-1", "--format=%gs", "a"]),
        "stack: restack develop"
    );
}

#[test]
fn uncommitted_changes_on_a_moving_branch_are_refused() {
    let fixture = advanced();
    fixture.git(&["switch", "--quiet", "b"]);
    fixture.write("b.txt", "work in progress");
    let before = fixture.snapshot();

    let result = workspace(&fixture).restack("develop");

    assert!(
        matches!(result, Err(Error::DirtyWorktree { ref branch }) if branch == "b"),
        "{result:?}"
    );
    let diff = before.diff(&fixture.snapshot());
    // Only unreferenced objects from planning.
    assert!(
        refs_only(&diff) && diff.refs.is_empty() && diff.stack.is_empty(),
        "{diff}"
    );
}

#[test]
fn branch_checked_out_in_another_worktree_is_refused() {
    let fixture = advanced();
    let other = fixture.scratch_path("other");
    fixture.git(&["worktree", "add", "--quiet", other.to_str().unwrap(), "b"]);
    let before = fixture.snapshot();

    let result = workspace(&fixture).restack("develop");

    assert!(
        matches!(result, Err(Error::CheckedOutElsewhere { ref branch }) if branch == "b"),
        "{result:?}"
    );
    assert!(before.diff(&fixture.snapshot()).refs.is_empty());
}

#[test]
fn conflict_skips_that_branch_and_its_descendants_only() {
    let fixture = repo();
    grow(&fixture, "c", "develop");
    let old_develop = tip(&fixture, "develop");
    fixture.git(&["switch", "--quiet", "--create", "a", "develop"]);
    fixture.commit("base.txt", "from a", "feat: a");
    grow(&fixture, "b", "a");
    fixture.git(&["switch", "--quiet", "develop"]);
    fixture.commit("base.txt", "from develop", "feat: later");
    let (a, b) = (tip(&fixture, "a"), tip(&fixture, "b"));

    let restacked = workspace(&fixture).restack("develop").unwrap();

    assert_eq!(moved_names(&restacked), ["c"]);
    assert_eq!(restacked.blocked, ["b"]);
    let [conflict] = restacked.conflicts.as_slice() else {
        panic!("{:?}", restacked.conflicts)
    };
    let conflict = conflict.clone();
    assert_eq!(
        (conflict.branch.as_str(), conflict.onto.as_str()),
        ("a", "develop")
    );
    assert_eq!(
        (conflict.summary.as_str(), conflict.paths.as_slice()),
        ("feat: a", ["base.txt".to_owned()].as_slice())
    );
    assert_eq!(
        (conflict.commit, conflict.offshoot),
        (a.clone(), old_develop)
    );
    assert_eq!((tip(&fixture, "a"), tip(&fixture, "b")), (a, b));
}

#[test]
fn commits_already_in_the_parent_are_dropped() {
    let fixture = repo();
    grow(&fixture, "b", "develop");
    fixture.git(&["switch", "--quiet", "develop"]);
    fixture.git(&["cherry-pick", "b"]);

    let restacked = workspace(&fixture).restack("develop").unwrap();

    assert_eq!(
        (restacked.moved[0].replayed, restacked.moved[0].dropped),
        (0, 1)
    );
    assert_eq!(tip(&fixture, "b"), tip(&fixture, "develop"));
}

#[test]
fn merge_commits_are_refused_without_changes() {
    let fixture = repo();
    grow(&fixture, "a", "develop");
    grow(&fixture, "side", "a");
    fixture.git(&["switch", "--quiet", "a"]);
    fixture.commit("a2.txt", "a2", "feat: a2");
    fixture.git(&["merge", "--quiet", "--no-ff", "--no-edit", "side"]);
    fixture.git(&["branch", "--quiet", "-D", "side"]);
    fixture.git(&["switch", "--quiet", "develop"]);
    fixture.commit("later.txt", "later", "feat: later");
    let before = fixture.snapshot();

    assert!(matches!(
        workspace(&fixture).restack("develop"),
        Err(Error::MergeCommit { .. })
    ));
    assert!(before.diff(&fixture.snapshot()).refs.is_empty());
}

#[test]
fn restacking_again_changes_nothing() {
    let fixture = advanced();
    workspace(&fixture).restack("develop").unwrap();
    let before = fixture.snapshot();

    let restacked = workspace(&fixture).restack("develop").unwrap();

    assert_eq!(
        (restacked.outcome, restacked.moved.len()),
        (Outcome::Unchanged, 0)
    );
    fixture.assert_unchanged(&before);
}

#[test]
fn undo_puts_every_branch_back() {
    let fixture = advanced();
    fixture.git(&["switch", "--quiet", "b"]);
    let before = fixture.snapshot();
    workspace(&fixture).restack("develop").unwrap();

    let undone = workspace(&fixture).undo().unwrap();

    assert_eq!(undone.description, "restack develop");
    let diff = before.diff(&fixture.snapshot());
    assert_eq!(
        diff.refs.keys().collect::<BTreeSet<_>>(),
        BTreeSet::from([&"refs/stack/keep".to_owned()])
    );
    assert!(refs_only(&diff), "{diff}");
    assert_eq!(fixture.git(&["status", "--porcelain"]), "");
}

#[test]
fn records_of_deleted_branches_are_pruned_and_undo_restores_them() {
    let fixture = advanced();
    grow(&fixture, "gone", "develop");
    workspace(&fixture).pin("gone", None).unwrap();
    fixture.git(&["switch", "--quiet", "develop"]);
    fixture.git(&["branch", "--quiet", "-D", "gone"]);

    workspace(&fixture).restack("develop").unwrap();
    assert_eq!(
        fixture.git(&["for-each-ref", "refs/stack/branches/gone"]),
        ""
    );

    workspace(&fixture).undo().unwrap();
    assert_ne!(
        fixture.git(&["for-each-ref", "refs/stack/branches/gone"]),
        ""
    );
}

#[test]
fn unknown_branch_is_an_error() {
    let fixture = repo();
    assert!(matches!(
        workspace(&fixture).restack("nope"),
        Err(Error::UnknownBranch { .. })
    ));
}

#[test]
fn restacked_commits_are_signed_when_the_user_signs_commits() {
    let fixture = advanced();
    let key = fixture.scratch_path("key");
    let generated = std::process::Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-C", "fixture", "-f"])
        .arg(&key)
        .status()
        .expect("ssh-keygen is installed");
    assert!(generated.success());
    fixture.git(&["config", "gpg.format", "ssh"]);
    fixture.git(&["config", "user.signingkey", key.to_str().unwrap()]);
    fixture.git(&["config", "commit.gpgsign", "true"]);

    workspace(&fixture).restack("develop").unwrap();

    for branch in ["a", "b"] {
        let object = fixture.git(&["cat-file", "-p", branch]);
        assert!(
            object.contains("\ngpgsig -----BEGIN SSH SIGNATURE-----"),
            "{branch} unsigned:\n{object}"
        );
    }
}

#[test]
fn restacked_commits_are_unsigned_when_the_user_does_not_sign() {
    let fixture = advanced();

    workspace(&fixture).restack("develop").unwrap();

    assert!(!fixture.git(&["cat-file", "-p", "b"]).contains("gpgsig"));
}

#[test]
fn reflog_entries_use_the_sealed_environment_s_identity() {
    let fixture = advanced();

    workspace(&fixture).restack("develop").unwrap();

    let identity = fixture.git(&["reflog", "-1", "--format=%gn <%ge>", "b"]);
    assert_eq!(identity, "Fixture Committer <committer@fixture.invalid>");
}
