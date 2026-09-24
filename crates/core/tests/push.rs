//! Lines, and pushing them.

mod common;

use common::*;
use stack_core::{Direction, Error, PushOutcome, Scope};
use stack_testkit::Fixture;

/// ```text
/// develop
/// └─ a
///    ├─ b (limb)
///    │  └─ c
///    │     └─ d
///    └─ s
/// ```
fn stacks() -> Fixture {
    let fixture = repo();
    grow(&fixture, "a", "develop");
    grow(&fixture, "s", "a");
    grow(&fixture, "b", "a");
    workspace(&fixture).add_trunk("b").unwrap();
    grow(&fixture, "c", "b");
    grow(&fixture, "d", "c");
    fixture.git(&["switch", "--quiet", "develop"]);
    fixture
}

fn line(fixture: &Fixture, branch: &str, direction: Direction, through_limbs: bool) -> Vec<String> {
    workspace(fixture)
        .line(
            branch,
            Scope {
                direction,
                through_limbs,
            },
        )
        .unwrap()
}

fn remote_refs(fixture: &Fixture, remote: &std::path::Path) -> String {
    fixture.git_in(
        remote,
        &["for-each-ref", "--format=%(refname:short)", "refs/heads/"],
    )
}

mod lines {
    use super::*;

    #[test]
    fn a_branch_takes_its_parents_to_the_nearest_trunk_or_limb_and_descendants_to_the_next_limbs() {
        let fixture = stacks();
        assert_eq!(line(&fixture, "a", Direction::Both, false), ["a", "b", "s"]);
        assert_eq!(line(&fixture, "s", Direction::Both, false), ["a", "s"]);
        assert_eq!(line(&fixture, "d", Direction::Both, false), ["c", "d"]);
    }

    #[test]
    fn a_limb_is_its_own_stack() {
        let fixture = stacks();
        assert_eq!(line(&fixture, "b", Direction::Both, false), ["b", "c", "d"]);
    }

    #[test]
    fn a_trunk_is_never_included() {
        let fixture = stacks();
        assert_eq!(
            line(&fixture, "develop", Direction::Both, false),
            ["a", "b", "s"]
        );
    }

    #[test]
    fn scopes_narrow_or_extend_the_line() {
        let fixture = stacks();
        assert_eq!(line(&fixture, "d", Direction::Rootward, false), ["c", "d"]);
        assert_eq!(
            line(&fixture, "a", Direction::Leafward, false),
            ["a", "b", "s"]
        );
        assert_eq!(line(&fixture, "s", Direction::Rootward, false), ["a", "s"]);
        assert_eq!(
            line(&fixture, "a", Direction::Both, true),
            ["a", "b", "s", "c", "d"]
        );
    }
}

mod pushing {
    use super::*;

    #[test]
    fn pushes_the_line_to_the_same_names_and_sets_upstreams() {
        let fixture = stacks();
        let remote = fixture.add_bare_remote("origin");

        let pushed = workspace(&fixture)
            .push("s", Scope::default(), None)
            .unwrap();

        assert_eq!(pushed.remote, "origin");
        let outcomes: Vec<(&str, PushOutcome)> = pushed
            .branches
            .iter()
            .map(|branch| (branch.name.as_str(), branch.outcome))
            .collect();
        assert_eq!(
            outcomes,
            [("a", PushOutcome::Created), ("s", PushOutcome::Created)]
        );
        assert_eq!(remote_refs(&fixture, &remote), "a\ns");
        assert_eq!(fixture.git(&["config", "branch.s.merge"]), "refs/heads/s");
        for branch in ["a", "s"] {
            assert_eq!(
                fixture.git_in(&remote, &["rev-parse", branch]),
                tip(&fixture, branch)
            );
        }
    }

    #[test]
    fn rewritten_branches_are_force_pushed_under_a_lease() {
        let fixture = stacks();
        let remote = fixture.add_bare_remote("origin");
        workspace(&fixture)
            .push("s", Scope::default(), None)
            .unwrap();
        fixture.commit("later.txt", "later", "feat: later");
        workspace(&fixture).restack("develop").unwrap();

        let pushed = workspace(&fixture)
            .push("s", Scope::default(), None)
            .unwrap();

        assert!(
            pushed
                .branches
                .iter()
                .all(|branch| branch.outcome == PushOutcome::Forced),
            "{pushed:?}"
        );
        assert_eq!(
            fixture.git_in(&remote, &["rev-parse", "s"]),
            tip(&fixture, "s")
        );
        let again = workspace(&fixture)
            .push("s", Scope::default(), None)
            .unwrap();
        assert!(
            again
                .branches
                .iter()
                .all(|branch| branch.outcome == PushOutcome::UpToDate)
        );
    }

