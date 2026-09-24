//! Worktrees: hidden sibling directories, followers, and landing a follower's commits.

mod common;

use std::fs;
use std::path::PathBuf;

use common::*;
use stack_core::{Error, FollowPosition, SyncOutcome};
use stack_testkit::Fixture;

/// `develop` (trunk) ← `a`, with `a` checked out in the main worktree.
fn repo_on_a() -> Fixture {
    let fixture = repo();
    grow(&fixture, "a", "develop");
    fixture
}

/// Adds a follower of `a` and returns its path.
fn follower(fixture: &Fixture) -> PathBuf {
    let worktree = workspace(fixture).add_worktree("a", None).unwrap();
    assert!(worktree.follows.is_some(), "{worktree:?}");
    worktree.path
}

fn position(fixture: &Fixture, path: &std::path::Path) -> FollowPosition {
    let worktrees = workspace(fixture).worktrees().unwrap();
    let canonical = path.canonicalize().unwrap();
    let worktree = worktrees
        .iter()
        .find(|worktree| worktree.path.canonicalize().unwrap() == canonical)
        .unwrap();
    worktree.follows.as_ref().unwrap().position
}

fn read(path: &std::path::Path, file: &str) -> String {
    fs::read_to_string(path.join(file)).unwrap()
}

mod adding {
    use super::*;

    #[test]
    fn a_free_branch_gets_a_hidden_sibling_worktree() {
        let fixture = repo_on_a();
        fixture.git(&["branch", "feat/free", "develop"]);

        let worktree = workspace(&fixture).add_worktree("feat/free", None).unwrap();

        assert_eq!(worktree.path.file_name().unwrap(), ".repo-feat-free");
        assert_eq!(
            worktree.path.parent().unwrap().canonicalize().unwrap(),
            fixture.path().parent().unwrap().canonicalize().unwrap()
        );
        assert_eq!(
            (worktree.branch.as_deref(), worktree.follows),
            (Some("feat/free"), None)
        );
    }

    #[test]
    fn a_checked_out_branch_gets_a_follower() {
        let fixture = repo_on_a();

        let path = follower(&fixture);

        assert_eq!(
            fixture.git_in(&path, &["rev-parse", "HEAD"]),
            tip(&fixture, "a")
        );
        assert_eq!(
            fixture.git_in(&path, &["rev-parse", "--abbrev-ref", "HEAD"]),
            "HEAD",
            "detached"
        );
        assert_eq!(position(&fixture, &path), FollowPosition::UpToDate);
    }

    #[test]
    fn existing_paths_and_unknown_branches_are_refused() {
        let fixture = repo_on_a();
        let taken = fixture.scratch_path("taken");
        fs::create_dir(&taken).unwrap();

        assert!(matches!(
            workspace(&fixture).add_worktree("a", Some(&taken)),
            Err(Error::PathExists { .. })
        ));
        assert!(matches!(
            workspace(&fixture).add_worktree("nope", None),
            Err(Error::UnknownBranch { .. })
        ));
    }
}

mod following {
    use super::*;

    #[test]
    fn a_follower_catches_up_with_commits_made_in_the_main_worktree() {
        let fixture = repo_on_a();
        let path = follower(&fixture);
        fixture.commit("more.txt", "more", "feat: more");
        assert_eq!(position(&fixture, &path), FollowPosition::Behind);

        let synced = workspace(&fixture).sync_followers().unwrap();

        assert!(
            matches!(synced[0].outcome, SyncOutcome::Moved { .. }),
            "{synced:?}"
        );
        assert_eq!(read(&path, "more.txt"), "more");
        assert_eq!(fixture.git_in(&path, &["status", "--porcelain"]), "");
        assert_eq!(position(&fixture, &path), FollowPosition::UpToDate);
    }

    #[test]
    fn a_follower_follows_a_restack() {
        let fixture = repo_on_a();
        let path = follower(&fixture);
        fixture.git(&["switch", "--quiet", "develop"]);
        fixture.commit("later.txt", "later", "feat: later");
        workspace(&fixture).restack("develop").unwrap();

        workspace(&fixture).sync_followers().unwrap();

        assert_eq!(
            fixture.git_in(&path, &["rev-parse", "HEAD"]),
            tip(&fixture, "a")
        );
        assert_eq!(read(&path, "later.txt"), "later");
    }

    #[test]
    fn a_follower_with_its_own_commits_is_left_alone() {
        let fixture = repo_on_a();
        let path = follower(&fixture);
        fixture.git_in(
            &path,
            &[
                "commit",
                "--quiet",
                "--allow-empty",
                "--message",
                "feat: mine",
            ],
        );

        let synced = workspace(&fixture).sync_followers().unwrap();

        assert_eq!(
            synced[0].outcome,
            SyncOutcome::Skipped {
                reason: "has commits to land".into()
            }
        );
        assert_eq!(position(&fixture, &path), FollowPosition::Ahead);
    }

