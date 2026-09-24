//! Amending commits other than HEAD.

use super::*;

impl Workspace {
    /// Applies the staged changes to the commit `revision` names, anywhere in the current branch's stack (its own
    /// commits, or a branch rootward of it up to the trunk): that commit is rewritten, the rest of its branch replayed
    /// on top, and every branch leafward of it restacked, the current one included. The index then matches the new
    /// tip; the working tree isn't touched, so unstaged changes stay as they are. One journalled command.
    ///
    /// # Errors
    /// - [`Error::DetachedHead`] or [`Error::NothingStaged`].
    /// - [`Error::UnknownRevision`], or [`Error::NotInStack`] if the commit isn't in the current stack.
    /// - [`Error::MergeCommit`] for a merge commit.
    /// - [`Error::AmendConflict`] if the change, or anything replayed after it, conflicts; nothing is changed.
    pub fn amend_into(&self, revision: &str) -> Result<Amended, Error> {
        let Head::Branch {
            name: current,
            commit: Some(head),
        } = self.git.head()?
        else {
            return Err(Error::DetachedHead);
        };
        let staged = self.git.write_index_tree()?;
        let head_tree = self.git.commit(&head)?.tree;
        if staged == head_tree {
            return Err(Error::NothingStaged);
        }
        let target = self.git.resolve_commit(revision)?;
        self.amend_change(&current, &target, &head_tree, &staged, revision)
    }

    /// Applies the change from tree `base` to tree `changed` to commit `target` in `current`'s stack, as
    /// [`Self::amend_into`] does; `revision` names the target in errors.
    pub(super) fn amend_change(
        &self,
        current: &str,
        target: &str,
        base: &str,
        changed: &str,
        revision: &str,
    ) -> Result<Amended, Error> {
        let (metadata, resolution) = self.resolve()?;
        let owner = self
            .owner_of(&resolution, current, target)?
            .ok_or_else(|| Error::NotInStack {
                revision: revision.into(),
            })?;
        let info = self.git.commit(target)?;
        let [parent] = info.parents.as_slice() else {
            return Err(Error::MergeCommit {
                branch: owner,
                commit: target.into(),
            });
        };

        // The change, applied to the target commit.
        let conflict = |paths: Vec<String>| Error::AmendConflict {
            commit: target.into(),
            paths,
        };
        let tree = match self.git.merge_trees(base, &info.tree, changed)? {
            Merge::Clean { tree } => tree,
            Merge::Conflicted { paths } => return Err(conflict(paths)),
        };
        let rewritten = self.git.copy_commit(target, &tree, parent)?;

        // The rest of the owner's commits, then everything leafward of it.
        let owner_tip = resolution.branches[&owner].tip.clone();
        let onto = crate::restack::Base {
            commit: Some(rewritten.clone()),
            tree,
        };
        let owner_new = match crate::restack::replay(
            &*self.git,
            &owner,
            &owner_tip,
            target,
            onto,
            Mode::Apply,
        )? {
            Ok((base, _, _)) => base.commit.expect("applied replays write commits"),
            Err(replayed) => return Err(conflict(replayed.paths)),
        };
        let plan =
            crate::restack::plan_after(&*self.git, &resolution, &owner, &owner_new, Mode::Apply)?;
        if let Some(first) = plan.conflicts.first() {
            return Err(conflict(first.paths.clone()));
        }
        let new_head = plan
            .moves
            .iter()
            .find(|moved| moved.branch == current)
            .and_then(|moved| moved.new.clone())
            .expect("the current branch is the owner or leafward of it");
        let index = Checkout {
            worktree: None,
            from: self.git.write_index_tree()?,
            to: new_head,
            index_only: true,
        };
        let description = format!("amend {} into {}", &target[..7.min(target.len())], owner);
        let restacked = self.apply_plan(
            &metadata,
            &resolution,
            plan,
            &description,
            Some((current, index)),
        )?;
        Ok(Amended {
            commit: target.into(),
            rewritten,
            branch: owner,
            restacked,
        })
    }

    /// The branch in `current`'s stack (itself, or rootward up to the trunk) whose own commits include `commit`.
    fn owner_of(
        &self,
        resolution: &Resolution,
        current: &str,
        commit: &str,
    ) -> Result<Option<String>, Error> {
        for branch in resolution.lineage(current) {
            let entry = &resolution.branches[branch];
            let Some(parent) = &entry.parent else { break };
            if entry.role == Role::Trunk {
                break;
            }
            if self
                .git
                .commits_excluding(&entry.tip, std::slice::from_ref(&parent.offshoot))?
                .iter()
                .any(|own| own == commit)
            {
                return Ok(Some(branch.to_owned()));
            }
        }
        Ok(None)
    }
}
