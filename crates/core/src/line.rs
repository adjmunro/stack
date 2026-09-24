//! Lines: the branches around one branch that push (and later PRs) cover by default. See the glossary.

use std::collections::{BTreeMap, VecDeque};

use crate::resolve::Resolution;
use crate::{Direction, Role, Scope};

/// The branches in `branch`'s line under `scope`, rootward first. Never includes a trunk.
///
/// - Rootward: parents up to the nearest trunk or limb, excluded. Nothing rootward of a limb or trunk.
/// - Leafward: descendants up to and including the next limbs, or to the leaves if `scope.through_limbs`.
pub(crate) fn line(resolution: &Resolution, branch: &str, scope: Scope) -> Vec<String> {
    let role = |name: &str| resolution.branches[name].role;
    let mut line = Vec::new();
    if scope.direction != Direction::Leafward && role(branch) == Role::Branch {
        let mut current = branch;
        while let Some(parent) = resolution.branches[current].parent.as_ref() {
            if role(&parent.name) != Role::Branch {
                break;
            }
            line.push(parent.name.clone());
            current = &parent.name;
        }
        line.reverse();
    }
    if role(branch) != Role::Trunk {
        line.push(branch.to_owned());
    }
    if scope.direction != Direction::Rootward {
        let mut children: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for (name, entry) in &resolution.branches {
            if let Some(parent) = &entry.parent {
                children.entry(parent.name.as_str()).or_default().push(name);
            }
        }
        let mut queue: VecDeque<&str> = children
            .get(branch)
            .into_iter()
            .flatten()
            .copied()
            .collect();
        while let Some(child) = queue.pop_front() {
            line.push(child.to_owned());
            if role(child) != Role::Limb || scope.through_limbs {
                queue.extend(children.get(child).into_iter().flatten());
            }
        }
    }
    line
}
