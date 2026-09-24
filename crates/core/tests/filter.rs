//! Reducing the tree to branches that change given paths.

mod common;

use common::*;
use stack_testkit::Fixture;

/// `develop` ← `a` (a.txt, docs/guide.md) ← `b` (b.txt); `develop` ← `c` (c.txt).
fn stacks() -> Fixture {
    let fixture = repo();
    grow(&fixture, "a", "develop");
    fixture.commit("docs/guide.md", "guide", "docs: guide");
    grow(&fixture, "b", "a");
    grow(&fixture, "c", "develop");
    fixture
}

fn touching(fixture: &Fixture, paths: &[&str]) -> String {
    let paths: Vec<String> = paths.iter().map(|path| (*path).to_owned()).collect();
    shape(&workspace(fixture).tree_touching(&paths).unwrap())
}

#[test]
fn keeps_matching_branches_and_the_branches_beneath_them() {
    let fixture = stacks();
    assert_eq!(touching(&fixture, &["b.txt"]), "develop(a(b))");
    assert_eq!(touching(&fixture, &["c.txt"]), "develop(c)");
    assert_eq!(touching(&fixture, &["a.txt", "c.txt"]), "develop(a c)");
}

#[test]
fn pathspecs_match_directories_and_globs() {
    let fixture = stacks();
    assert_eq!(touching(&fixture, &["docs"]), "develop(a)");
    assert_eq!(touching(&fixture, &["*.md"]), "develop(a)");
}

#[test]
fn only_a_branch_s_own_commits_count() {
    let fixture = stacks();
    // base.txt was changed on develop, before any branch.
    assert_eq!(touching(&fixture, &["base.txt"]), "develop");
    assert_eq!(touching(&fixture, &["nowhere.txt"]), "develop");
}

#[test]
fn focusing_on_a_branch_keeps_its_line_and_what_is_on_it() {
    let fixture = stacks();
    grow(&fixture, "d", "b");
    let focus = |branch: &str| shape(&workspace(&fixture).tree_around(branch).unwrap());

    assert_eq!(focus("a"), "develop(a(b(d)))");
    assert_eq!(focus("d"), "develop(a(b(d)))");
    assert_eq!(focus("c"), "develop(c)");
    assert_eq!(focus("develop"), "develop(a(b(d)) c)");
}
