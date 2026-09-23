//! Plans a restack: replays each branch's own commits (those after its offshoot) onto its parent's current tip, in
//! memory, parents before children. Nothing is written but unreferenced objects; [`crate::Workspace::restack`]
//! applies the plan in one journalled transaction.

use std::collections::{BTreeMap, VecDeque};

use crate::git::{GitRepo, Merge};
use crate::metadata::Link;
use crate::resolve::Resolution;
use crate::{Conflict, Error, Role, Source};

/// A branch the plan moves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Move {
    pub branch: String,
    pub onto: String,
    pub old: String,
    pub new: String,
    pub replayed: usize,
    pub dropped: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Plan {
    pub moves: Vec<Move>,
    /// The parent to record for every branch planned (moved or already up to date).
    pub links: BTreeMap<String, Link>,
    /// The first conflict found. Its branch and everything leafward of it are left out of the plan.
    pub conflict: Option<Conflict>,
}

/// Plans restacking `target` and every branch leafward of it. A trunk target itself stays put.
pub(crate) fn plan(
    git: &dyn GitRepo,
    resolution: &Resolution,
    target: &str,
) -> Result<Plan, Error> {
    let mut children: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (name, entry) in &resolution.branches {
        if let Some(parent) = &entry.parent {
            children.entry(parent.name.as_str()).or_default().push(name);
        }
    }
    let mut plan = Plan::default();
    let mut new_tips: BTreeMap<String, String> = BTreeMap::new();
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
        let onto = new_tips
            .get(&parent.name)
            .unwrap_or(&resolution.branches[&parent.name].tip)
            .clone();
        if !git.is_ancestor(&parent.offshoot, &entry.tip)? {
            return Err(Error::OffshootNotInHistory {
                branch: branch.into(),
                parent: parent.name.clone(),
            });
        }
        plan.links.insert(
            branch.to_owned(),
            Link {
                parent: parent.name.clone(),
                offshoot: onto.clone(),
                pinned: parent.source == Source::Pinned,
            },
        );
        if parent.offshoot != onto {
            match replay(git, branch, &entry.tip, &parent.offshoot, &onto)? {
                Ok((new, replayed, dropped)) => {
                    new_tips.insert(branch.to_owned(), new.clone());
                    plan.moves.push(Move {
                        branch: branch.to_owned(),
                        onto: parent.name.clone(),
                        old: entry.tip.clone(),
                        new,
                        replayed,
                        dropped,
                    });
                }
                Err(mut conflict) => {
                    conflict.onto = parent.name.clone();
                    plan.links.remove(branch);
                    plan.conflict.get_or_insert(conflict);
                    continue;
                }
            }
        }
        queue.extend(children.get(branch).into_iter().flatten());
    }
    Ok(plan)
}

/// Replays the commits after `offshoot` up to `tip` onto `onto`. Returns the new tip and how many commits were
/// replayed and dropped (those whose changes are already in `onto`), or the first conflict.
fn replay(
    git: &dyn GitRepo,
    branch: &str,
    tip: &str,
    offshoot: &str,
    onto: &str,
) -> Result<Result<(String, usize, usize), Conflict>, Error> {
    let mut commits = git.commits_excluding(tip, &[offshoot.to_owned()])?;
    commits.reverse();
    let mut current = onto.to_owned();
    let (mut replayed, mut dropped) = (0, 0);
    for commit in commits {
        let info = git.commit(&commit)?;
        let [original_parent] = info.parents.as_slice() else {
            return Err(Error::MergeCommit {
                branch: branch.into(),
                commit,
            });
        };
        match git.merge_trees(original_parent, &current, &commit)? {
            Merge::Clean { tree } => {
                let was_empty = info.tree == git.commit(original_parent)?.tree;
                if !was_empty && tree == git.commit(&current)?.tree {
                    dropped += 1;
                    continue;
                }
                current = git.copy_commit(&commit, &tree, &current)?;
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
