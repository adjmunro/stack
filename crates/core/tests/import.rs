//! Importing Graphite's stacks.

mod common;

use common::*;
use stack_core::{Outcome, Source};
use stack_testkit::Fixture;

/// Writes Graphite's metadata for `branch`, as Graphite does.
fn graphite_parent(fixture: &Fixture, branch: &str, parent: &str, revision: &str) {
    let json = format!(
        r#"{{"parentBranchName":"{parent}","parentBranchRevision":"{revision}","prInfo":{{}}}}"#
    );
    let blob = fixture.git_stdin(&["hash-object", "-w", "--stdin"], json.as_bytes());
    fixture.git(&[
        "update-ref",
        &format!("refs/branch-metadata/{branch}"),
        &blob,
    ]);
}

fn graphite_trunk(fixture: &Fixture, trunk: &str) {
    std::fs::write(
        fixture.path().join(".git/.graphite_repo_config"),
        format!(r#"{{"trunk":"{trunk}"}}"#),
    )
    .unwrap();
}

/// `develop` ← `a` ← `b`, tracked by Graphite; no stack metadata yet.
fn graphite_repo() -> Fixture {
    let fixture = Fixture::new();
    fixture.commit("base.txt", "base", "feat: base");
    grow(&fixture, "a", "develop");
    grow(&fixture, "b", "a");
    graphite_trunk(&fixture, "develop");
    graphite_parent(&fixture, "a", "develop", &tip(&fixture, "develop"));
    graphite_parent(&fixture, "b", "a", &tip(&fixture, "a"));
    fixture
}

#[test]
fn imports_trunks_and_parents_as_recorded_hints() {
    let fixture = graphite_repo();
    let graphite_refs = fixture.git(&["for-each-ref", "refs/branch-metadata/"]);

    let imported = workspace(&fixture).import_graphite().unwrap();

    assert_eq!(
        (imported.outcome, imported.trunks.as_slice()),
        (Outcome::Changed, ["develop".to_owned()].as_slice())
    );
    assert_eq!(imported.parents.len(), 2);
    assert!(imported.skipped.is_empty(), "{:?}", imported.skipped);
    let tree = tree(&fixture);
    assert_eq!(shape(&tree), "develop(a(b))");
    assert_eq!(parent(&tree, "b").unwrap().source, Source::Recorded);
    assert_eq!(
        fixture.git(&["for-each-ref", "refs/branch-metadata/"]),
        graphite_refs
    );
}

#[test]
fn stale_graphite_tracking_gives_way_to_the_graph() {
    let fixture = graphite_repo();
    grow(&fixture, "c", "develop");
    // b was moved onto c with plain git; Graphite still says a.
    fixture.git(&["rebase", "--quiet", "--onto", "c", "a", "b"]);

    workspace(&fixture).import_graphite().unwrap();

    let tree = tree(&fixture);
    assert_eq!(shape(&tree), "develop(a c(b))");
    assert_eq!(parent(&tree, "b").unwrap().replaces.as_deref(), Some("a"));
}

#[test]
fn skips_what_it_cannot_or_should_not_import() {
    let fixture = graphite_repo();
    graphite_parent(&fixture, "gone", "develop", &tip(&fixture, "develop"));
    graphite_parent(&fixture, "develop", "a", &tip(&fixture, "a"));
    workspace(&fixture).add_trunk("develop").unwrap();
    workspace(&fixture).pin("a", Some("develop")).unwrap();

    let imported = workspace(&fixture).import_graphite().unwrap();

    let mut skipped: Vec<(String, String)> = imported
        .skipped
        .into_iter()
        .map(|skipped| (skipped.branch, skipped.reason))
        .collect();
    skipped.sort();
    assert_eq!(
        skipped,
        [
            (
                "a".to_owned(),
                "stack already records its parent".to_owned()
            ),
            ("develop".to_owned(), "it's a trunk".to_owned()),
            (
                "gone".to_owned(),
                "it or its parent no longer exists".to_owned()
            ),
        ]
    );
    assert_eq!(imported.parents.len(), 1);
}

#[test]
fn undo_reverts_the_import_and_a_second_import_changes_nothing() {
    let fixture = graphite_repo();
    let before = fixture.snapshot();
    workspace(&fixture).import_graphite().unwrap();
    let after = fixture.snapshot();

    assert_eq!(
        workspace(&fixture).import_graphite().unwrap().outcome,
        Outcome::Unchanged
    );
    assert!(after.diff(&fixture.snapshot()).is_empty());

    workspace(&fixture).undo().unwrap();
    let diff = before.diff(&fixture.snapshot());
    assert!(
        diff.refs.keys().all(|name| name == "refs/stack/keep"),
        "{diff}"
    );
}

#[test]
fn without_graphite_nothing_happens() {
    let fixture = repo();
    let before = fixture.snapshot();

    let imported = workspace(&fixture).import_graphite().unwrap();

    assert_eq!(imported.outcome, Outcome::Unchanged);
    fixture.assert_unchanged(&before);
}
