//! Proposing branches for review: pull requests.

use super::*;

impl Workspace {
    /// Pushes `branch`'s line (see [`Self::push`]), then makes sure each pushed branch has an open pull request
    /// targeting its parent: creating one (titled and described from its commits) or retargeting one whose base has
    /// drifted, e.g. after a move. Branches whose push was rejected, or whose pull request is merged or closed, are
    /// left alone.
    ///
    /// # Errors
    /// As [`Self::push`]; [`Error::Forge`] if `gh` fails.
    pub fn propose(
        &self,
        branch: &str,
        scope: Scope,
        remote: Option<&str>,
        draft: bool,
    ) -> Result<Proposed, Error> {
        let pushed = self.push(branch, scope, remote)?;
        let (_, resolution) = self.resolve()?;
        let mut pull_requests = Vec::new();
        for pushed_branch in &pushed.branches {
            let name = &pushed_branch.name;
            let base = resolution.branches[name]
                .parent
                .as_ref()
                .map(|parent| parent.name.clone())
                .unwrap_or_default();
            let proposal = |action, found: Option<&PullRequest>| ProposedBranch {
                branch: name.clone(),
                base: base.clone(),
                number: found.map(|found| found.number),
                url: found.map(|found| found.url.clone()),
                action,
            };
            let skipped = |reason: String| ProposalAction::Skipped { reason };
            if pushed_branch.outcome == PushOutcome::Rejected {
                pull_requests.push(proposal(skipped("its push was rejected".into()), None));
                continue;
            }
            if base.is_empty() {
                pull_requests.push(proposal(skipped("it has no parent to target".into()), None));
                continue;
            }
            if self
                .git
                .ref_value(&format!("refs/remotes/{}/{base}", pushed.remote))?
                .is_none()
            {
                let reason = format!("its parent {base} isn't on {}", pushed.remote);
                pull_requests.push(proposal(skipped(reason), None));
                continue;
            }
            let entry = match self.forge.find(name)? {
                None => {
                    let created = self.forge.create(name, &base, draft)?;
                    proposal(ProposalAction::Created, Some(&created))
                }
                Some(found) if found.state != "OPEN" => {
                    let reason = format!("its pull request is {}", found.state.to_lowercase());
                    proposal(skipped(reason), Some(&found))
                }
                Some(found) if found.base != base => {
                    self.forge.retarget(found.number, &base)?;
                    proposal(
                        ProposalAction::Retargeted {
                            from: found.base.clone(),
                        },
                        Some(&found),
                    )
                }
                Some(found) => proposal(ProposalAction::UpToDate, Some(&found)),
            };
            pull_requests.push(entry);
        }
        Ok(Proposed {
            pushed,
            pull_requests,
        })
    }
}