    #[test]
    fn a_worktree_that_checks_out_a_branch_stops_following() {
        let fixture = repo_on_a();
        let path = follower(&fixture);
        fixture.git_in(&path, &["switch", "--quiet", "develop"]);

        assert!(workspace(&fixture).sync_followers().unwrap().is_empty());
        assert!(
            workspace(&fixture)
                .worktrees()
                .unwrap()
                .iter()
                .all(|worktree| worktree.follows.is_none())
        );
    }
}

mod landing {
    use super::*;

    #[test]
    fn landing_moves_the_branch_and_the_main_worktree_with_it() {
        let fixture = repo_on_a();
        let path = follower(&fixture);
        fs::write(path.join("landed.txt"), "landed").unwrap();
        fixture.git_in(&path, &["add", "landed.txt"]);
        fixture.git_in(&path, &["commit", "--quiet", "--message", "feat: landed"]);
        // Work in progress in the main worktree that the landed commit doesn't touch.
        fixture.write("a.txt", "in progress");
        let head = fixture.git_in(&path, &["rev-parse", "HEAD"]);

        let landed = workspace_at(&fixture, &path).land(None).unwrap();

        assert_eq!(
            (landed.branch.as_str(), landed.commits, landed.new.as_str()),
            ("a", 1, head.as_str())
        );
        assert_eq!(tip(&fixture, "a"), head);
        assert_eq!(fixture.git(&["symbolic-ref", "--short", "HEAD"]), "a");
        assert_eq!(read(&fixture.path(), "landed.txt"), "landed");
        assert_eq!(read(&fixture.path(), "a.txt"), "in progress");
        assert_eq!(fixture.git(&["status", "--porcelain"]), "M a.txt");
        assert_eq!(position(&fixture, &path), FollowPosition::UpToDate);
    }

    #[test]
    fn landing_is_refused_if_the_branch_moved_on_meanwhile() {
        let fixture = repo_on_a();
        let path = follower(&fixture);
        fixture.git_in(
            &path,
            &[
                "commit",
                "--quiet",
                "--allow-empty",
                "--message",
                "feat: mine",
            ],
        );
        fixture.commit("theirs.txt", "theirs", "feat: theirs");
        let before = fixture.snapshot();

        let result = workspace_at(&fixture, &path).land(None);

        assert!(
            matches!(result, Err(Error::NotFastForward { ref branch }) if branch == "a"),
            "{result:?}"
        );
        fixture.assert_unchanged(&before);
    }

    #[test]
    fn landing_is_refused_if_it_would_clash_with_the_main_worktree_s_changes() {
        let fixture = repo_on_a();
        let path = follower(&fixture);
        fs::write(path.join("a.txt"), "from the follower").unwrap();
        fixture.git_in(
            &path,
            &["commit", "--quiet", "--all", "--message", "feat: change a"],
        );
        fixture.write("a.txt", "in progress");
        let a = tip(&fixture, "a");

        let result = workspace_at(&fixture, &path).land(None);

        assert!(matches!(result, Err(Error::Git(_))), "{result:?}");
        assert_eq!(tip(&fixture, "a"), a);
        assert_eq!(read(&fixture.path(), "a.txt"), "in progress");
    }

    #[test]
    fn landing_on_a_branch_nobody_has_checked_out_just_moves_it() {
        let fixture = repo_on_a();
        let path = follower(&fixture);
        fixture.git(&["switch", "--quiet", "develop"]);
        fixture.git_in(
            &path,
            &[
                "commit",
                "--quiet",
                "--allow-empty",
                "--message",
                "feat: mine",
            ],
        );

        let landed = workspace_at(&fixture, &path).land(None).unwrap();

        assert_eq!(
            (landed.holder, tip(&fixture, "a")),
            (None, fixture.git_in(&path, &["rev-parse", "HEAD"]))
        );
    }

    #[test]
    fn landing_can_be_undone() {
        let fixture = repo_on_a();
        let path = follower(&fixture);
        fixture.git_in(
            &path,
            &[
                "commit",
                "--quiet",
                "--allow-empty",
                "--message",
                "feat: mine",
            ],
        );
        let a = tip(&fixture, "a");
        workspace_at(&fixture, &path).land(None).unwrap();

        // The main worktree has `a` checked out, so undo runs from there.
        workspace(&fixture).undo().unwrap();

        assert_eq!(tip(&fixture, "a"), a);
        assert_eq!(fixture.git(&["status", "--porcelain"]), "");
    }

    #[test]
    fn landing_needs_a_follower_with_something_to_land() {
        let fixture = repo_on_a();
        let path = follower(&fixture);

        assert!(matches!(
            workspace(&fixture).land(None),
            Err(Error::NotDetached)
        ));
        assert!(matches!(
            workspace_at(&fixture, &path).land(None),
            Err(Error::NothingToLand { .. })
        ));
        fixture.git_in(&path, &["switch", "--quiet", "--detach", "develop"]);
        let other = fixture.scratch_path("plain");
        fixture.git(&[
            "worktree",
            "add",
            "--quiet",
            "--detach",
            other.to_str().unwrap(),
            "develop",
        ]);
        assert!(matches!(
            workspace_at(&fixture, &other).land(None),
            Err(Error::NotFollowing)
        ));
    }
}
