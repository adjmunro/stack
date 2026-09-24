//! Comparing what two branches change, without the noise of different bases.

use super::*;

impl Workspace {
    /// Compares the changes `a` and `b` make, each against its own base: its offshoot, or where the two branches
    /// meet if it has no parent. Also replays `b`'s own changes onto `a`'s base in memory, so the trees can be
    /// diffed directly (`git diff <a tip> <b_on_a_base>`) to show only how the changes differ.
    ///
    /// # Errors
    /// [`Error::UnknownBranch`], or [`Error::Unrelated`] if the branches share no history.
    pub fn delta(&self, a: &str, b: &str) -> Result<Delta, Error> {
        let (_, resolution) = self.resolve()?;
        let range = |name: &str| -> Result<CommitRange, Error> {
            let entry = resolution
                .branches
                .get(name)
                .ok_or_else(|| Error::UnknownBranch { name: name.into() })?;
            let base = match &entry.parent {
                Some(parent) => parent.offshoot.clone(),
                None => {
                    let other = if name == a { b } else { a };
                    let other_tip = &resolution
                        .branches
                        .get(other)
                        .ok_or_else(|| Error::UnknownBranch { name: other.into() })?
                        .tip;
                    self.git
                        .merge_base(&entry.tip, other_tip)?
                        .ok_or_else(|| Error::Unrelated {
                            branch: a.into(),
                            parent: b.into(),
                        })?
                }
            };
            Ok(CommitRange {
                branch: name.into(),
                base,
                tip: entry.tip.clone(),
            })
        };
        let (a, b) = (range(a)?, range(b)?);
        let tree = |commit: &str| -> Result<String, Error> { Ok(self.git.commit(commit)?.tree) };
        let (b_on_a_base, conflicts) =
            match self
                .git
                .merge_trees(&tree(&b.base)?, &tree(&a.base)?, &tree(&b.tip)?)?
            {
                Merge::Clean { tree } => (Some(tree), Vec::new()),
                Merge::Conflicted { paths } => (None, paths),
            };
        Ok(Delta {
            a,
            b,
            b_on_a_base,
            conflicts,
        })
    }
}
