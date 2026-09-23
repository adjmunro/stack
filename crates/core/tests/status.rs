use stack_core::{Error, Head, Workspace};
use stack_testkit::Fixture;

fn head(fixture: &Fixture) -> Head {
    Workspace::discover(fixture.path())
        .unwrap()
        .status()
        .unwrap()
        .head
}

#[test]
fn unborn_branch_has_no_commit() {
    let fixture = Fixture::new();
    let before = fixture.snapshot();

    assert_eq!(
        head(&fixture),
        Head::Branch {
            name: "develop".into(),
            commit: None
        }
    );
    fixture.assert_unchanged(&before);
}

#[test]
fn branch_reports_its_tip() {
    let fixture = Fixture::new();
    let tip = fixture.commit("a.txt", "a", "feat: a");
    let before = fixture.snapshot();

    assert_eq!(
        head(&fixture),
        Head::Branch {
            name: "develop".into(),
            commit: Some(tip)
        }
    );
    fixture.assert_unchanged(&before);
}

#[test]
fn nested_branch_name_is_kept_whole() {
    let fixture = Fixture::new();
    let tip = fixture.commit("a.txt", "a", "feat: a");
    fixture.git(&["switch", "--quiet", "--create", "feat/nested"]);

    assert_eq!(
        head(&fixture),
        Head::Branch {
            name: "feat/nested".into(),
            commit: Some(tip)
        }
    );
}

#[test]
fn detached_head_reports_commit() {
    let fixture = Fixture::new();
    let tip = fixture.commit("a.txt", "a", "feat: a");
    fixture.git(&["switch", "--quiet", "--detach"]);
    let before = fixture.snapshot();

    assert_eq!(head(&fixture), Head::Detached { commit: tip });
    fixture.assert_unchanged(&before);
}

#[test]
fn discovers_from_subdirectory() {
    let fixture = Fixture::new();
    fixture.commit("dir/a.txt", "a", "feat: a");

    let status = Workspace::discover(fixture.path().join("dir"))
        .unwrap()
        .status()
        .unwrap();
    assert!(matches!(status.head, Head::Branch { ref name, .. } if name == "develop"));
}

#[test]
fn outside_a_repo_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    assert!(matches!(
        Workspace::discover(dir.path()),
        Err(Error::NotARepository { .. })
    ));
}
