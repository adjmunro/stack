//! Splitting a branch into a stack.

mod common;

use common::*;
use stack_core::Error;
use stack_testkit::Fixture;

/// `develop` ← `d` with three commits; returns their ids, oldest first.
fn long_branch() -> (Fixture, Vec<String>) {
    let fixture = repo();
    fixture.git(&["switch", "--quiet", "--create", "d", "develop"]);
    let commits = ["b", "c", "d"]
        .map(|name| fixture.commit(&format!("{name}.txt"), name, &format!("feat: {name}")));
    (fixture, commits.to_vec())
}

#[test]
fn creates_the_branches_and_they_stack_in_order() {
    let (fixture, commits) = long_branch();

    let created = workspace(&fixture)
        .split(
            "d",
            &[
                (commits[0].clone(), "b".into()),
                (commits[1].clone(), "c".into()),
            ],
        )
        .unwrap();

    assert_eq!(created, ["b", "c"]);
    assert_eq!(shape(&tree(&fixture)), "develop(b(c(d)))");
    assert_eq!(
        workspace(&fixture).oplog(1).unwrap()[0].description,
        "split d into b, c"
    );
}

#[test]
fn undo_removes_every_new_branch() {
    let (fixture, commits) = long_branch();
    let before = fixture.snapshot();
    workspace(&fixture)
        .split(
            "d",
            &[
                (commits[0].clone(), "b".into()),
                (commits[1].clone(), "c".into()),
            ],
        )
        .unwrap();

    workspace(&fixture).undo().unwrap();

    assert!(before.diff(&fixture.snapshot()).refs.is_empty());
}

#[test]
fn refuses_points_outside_the_branch_and_taken_names() {
    let (fixture, commits) = long_branch();
    let before = fixture.snapshot();
    let workspace = workspace(&fixture);
    let base = tip(&fixture, "develop");

    for (revision, name) in [(base, "x"), (commits[2].clone(), "x")] {
        let result = workspace.split("d", &[(revision, name.into())]);
        assert!(
            matches!(result, Err(Error::NotOwnCommit { .. })),
            "{result:?}"
        );
    }
    let result = workspace.split("d", &[(commits[0].clone(), "develop".into())]);
    assert!(matches!(result, Err(Error::BranchExists { .. })));
    let result = workspace.split(
        "d",
        &[
            (commits[0].clone(), "b".into()),
            (commits[1].clone(), "b".into()),
        ],
    );
    assert!(matches!(result, Err(Error::BranchExists { .. })));
    fixture.assert_unchanged(&before);
}
