//! Finding lost commits.

use super::*;

impl Workspace {
    /// Commits you've been on (per the `HEAD` and branch reflogs) that no ref reaches any more: e.g. the version
    /// before an amend, work left by `reset --hard`, or a deleted branch you had checked out. Only the newest commit of
    /// each lost chain is listed, newest sighting first, up to `limit`. Restore one with `git branch <name> <commit>`.
    pub fn lost(&self, limit: usize) -> Result<Vec<LostCommit>, Error> {
        let mut references = vec!["HEAD".to_owned()];
        references.extend(
            self.git
                .branches()?
                .into_iter()
                .map(|branch| format!("refs/heads/{}", branch.name)),
        );
        // The latest sighting of each commit.
        let mut sightings: BTreeMap<String, (String, String, i64)> = BTreeMap::new();
        for reference in &references {
            for entry in self.git.reflog(reference)? {
                for commit in [&entry.old, &entry.new] {
                    if commit.chars().all(|c| c == '0') {
                        continue;
                    }
                    let newer = sightings
                        .get(commit)
                        .is_none_or(|(_, _, time)| entry.time >= *time);
                    if newer {
                        sightings.insert(
                            commit.clone(),
                            (reference.clone(), entry.message.clone(), entry.time),
                        );
                    }
                }
            }
        }
        let tips = self.git.commit_tips()?;
        let candidates: Vec<String> = sightings
            .keys()
            .filter(|commit| !tips.contains(commit))
            .cloned()
            .collect();
        if candidates.is_empty() {
            return Ok(Vec::new());
        }
        // Lost: reachable from a sighting but from no ref, found in one walk.
        let lost: std::collections::BTreeSet<String> = self
            .git
            .reachable_excluding(&candidates, &tips)?
            .into_iter()
            .collect();
        // Newest of each chain: a lost sighting that no other lost sighting builds on.
        let mut found = Vec::new();
        for candidate in candidates
            .iter()
            .filter(|candidate| lost.contains(*candidate))
        {
            let mut superseded = false;
            for other in candidates
                .iter()
                .filter(|other| *other != candidate && lost.contains(*other))
            {
                if self.git.is_ancestor(candidate, other)? {
                    superseded = true;
                    break;
                }
            }
            if !superseded {
                let (seen_on, how, seen_at) = sightings[candidate].clone();
                let summary = self.git.commit(candidate)?.summary;
                found.push(LostCommit {
                    commit: candidate.clone(),
                    summary,
                    seen_on,
                    how,
                    seen_at,
                });
            }
        }
        found.sort_by(|a, b| {
            b.seen_at
                .cmp(&a.seen_at)
                .then_with(|| a.commit.cmp(&b.commit))
        });
        found.truncate(limit);
        Ok(found)
    }
}
