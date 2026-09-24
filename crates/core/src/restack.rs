//! Plans a restack: replays each branch's own commits (those after its offshoot) onto its parent's current tip,
//! parents before children. Merges run on trees with `git merge-tree`, so nothing touches the index or working tree.
//!
//! In [`Mode::Apply`] replayed commits are written (as unreferenced objects until
//! [`crate::Workspace::restack`] moves the branches); in [`Mode::Preview`] only the merged trees are, and no commit is
//! created or signed.

use std::collections::{BTreeMap, VecDeque};

use crate::git::{GitRepo, Merge};
use crate::metadata::Link;
use crate::resolve::Resolution;
use crate::{Conflict, Error, Role, Source};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    /// Write the replayed commits.
    Apply,
    /// Only merge trees: no commits are created or signed.
    Preview,
}

/// A branch the plan moves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Move {
    pub branch: String,
    pub onto: String,
    pub old: String,
    /// The new tip; `None` in [`Mode::Preview`].
    pub new: Option<String>,
    pub replayed: usize,
    pub dropped: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Plan {
    pub moves: Vec<Move>,
    /// The parent to record for every branch planned (moved or already up to date).
    pub links: BTreeMap<String, Link>,
    /// Every branch that hit a conflict. Each is left out of the plan with everything leafward of it.
    pub conflicts: Vec<Conflict>,
    /// Branches left out because a branch rootward of them conflicted, sorted.
    pub blocked: Vec<String>,
}

/// Where a branch's commits land: a commit (when applying) and its tree.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Base {
    commit: Option<String>,
    tree: String,
}

/// Plans restacking `target` and every branch leafward of it. A trunk target itself stays put.
pub(crate) fn plan(
    git: &dyn GitRepo,
    resolution: &Resolution,
    target: &str,
    mode: Mode,
) -> Result<Plan, Error> {
    let mut children: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (name, entry) in &resolution.branches {
        if let Some(parent) = &entry.parent {
            children.entry(parent.name.as_str()).or_default().push(name);
        }
    }
    let mut plan = Plan::default();
    let mut new_bases: BTreeMap<String, Base> = BTreeMap::new();
    let mut queue: VecDeque<&str> = if resolution.branches[target].role == Role::Trunk {
        children
            .get(target)
            .into_iter()
            .flatten()
            .copied()
            .collect()
    } else {
        VecDeque::from([target])
    };
    // Parents first: each branch is queued only after its parent has been planned.
    while let Some(branch) = queue.pop_front() {
        let entry = &resolution.branches[branch];
        let Some(parent) = &entry.parent else {
            continue;
        };
        let onto = match new_bases.get(&parent.name) {
            Some(base) => base.clone(),
            None => {
                let tip = &resolution.branches[&parent.name].tip;
                Base {
                    commit: Some(tip.clone()),
                    tree: git.commit(tip)?.tree,
                }
            }
        };
        if !git.is_ancestor(&parent.offshoot, &entry.tip)? {
            return Err(Error::OffshootNotInHistory {
                branch: branch.into(),
                parent: parent.name.clone(),
            });
        }
        let up_to_date = onto.commit.as_deref() == Some(parent.offshoot.as_str());
        if !up_to_date {
            match replay(
                git,
                branch,
                &entry.tip,
                &parent.offshoot,
                onto.clone(),
                mode,
            )? {
                Ok((base, replayed, dropped)) => {
                    plan.moves.push(Move {
                        branch: branch.to_owned(),
                        onto: parent.name.clone(),
                        old: entry.tip.clone(),
                        new: base.commit.clone(),
                        replayed,
                        dropped,
                    });
                    new_bases.insert(branch.to_owned(), base);
                }
                Err(mut conflict) => {
                    conflict.onto = parent.name.clone();
                    plan.conflicts.push(conflict);
                    plan.blocked.extend(descendants(&children, branch));
                    continue;
                }
            }
        }
        if let Some(offshoot) = onto.commit {
            let pinned = parent.source == Source::Pinned;
            plan.links.insert(
                branch.to_owned(),
                Link {
                    parent: parent.name.clone(),
                    offshoot,
                    pinned,
                },
            );
        }
        queue.extend(children.get(branch).into_iter().flatten());
    }
    plan.blocked.sort();
    Ok(plan)
}

/// Replays the commits after `offshoot` up to `tip` onto `onto`. Returns where they landed and how many commits were
/// replayed and dropped (those whose changes are already in `onto`), or the first conflict.
fn replay(
    git: &dyn GitRepo,
    branch: &str,
    tip: &str,
    offshoot: &str,
    onto: Base,
    mode: Mode,
) -> Result<Result<(Base, usize, usize), Conflict>, Error> {
    let mut commits = git.commits_excluding(tip, &[offshoot.to_owned()])?;
    commits.reverse();
    let mut current = onto;
    let (mut replayed, mut dropped) = (0, 0);
    for commit in commits {
        let info = git.commit(&commit)?;
        let [original_parent] = info.parents.as_slice() else {
            return Err(Error::MergeCommit {
                branch: branch.into(),
                commit,
            });
        };
        let parent_tree = git.commit(original_parent)?.tree;
        match git.merge_trees(&parent_tree, &current.tree, &info.tree)? {
            Merge::Clean { tree } => {
                let was_empty = info.tree == parent_tree;
                if !was_empty && tree == current.tree {
                    dropped += 1;
                    continue;
                }
                let commit = match (mode, &current.commit) {
                    (Mode::Apply, Some(parent)) => Some(git.copy_commit(&commit, &tree, parent)?),
                    _ => None,
                };
                current = Base { commit, tree };
                replayed += 1;
            }
            Merge::Conflicted { paths } => {
                return Ok(Err(Conflict {
                    branch: branch.into(),
                    commit,
                    summary: info.summary,
                    paths,
                    onto: String::new(),
                    offshoot: offshoot.into(),
                }));
            }
        }
    }
    Ok(Ok((current, replayed, dropped)))
}

/// Every branch leafward of `branch`.
fn descendants(children: &BTreeMap<&str, Vec<&str>>, branch: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut queue: VecDeque<&str> = children
        .get(branch)
        .into_iter()
        .flatten()
        .copied()
        .collect();
    while let Some(child) = queue.pop_front() {
        found.push(child.to_owned());
        queue.extend(children.get(child).into_iter().flatten());
    }
    found
}
