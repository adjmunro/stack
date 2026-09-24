//! Works out every branch's parent from the commit graph and reconciles it with recorded parents.
//!
//! - **Derived** parent: the nearest branch with a version among this branch's own commits (those not in any trunk's
//!   history), else the trunk with the latest merge base. A version is the branch's tip, or a former tip (reflog)
//!   it moved forward from, was amended or rebased off, or still carries a patch of. Branches on the same commit:
//!   the older is the parent.
//! - **Recorded** parent: holds while the branch contains some version of it (its tip, or the recorded offshoot)
//!   and hasn't been moved onto a branch leafward of that version. Otherwise the derived parent replaces it.
//! - **Pinned** parent: always holds while the parent exists; flagged when contradicted.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use crate::git::{Branch, GitRepo};
use crate::metadata::Metadata;
use crate::{Error, Node, Parent, Role, Source, Tree};

/// A branch with its resolved role and parent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Resolved {
    pub tip: String,
    pub role: Role,
    pub parent: Option<Parent>,
}

/// Every local branch, resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Resolution {
    pub branches: BTreeMap<String, Resolved>,
    /// Whether any trunk exists. Without one, nothing is derived.
    has_trunks: bool,
}

impl Resolution {
    pub(crate) fn resolve(git: &dyn GitRepo, metadata: &Metadata) -> Result<Self, Error> {
        let branches: BTreeMap<String, Branch> = git
            .branches()?
            .into_iter()
            .map(|branch| (branch.name.clone(), branch))
            .collect();
        let role = |name: &str| {
            metadata
                .marks
                .get(name)
                .map_or(Role::Branch, |mark| mark.value.role)
        };
        let trunks: Vec<&Branch> = branches
            .values()
            .filter(|branch| role(&branch.name) == Role::Trunk)
            .collect();

        let mut resolved: BTreeMap<String, Resolved> = branches
            .values()
            .map(|branch| {
                let value = Resolved {
                    tip: branch.tip.clone(),
                    role: role(&branch.name),
                    parent: None,
                };
                (branch.name.clone(), value)
            })
            .collect();
        if trunks.is_empty() {
            return Ok(Self {
                branches: resolved,
                has_trunks: false,
            });
        }

        let resolver = Resolver::new(git, metadata, &branches, trunks);
        let mut derived = BTreeMap::new();
        for (name, entry) in &mut resolved {
            if entry.role == Role::Trunk {
                continue;
            }
            let parent = resolver.derive(name)?;
            entry.parent = resolver.reconcile(name, parent.clone())?;
            derived.insert(name.clone(), parent);
        }
        break_cycles(&mut resolved, &derived);
        Ok(Self {
            branches: resolved,
            has_trunks: true,
        })
    }

    /// `name` followed by its parent, grandparent, and so on.
    pub(crate) fn lineage<'a>(&'a self, name: &'a str) -> Vec<&'a str> {
        let mut lineage = vec![name];
        let mut current = name;
        while let Some(parent) = self
            .branches
            .get(current)
            .and_then(|entry| entry.parent.as_ref())
        {
            current = &parent.name;
            lineage.push(current);
        }
        lineage
    }

    pub(crate) fn tree(&self, current: Option<&str>) -> Tree {
        let mut children: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        let mut roots = Vec::new();
        for (name, entry) in &self.branches {
            match &entry.parent {
                Some(parent) => children.entry(parent.name.as_str()).or_default().push(name),
                None => roots.push(name.as_str()),
            }
        }
        let node = |name: &str| self.node(name, &children, current);
        let (trunks, unattached): (Vec<&str>, Vec<&str>) = roots
            .into_iter()
            .partition(|name| self.has_trunks && self.branches[*name].role == Role::Trunk);
        Tree {
            trunks: trunks.into_iter().map(node).collect(),
            unattached: unattached.into_iter().map(node).collect(),
        }
    }

