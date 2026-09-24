//! The `Forge` port (code-review hosts) and its `gh` CLI adapter.

use std::path::PathBuf;
use std::process::Command;

use serde::Deserialize;

use crate::{Environment, Error};

/// A pull request as the forge reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PullRequest {
    pub number: u64,
    pub url: String,
    pub base: String,
    /// E.g. `OPEN`, `CLOSED`, `MERGED`.
    pub state: String,
}

/// All code-review host access goes through this trait.
pub(crate) trait Forge: Send + Sync {
    /// The most recent pull request from `branch`, or `None` if there is none.
    fn find(&self, branch: &str) -> Result<Option<PullRequest>, Error>;

    /// Opens a pull request from `branch` into `base`, titled and described from its commits.
    fn create(&self, branch: &str, base: &str, draft: bool) -> Result<PullRequest, Error>;

    /// Points pull request `number` at `base`.
    fn retarget(&self, number: u64, base: &str) -> Result<(), Error>;
}

/// [`Forge`] backed by GitHub's `gh` CLI, run in the repository's worktree with the user's `gh` login.
pub(crate) struct GhForge {
    pub workdir: PathBuf,
    pub environment: Environment,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhPullRequest {
    number: u64,
    url: String,
    base_ref_name: String,
    state: String,
}

impl GhForge {
    fn gh(&self, args: &[&str]) -> Command {
        let mut command = Command::new("gh");
        if let Environment::Exactly(variables) = &self.environment {
            command
                .env_clear()
                .envs(variables.iter().map(|(key, value)| (key, value)));
        }
        command.current_dir(&self.workdir).args(args);
        command
    }

    fn run(&self, args: &[&str]) -> Result<std::process::Output, Error> {
        self.gh(args)
            .output()
            .map_err(|error| Error::Forge(format!("can't run gh: {error}")))
    }

    fn run_ok(&self, args: &[&str]) -> Result<String, Error> {
        let output = self.run(args)?;
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(Error::Forge(format!(
                "gh {}: {}",
                args.join(" "),
                stderr.trim()
            )))
        }
    }
}

impl Forge for GhForge {
    fn find(&self, branch: &str) -> Result<Option<PullRequest>, Error> {
        let output = self.run(&[
            "pr",
            "view",
            branch,
            "--json",
            "number,url,baseRefName,state",
        ])?;
        if !output.status.success() {
            // gh exits non-zero when the branch has no pull request.
            return Ok(None);
        }
        let found: GhPullRequest = serde_json::from_slice(&output.stdout)
            .map_err(|error| Error::Forge(format!("unexpected gh output: {error}")))?;
        Ok(Some(PullRequest {
            number: found.number,
            url: found.url,
            base: found.base_ref_name,
            state: found.state,
        }))
    }

    fn create(&self, branch: &str, base: &str, draft: bool) -> Result<PullRequest, Error> {
        let mut args = vec!["pr", "create", "--head", branch, "--base", base, "--fill"];
        if draft {
            args.push("--draft");
        }
        self.run_ok(&args)?;
        self.find(branch)?
            .ok_or_else(|| Error::Forge(format!("gh created no pull request for {branch}")))
    }

    fn retarget(&self, number: u64, base: &str) -> Result<(), Error> {
        self.run_ok(&["pr", "edit", &number.to_string(), "--base", base])
            .map(|_| ())
    }
}