    #[test]
    fn a_remote_branch_changed_by_someone_else_is_never_overwritten() {
        let fixture = stacks();
        let remote = fixture.add_bare_remote("origin");
        workspace(&fixture)
            .push(
                "a",
                Scope {
                    direction: Direction::Rootward,
                    through_limbs: false,
                },
                None,
            )
            .unwrap();
        // Someone else pushes to a.
        let other = fixture.scratch_path("other");
        fixture.git(&[
            "clone",
            "--quiet",
            remote.to_str().unwrap(),
            other.to_str().unwrap(),
        ]);
        fixture.git_in(&other, &["switch", "--quiet", "a"]);
        fixture.git_in(
            &other,
            &["commit", "--quiet", "--allow-empty", "--message", "theirs"],
        );
        fixture.git_in(&other, &["push", "--quiet", "origin", "a"]);
        let theirs = fixture.git_in(&other, &["rev-parse", "a"]);
        fixture.git(&["switch", "--quiet", "a"]);
        fixture.git(&[
            "commit",
            "--quiet",
            "--amend",
            "--allow-empty",
            "--message",
            "ours",
        ]);

        let pushed = workspace(&fixture)
            .push(
                "a",
                Scope {
                    direction: Direction::Rootward,
                    through_limbs: false,
                },
                None,
            )
            .unwrap();

        assert_eq!(pushed.branches[0].outcome, PushOutcome::Rejected);
        assert!(
            pushed.branches[0].summary.contains("stale info"),
            "{}",
            pushed.branches[0].summary
        );
        assert_eq!(fixture.git_in(&remote, &["rev-parse", "a"]), theirs);
    }

    #[test]
    fn trunks_and_siblings_are_never_pushed() {
        let fixture = stacks();
        let remote = fixture.add_bare_remote("origin");

        workspace(&fixture)
            .push("d", Scope::default(), None)
            .unwrap();

        assert_eq!(remote_refs(&fixture, &remote), "c\nd");
    }

    #[test]
    fn remote_choice() {
        let fixture = stacks();
        fixture.add_bare_remote("backup");
        let origin = fixture.add_bare_remote("origin");
        let rootward = Scope {
            direction: Direction::Rootward,
            through_limbs: false,
        };

        assert_eq!(
            workspace(&fixture)
                .push("a", rootward, None)
                .unwrap()
                .remote,
            "origin"
        );
        assert_eq!(remote_refs(&fixture, &origin), "a");
        assert_eq!(
            workspace(&fixture)
                .push("s", rootward, Some("backup"))
                .unwrap()
                .remote,
            "backup"
        );
        // A backup to a second remote keeps the upstream that origin set.
        assert_eq!(fixture.git(&["config", "branch.a.remote"]), "origin");
        assert_eq!(fixture.git(&["config", "branch.s.remote"]), "backup");
        assert!(matches!(
            workspace(&fixture).push("s", rootward, Some("nope")),
            Err(Error::UnknownRemote { .. })
        ));
    }

    #[test]
    fn refusals_change_nothing() {
        let fixture = stacks();
        assert!(matches!(
            workspace(&fixture).push("a", Scope::default(), None),
            Err(Error::NoRemote)
        ));

        fixture.add_bare_remote("origin");
        fixture.git(&["config", "branch.a.remote", "origin"]);
        fixture.git(&["config", "branch.a.merge", "refs/heads/elsewhere"]);
        let before = fixture.snapshot();
        let result = workspace(&fixture).push("s", Scope::default(), None);
        assert!(
            matches!(result, Err(Error::UpstreamMismatch { ref branch, ref upstream, .. }) if branch == "a" && upstream == "origin/elsewhere"),
            "{result:?}"
        );
        fixture.assert_unchanged(&before);
    }
}
