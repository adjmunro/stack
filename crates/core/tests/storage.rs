//! How `stack` metadata sits in a real repo alongside plain git.

mod common;

use common::*;

#[test]
fn metadata_is_not_pushed_or_cloned_by_default() {
    let fixture = repo();
    grow(&fixture, "a", "develop");
    workspace(&fixture).pin("a", None).unwrap();
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
    let fixture = repo();
    grow(&fixture, "a", "develop");
    let log_before = fixture.git(&["log", "--all", "--oneline"]);
    workspace(&fixture).pin("a", None).unwrap();

    assert_eq!(fixture.git(&["log", "--all", "--oneline"]), log_before);
}

#[test]
fn metadata_survives_gc_and_packed_refs() {
    let fixture = repo();
    grow(&fixture, "a", "develop");
    workspace(&fixture).pin("a", None).unwrap();
    let before = tree(&fixture);
    fixture.git(&["gc", "--quiet", "--prune=now"]);
    fixture.git(&["pack-refs", "--all"]);

    assert_eq!(tree(&fixture), before);
    // Compare-and-swap against a packed ref.
    workspace(&fixture).unpin("a").unwrap();
    assert_eq!(fixture.git(&["for-each-ref", "refs/stack/branches/a"]), "");
    fixture.git(&["fsck", "--no-progress"]);
}

#[test]
fn corrupt_metadata_is_reported() {
    let fixture = repo();
    grow(&fixture, "a", "develop");
    let garbage = fixture.git_stdin(&["hash-object", "-w", "--stdin"], b"not json");
    fixture.git(&["update-ref", "refs/stack/branches/a", &garbage]);
    assert!(matches!(
        workspace(&fixture).tree(),
        Err(stack_core::Error::CorruptMetadata { .. })
    ));

    fixture.git(&["update-ref", "refs/stack/branches/a", "develop"]);
    assert!(matches!(
        workspace(&fixture).tree(),
        Err(stack_core::Error::CorruptMetadata { .. })
    ));
}
