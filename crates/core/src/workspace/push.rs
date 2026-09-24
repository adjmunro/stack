//! Lines and pushing them.

use super::*;

impl Workspace {
    /// The branches in `branch`'s line under `scope`, rootward first: what push covers. Never includes a trunk.
    ///
    /// # Errors
    /// [`Error::UnknownBranch`].
    pub fn line(&self, branch: &str, scope: Scope) -> Result<Vec<String>, Error> {
        let (_, resolution) = self.resolve()?;
        if !resolution.branches.contains_key(branch) {
            return Err(Error::UnknownBranch {
                name: branch.into(),
            });
        }
        Ok(crate::line::line(&resolution, branch, scope))
    }

    /// Pushes `branch`'s line (see [`Self::line`]) to the same names on `remote`, with a lease on each so a remote
    /// branch changed since the last fetch is never overwritten. Branches without an upstream get `remote`'s; others
    /// keep theirs (so pushing a backup to a second remote changes nothing locally). Without `remote`: `branch`'s
    /// upstream remote, else `origin`, else the only remote.
    ///
    /// # Errors
    /// - [`Error::UnknownBranch`], [`Error::NoRemote`], or [`Error::UnknownRemote`].
    /// - [`Error::UpstreamMismatch`] if a branch tracks a remote branch with a different name.
    /// - [`Error::Git`] if the remote can't be reached. Rejected branches are reported in the result instead.
    pub fn push(&self, branch: &str, scope: Scope, remote: Option<&str>) -> Result<Pushed, Error> {
        let branches = self.line(branch, scope)?;
        let remotes = self.git.remotes()?;
        let remote = match remote {
            Some(remote) if remotes.iter().any(|known| known == remote) => remote.to_owned(),
            Some(remote) => {
                return Err(Error::UnknownRemote {
                    name: remote.into(),
                });
            }
            None => self
                .git
                .config_value(&format!("branch.{branch}.remote"))?
                .filter(|upstream| remotes.contains(upstream))
                .or_else(|| remotes.iter().find(|known| *known == "origin").cloned())
                .or_else(|| (remotes.len() == 1).then(|| remotes[0].clone()))
                .ok_or(Error::NoRemote)?,
        };
        let mut refs = Vec::new();
        let mut without_upstream = Vec::new();
        for name in &branches {
            match self.git.config_value(&format!("branch.{name}.merge"))? {
                Some(merge) if merge != format!("refs/heads/{name}") => {
                    let upstream_remote = self
                        .git
                        .config_value(&format!("branch.{name}.remote"))?
                        .unwrap_or_default();
                    let upstream = format!(
                        "{upstream_remote}/{}",
                        merge.trim_start_matches("refs/heads/")
                    );
                    return Err(Error::UpstreamMismatch {
                        branch: name.clone(),
                        upstream,
                    });
                }
                Some(_) => {}
                None => without_upstream.push(name.clone()),
            }
            let lease = self
                .git
                .ref_value(&format!("refs/remotes/{remote}/{name}"))?;
            refs.push(PushRef {
                branch: name.clone(),
                lease,
            });
        }
        let statuses = if refs.is_empty() {
            Vec::new()
        } else {
            self.git.push(&remote, &refs)?
        };
        for status in &statuses {
            if status.outcome != PushOutcome::Rejected && without_upstream.contains(&status.branch)
            {
                self.git.set_upstream(&status.branch, &remote)?;
            }
        }
        let branches = branches
            .into_iter()
            .map(|name| {
                let status = statuses.iter().find(|status| status.branch == name);
                PushedBranch {
                    outcome: status.map_or(PushOutcome::Rejected, |status| status.outcome),
                    summary: status.map_or_else(
                        || "not reported by git".to_owned(),
                        |status| status.summary.clone(),
                    ),
                    name,
                }
            })
            .collect();
        Ok(Pushed { remote, branches })
    }
}