    fn node(
        &self,
        name: &str,
        children: &BTreeMap<&str, Vec<&str>>,
        current: Option<&str>,
    ) -> Node {
        let entry = &self.branches[name];
        Node {
            name: name.to_owned(),
            commit: entry.tip.clone(),
            role: entry.role,
            current: current == Some(name),
            parent: entry.parent.clone(),
            children: children
                .get(name)
                .into_iter()
                .flatten()
                .map(|child| self.node(child, children, current))
                .collect(),
        }
    }
}

struct Resolver<'a> {
    git: &'a dyn GitRepo,
    metadata: &'a Metadata,
    branches: &'a BTreeMap<String, Branch>,
    /// Sorted oldest first.
    trunks: Vec<&'a Branch>,
    trunk_tips: Vec<String>,
}

impl<'a> Resolver<'a> {
    fn new(
        git: &'a dyn GitRepo,
        metadata: &'a Metadata,
        branches: &'a BTreeMap<String, Branch>,
        mut trunks: Vec<&'a Branch>,
    ) -> Self {
        trunks.sort_by_key(|branch| age(branch));
        let trunk_tips = trunks.iter().map(|trunk| trunk.tip.clone()).collect();
        Self {
            git,
            metadata,
            branches,
            trunks,
            trunk_tips,
        }
    }

    fn is_trunk(&self, name: &str) -> bool {
        self.trunks.iter().any(|trunk| trunk.name == name)
    }

    /// The parent the commit graph implies, or `None` if the branch shares no history with any trunk.
    fn derive(&self, name: &str) -> Result<Option<Parent>, Error> {
        let branch = &self.branches[name];
        let own: HashSet<String> = self
            .git
            .commits_excluding(&branch.tip, &self.trunk_tips)?
            .into_iter()
            .collect();

        // The whole branch is already in a trunk's history (e.g. new and empty, or merged).
        if own.is_empty() {
            for trunk in &self.trunks {
                if self.git.is_ancestor(&branch.tip, &trunk.tip)? {
                    return Ok(Some(self.derived(trunk, branch.tip.clone())));
                }
            }
            return Ok(None);
        }

        // Each other branch with a version (its tip, or a confirmed former tip) among our own commits.
        let mut candidates: Vec<(&Branch, String)> = Vec::new();
        for other in self.branches.values() {
            if other.name == name || self.is_trunk(&other.name) {
                continue;
            }
            let mut latest: Option<String> = None;
            let versions =
                std::iter::once(&other.tip).chain(other.former.iter().map(|former| &former.id));
            for version in versions.filter(|version| own.contains(*version)) {
                if *version == branch.tip && age(other) >= age(branch) {
                    continue;
                }
                if *version != other.tip && !self.is_former_version(other, version)? {
                    continue;
                }
                latest = Some(self.later(latest, version.clone())?);
            }
            let Some(version) = latest else { continue };
            // A branch built on this one (e.g. the long branch that was split to create it) is never its parent,
            // even if it once pointed where this one now does.
            if other.tip != branch.tip && self.git.is_ancestor(&branch.tip, &other.tip)? {
                continue;
            }
            candidates.push((other, version));
        }
        let mut nearest: Vec<&(&Branch, String)> = Vec::new();
        for candidate in &candidates {
            let mut below_another = false;
            for other in &candidates {
                if other.1 != candidate.1 && self.git.is_ancestor(&candidate.1, &other.1)? {
                    below_another = true;
                    break;
                }
            }
            if !below_another {
                nearest.push(candidate);
            }
        }
        if let Some((parent, version)) = nearest
            .into_iter()
            .min_by_key(|(candidate, _)| age(candidate))
        {
            return Ok(Some(self.derived(parent, version.clone())));
        }

        let mut best: Option<(&Branch, String)> = None;
        for trunk in &self.trunks {
            let Some(base) = self.git.merge_base(&branch.tip, &trunk.tip)? else {
                continue;
            };
            let later = match &best {
                None => true,
                Some((_, current)) => base != *current && self.git.is_ancestor(current, &base)?,
            };
            if later {
                best = Some((trunk, base));
            }
        }
        Ok(best.map(|(trunk, base)| self.derived(trunk, base)))
    }

