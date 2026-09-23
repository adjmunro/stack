#![allow(dead_code)]

use stack_core::{Environment, Node, Parent, Tree, Workspace};
use stack_testkit::Fixture;

/// A workspace on the fixture, running `git` in the fixture's sealed environment.
pub fn workspace(fixture: &Fixture) -> Workspace {
    Workspace::discover_with(fixture.path(), Environment::Exactly(fixture.environment())).unwrap()
}

/// A repo with one commit on `develop`, marked as a trunk.
pub fn repo() -> Fixture {
    let fixture = Fixture::new();
    fixture.commit("base.txt", "base", "feat: base");
    workspace(&fixture).add_trunk("develop").unwrap();
    fixture
}

/// Creates `name` from `from` with one commit, leaving HEAD on it.
pub fn grow(fixture: &Fixture, name: &str, from: &str) -> String {
    fixture.git(&["switch", "--quiet", "--create", name, from]);
    fixture.commit(&format!("{name}.txt"), name, &format!("feat: {name}"))
}

pub fn tip(fixture: &Fixture, reference: &str) -> String {
    fixture.git(&["rev-parse", reference])
}

/// Writes a recorded parent directly, as a future `stack` operation would.
pub fn record(fixture: &Fixture, branch: &str, parent: &str, offshoot: &str, pinned: bool) {
    let json =
        format!(r#"{{"version":1,"parent":"{parent}","offshoot":"{offshoot}","pinned":{pinned}}}"#);
    let blob = fixture.git_stdin(&["hash-object", "-w", "--stdin"], json.as_bytes());
    fixture.git(&[
        "update-ref",
        &format!("refs/stack/branches/{branch}"),
        &blob,
    ]);
}

pub fn tree(fixture: &Fixture) -> Tree {
    workspace(fixture).tree().unwrap()
}

/// The tree as `trunk(child(grandchild) child) | ~unattached`.
pub fn shape(tree: &Tree) -> String {
    fn node(branch: &Node) -> String {
        if branch.children.is_empty() {
            return branch.name.clone();
        }
        let children: Vec<String> = branch.children.iter().map(node).collect();
        format!("{}({})", branch.name, children.join(" "))
    }
    let mut parts: Vec<String> = tree.trunks.iter().map(node).collect();
    parts.extend(
        tree.unattached
            .iter()
            .map(|branch| format!("~{}", node(branch))),
    );
    parts.join(" | ")
}

pub fn parent(tree: &Tree, name: &str) -> Option<Parent> {
    fn find<'a>(nodes: &'a [Node], name: &str) -> Option<&'a Node> {
        nodes.iter().find_map(|node| {
            if node.name == name {
                Some(node)
            } else {
                find(&node.children, name)
            }
        })
    }
    find(&tree.trunks, name)
        .or_else(|| find(&tree.unattached, name))
        .expect("branch in tree")
        .parent
        .clone()
}
