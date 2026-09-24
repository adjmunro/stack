//! Proposing branches as pull requests, against a fake `gh`.

mod common;

use std::ffi::OsString;
use std::path::PathBuf;

use common::*;
use stack_core::{Direction, Environment, ProposalAction, Scope, Workspace};
use stack_testkit::Fixture;

/// A fake `gh` on `PATH` whose pull requests live in `state`.
struct FakeGh {
    bin: PathBuf,
    state: PathBuf,
}

impl FakeGh {
    fn install(fixture: &Fixture) -> Self {
        let bin = fixture.scratch_path("bin");
        let state = fixture.scratch_path("gh-state");
        std::fs::create_dir(&bin).unwrap();
        std::fs::create_dir(&state).unwrap();
        std::fs::copy(
            concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fake-gh.sh"),
            bin.join("gh"),
        )
        .unwrap();
        Self { bin, state }
    }

    fn workspace(&self, fixture: &Fixture) -> Workspace {
        let environment = fixture
            .environment()
            .into_iter()
            .map(|(key, value)| {
                if key == "PATH" {
                    let mut path = OsString::from(self.bin.as_os_str());
                    path.push(":");
                    path.push(value);
                    (key, path)
                } else {
                    (key, value)
                }
            })
            .chain([(
                OsString::from("FAKE_GH_DIR"),
                self.state.clone().into_os_string(),
            )])
            .collect();
        Workspace::discover_with(fixture.path(), Environment::Exactly(environment)).unwrap()
    }

    fn calls(&self) -> Vec<String> {
        std::fs::read_to_string(self.state.join("calls"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    fn set(&self, branch: &str, number: u64, base: &str, state: &str) {
        std::fs::write(
            self.state.join(branch.replace('/', "__")),
            format!("{number} {base} {state}\n"),
        )
        .unwrap();
    }
}

/// `develop` (trunk, on origin) ← `a` ← `b`, HEAD on `b`.
fn stack_on_origin() -> Fixture {
    let fixture = repo();
    fixture.add_bare_remote("origin");
    fixture.git(&["push", "--quiet", "origin", "develop"]);
    grow(&fixture, "a", "develop");
    grow(&fixture, "b", "a");
    fixture
}

fn actions(proposed: &stack_core::Proposed) -> Vec<(String, String, ProposalAction)> {
    proposed
        .pull_requests
        .iter()
        .map(|pr| (pr.branch.clone(), pr.base.clone(), pr.action.clone()))
        .collect()
}

#[test]
fn opens_pull_requests_against_each_parent() {
    let fixture = stack_on_origin();
    let gh = FakeGh::install(&fixture);

    let proposed = gh
        .workspace(&fixture)
        .propose("b", Scope::default(), None, false)
        .unwrap();

    assert_eq!(
        actions(&proposed),
        [
            ("a".into(), "develop".into(), ProposalAction::Created),
            ("b".into(), "a".into(), ProposalAction::Created),
        ]
    );
    assert_eq!(
        proposed.pull_requests[1].url.as_deref(),
        Some("https://example.test/pull/2")
    );
    let creates: Vec<String> = gh
        .calls()
        .into_iter()
        .filter(|call| call.starts_with("pr create"))
        .collect();
    assert_eq!(
        creates,
        [
            "pr create --head a --base develop --fill",
            "pr create --head b --base a --fill"
        ]
    );
}

#[test]
fn a_second_run_changes_nothing() {
    let fixture = stack_on_origin();
    let gh = FakeGh::install(&fixture);
    gh.workspace(&fixture)
        .propose("b", Scope::default(), None, false)
        .unwrap();

    let proposed = gh
        .workspace(&fixture)
        .propose("b", Scope::default(), None, false)
        .unwrap();

    assert!(
        proposed
            .pull_requests
            .iter()
            .all(|pr| pr.action == ProposalAction::UpToDate)
    );
    assert_eq!(
        gh.calls()
            .iter()
            .filter(|call| call.starts_with("pr create"))
            .count(),
        2
    );
}

#[test]
fn a_moved_branch_s_pull_request_is_retargeted() {
    let fixture = stack_on_origin();
    let gh = FakeGh::install(&fixture);
    gh.workspace(&fixture)
        .propose("b", Scope::default(), None, false)
        .unwrap();
    gh.workspace(&fixture).move_branch("b", "develop").unwrap();

    let leafward = Scope {
        direction: Direction::Leafward,
        through_limbs: false,
    };
    let proposed = gh
        .workspace(&fixture)
        .propose("b", leafward, None, false)
        .unwrap();

    assert_eq!(
        actions(&proposed),
        [(
            "b".into(),
            "develop".into(),
            ProposalAction::Retargeted { from: "a".into() }
        )]
    );
    assert!(gh.calls().contains(&"pr edit 2 --base develop".to_owned()));
}

#[test]
fn merged_pull_requests_are_left_alone_and_drafts_are_drafts() {
    let fixture = stack_on_origin();
    let gh = FakeGh::install(&fixture);
    gh.set("a", 7, "develop", "MERGED");

    let proposed = gh
        .workspace(&fixture)
        .propose("b", Scope::default(), None, true)
        .unwrap();

    assert_eq!(
        proposed.pull_requests[0].action,
        ProposalAction::Skipped {
            reason: "its pull request is merged".into()
        }
    );
    assert!(
        gh.calls()
            .contains(&"pr create --head b --base a --fill --draft".to_owned())
    );
}

#[test]
fn a_parent_missing_from_the_remote_is_reported_not_guessed() {
    let fixture = repo();
    fixture.add_bare_remote("origin");
    grow(&fixture, "a", "develop");
    let gh = FakeGh::install(&fixture);

    let proposed = gh
        .workspace(&fixture)
        .propose("a", Scope::default(), None, false)
        .unwrap();

    assert_eq!(
        proposed.pull_requests[0].action,
        ProposalAction::Skipped {
            reason: "its parent develop isn't on origin".into()
        }
    );
    assert!(gh.calls().iter().all(|call| !call.starts_with("pr create")));
}