    /// Whether `branch`'s former tip `version` is still a version of it, rather than a commit it was reset away from:
    /// the branch moved forward from it, was amended or rebased off it, or still carries a commit with the same patch.
    fn is_former_version(&self, branch: &Branch, version: &str) -> Result<bool, Error> {
        if self.git.is_ancestor(version, &branch.tip)? {
            return Ok(true);
        }
        let left_by = branch
            .former
            .iter()
            .find(|former| former.id == version)
            .map(|former| former.left_by.as_str());
        if left_by.is_some_and(|message| {
            message.starts_with("commit (amend)") || message.starts_with("rebase")
        }) {
            return Ok(true);
        }
        let current = self.git.commits_excluding(&branch.tip, &self.trunk_tips)?;
        let mut hidden = self.trunk_tips.clone();
        hidden.push(branch.tip.clone());
        let left_behind = self.git.commits_excluding(version, &hidden)?;
        if current.is_empty() || left_behind.is_empty() {
            return Ok(false);
        }
        let current: HashSet<String> = self.git.patch_ids(&current)?.into_values().collect();
        Ok(self
            .git
            .patch_ids(&left_behind)?
            .values()
            .any(|patch| current.contains(patch)))
    }

    /// Whichever of `current` and `candidate` is later in history; `current` if they're unrelated.
    fn later(&self, current: Option<String>, candidate: String) -> Result<String, Error> {
        Ok(match current {
            Some(current)
                if current == candidate || !self.git.is_ancestor(&current, &candidate)? =>
            {
                current
            }
            _ => candidate,
        })
    }

    fn derived(&self, parent: &Branch, offshoot: String) -> Parent {
        Parent {
            name: parent.name.clone(),
            needs_restack: offshoot != parent.tip,
            offshoot,
            source: Source::Derived,
            contradicted: false,
            replaces: None,
        }
    }

    /// Applies the recorded parent, if any, over the `derived` one.
    fn reconcile(&self, name: &str, derived: Option<Parent>) -> Result<Option<Parent>, Error> {
        let Some(link) = self.metadata.links.get(name).map(|stored| &stored.value) else {
            return Ok(derived);
        };
        let tip = &self.branches[name].tip;
        let recorded_parent = self
            .branches
            .get(&link.parent)
            .filter(|parent| parent.name != name);
        let replaced = |derived: Option<Parent>| {
            derived.map(|parent| Parent {
                replaces: Some(link.parent.clone()),
                ..parent
            })
        };
        let Some(parent) = recorded_parent else {
            return Ok(replaced(derived));
        };

        let version = self.contained_version(tip, parent, &link.offshoot)?;
        let consistent = match &version {
            Some(version) => !self.moved_leafward_of(version, derived.as_ref(), &parent.name)?,
            None => false,
        };
        let source = if link.pinned {
            Source::Pinned
        } else {
            Source::Recorded
        };
        if consistent || link.pinned {
            let offshoot = version.unwrap_or_else(|| link.offshoot.clone());
            return Ok(Some(Parent {
                name: parent.name.clone(),
                needs_restack: offshoot != parent.tip,
                offshoot,
                source,
                contradicted: !consistent,
                replaces: None,
            }));
        }
        Ok(replaced(derived))
    }

    /// The latest version of `parent` that `tip` contains: its current tip, the recorded offshoot, or (for trunks,
    /// which only move forward) the merge base.
    fn contained_version(
        &self,
        tip: &str,
        parent: &Branch,
        offshoot: &str,
    ) -> Result<Option<String>, Error> {
        let mut versions = vec![parent.tip.clone(), offshoot.to_owned()];
        if self.is_trunk(&parent.name) {
            versions.extend(self.git.merge_base(tip, &parent.tip)?);
        }
        let mut latest: Option<String> = None;
        for version in versions {
            if !self.git.is_ancestor(&version, tip)? {
                continue;
            }
            let later = match &latest {
                None => true,
                Some(current) => *current != version && self.git.is_ancestor(current, &version)?,
            };
            if later {
                latest = Some(version);
            }
        }
        Ok(latest)
    }

    /// Whether the branch now sits on a derived parent strictly leafward of `version` (e.g. rebased onto a child of
    /// its recorded parent with plain git).
    fn moved_leafward_of(
        &self,
        version: &str,
        derived: Option<&Parent>,
        recorded: &str,
    ) -> Result<bool, Error> {
        let Some(derived) =
            derived.filter(|derived| derived.name != recorded && !self.is_trunk(&derived.name))
        else {
            return Ok(false);
        };
        let derived_tip = &self.branches[&derived.name].tip;
        Ok(derived_tip != version && self.git.is_ancestor(version, derived_tip)?)
    }
}

/// Oldest first: branches without a reflog count as oldest; ties go by name.
fn age(branch: &Branch) -> (i64, &str) {
    (branch.created.unwrap_or(i64::MIN), &branch.name)
}

/// Breaks every cycle: first by replacing a recorded parent (unpinned first) with the derived one, and if a cycle is
/// made only of derived parents, by detaching one member so it shows as unattached rather than looping.
fn break_cycles(
    resolved: &mut BTreeMap<String, Resolved>,
    derived: &BTreeMap<String, Option<Parent>>,
) {
    let mut replaced: BTreeSet<String> = BTreeSet::new();
    while let Some(cycle) = find_cycle(resolved) {
        let pinned = |name: &String| {
            resolved[name]
                .parent
                .as_ref()
                .is_some_and(|parent| parent.source == Source::Pinned)
        };
        let replaceable = |name: &&String| !replaced.contains(*name);
        let victim = cycle
            .iter()
            .filter(replaceable)
            .find(|name| !pinned(name))
            .or_else(|| cycle.iter().find(replaceable))
            .cloned();
        let Some(victim) = victim else {
            let entry = resolved
                .get_mut(&cycle[0])
                .expect("cycle members are resolved");
            entry.parent = None;
            continue;
        };
        let entry = resolved
            .get_mut(&victim)
            .expect("cycle members are resolved");
        let replaces = entry.parent.as_ref().map(|parent| parent.name.clone());
        entry.parent = derived[&victim]
            .clone()
            .map(|parent| Parent { replaces, ..parent });
        replaced.insert(victim);
    }
}

fn find_cycle(resolved: &BTreeMap<String, Resolved>) -> Option<Vec<String>> {
    let mut cleared = BTreeSet::new();
    for start in resolved.keys() {
        let mut path: Vec<&String> = Vec::new();
        let mut current = Some(start);
        while let Some(name) = current {
            if cleared.contains(name) {
                break;
            }
            if let Some(position) = path.iter().position(|seen| *seen == name) {
                return Some(
                    path[position..]
                        .iter()
                        .map(|name| (*name).clone())
                        .collect(),
                );
            }
            path.push(name);
            current = resolved
                .get(name)
                .and_then(|entry| entry.parent.as_ref())
                .map(|parent| &parent.name);
        }
        cleared.extend(path);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(parent: &str) -> Resolved {
        Resolved {
            tip: String::new(),
            role: Role::Branch,
            parent: Some(Parent {
                name: parent.into(),
                offshoot: String::new(),
                source: Source::Derived,
                needs_restack: false,
                contradicted: false,
                replaces: None,
            }),
        }
    }

    #[test]
    fn cycles_of_derived_parents_are_cut_rather_than_looping() {
        let mut resolved =
            BTreeMap::from([("a".to_owned(), entry("b")), ("b".to_owned(), entry("a"))]);
        let derived = resolved
            .iter()
            .map(|(name, entry)| (name.clone(), entry.parent.clone()))
            .collect();

        break_cycles(&mut resolved, &derived);

        assert!(find_cycle(&resolved).is_none());
        assert_eq!(
            resolved
                .values()
                .filter(|entry| entry.parent.is_none())
                .count(),
            1
        );
    }
}
