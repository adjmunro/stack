//! The push guard: a `pre-push` hook that checks every push, including plain `git push` by people or agents.

use super::*;

/// Marks a `pre-push` hook as ours.
const MARKER: &str = "# Installed by `stack guard install`";
/// Where an existing `pre-push` hook is kept while the guard runs in front of it.
const CHAINED: &str = "pre-push.before-stack";

impl Workspace {
    /// Branches pushes must not touch without a human's say-so: every root trunk, plus each `stack.protect` value
    /// in git config. Sorted.
    pub fn protected_branches(&self) -> Result<Vec<String>, Error> {
        let (_, resolution) = self.resolve()?;
        let mut protected: Vec<String> = resolution
            .branches
            .iter()
            .filter(|(_, entry)| entry.role == Role::Trunk)
            .map(|(name, _)| name.clone())
            .collect();
        protected.extend(self.git.config_values("stack.protect")?);
        protected.sort();
        protected.dedup();
        Ok(protected)
    }

    /// Checks the ref updates of a push, as a `pre-push` hook receives them (`<local ref> <local sha> <remote ref>
    /// <remote sha>` per line). Flags pushes to or deletions of protected branches, and branches pushed under a
    /// different name (including detached commits pushed to a branch).
    pub fn check_push(&self, lines: &str) -> Result<Vec<GuardViolation>, Error> {
        let protected = self.protected_branches()?;
        let current = self.current_branch()?;
        let mut violations = Vec::new();
        for line in lines.lines().filter(|line| !line.trim().is_empty()) {
            let fields: Vec<&str> = line.split_whitespace().collect();
            let [local_ref, local_sha, remote_ref, _] = fields.as_slice() else {
                return Err(Error::git(format!("unexpected pre-push line: {line}")));
            };
            let Some(remote) = remote_ref.strip_prefix("refs/heads/") else {
                continue;
            };
            let deleting = local_sha.chars().all(|c| c == '0');
            if protected.iter().any(|branch| branch == remote) {
                violations.push(GuardViolation::Protected {
                    branch: remote.into(),
                    deleting,
                });
                continue;
            }
            if deleting {
                continue;
            }
            let local = match *local_ref {
                "HEAD" => current.clone(),
                other => other.strip_prefix("refs/heads/").map(str::to_owned),
            };
            if local.as_deref() != Some(remote) {
                let local = local.unwrap_or_else(|| (*local_ref).to_owned());
                violations.push(GuardViolation::NameMismatch {
                    local,
                    remote: remote.into(),
                });
            }
        }
        Ok(violations)
    }

    /// Installs the guard as this repository's `pre-push` hook, running `stack_binary guard check-push`. An existing
    /// hook of someone else's is kept and runs after the guard's checks pass. Reinstalling updates the hook.
    ///
    /// # Errors
    /// [`Error::Git`] if the hooks directory can't be written.
    pub fn install_guard(&self, stack_binary: &Path) -> Result<GuardInstalled, Error> {
        let hooks = self.git.hooks_dir()?;
        std::fs::create_dir_all(&hooks).map_err(Error::git)?;
        let hook = hooks.join("pre-push");
        let existing = std::fs::read_to_string(&hook).ok();
        let mut chained = hooks.join(CHAINED).exists();
        if existing
            .as_deref()
            .is_some_and(|script| !script.contains(MARKER))
        {
            if chained {
                return Err(Error::git(format!(
                    "both {} and {CHAINED} exist; merge them first",
                    hook.display()
                )));
            }
            std::fs::rename(&hook, hooks.join(CHAINED)).map_err(Error::git)?;
            chained = true;
        }
        let binary = stack_binary.to_string_lossy().replace('\'', r"'\''");
        let script = format!(
            "#!/bin/sh\n\
             {MARKER}: checks every push. Remove with `stack guard uninstall`.\n\
             input=$(cat)\n\
             printf '%s\\n' \"$input\" | '{binary}' guard check-push \"$1\" \"$2\" || exit 1\n\
             chained=\"$(dirname \"$0\")/{CHAINED}\"\n\
             if [ -x \"$chained\" ]; then\n    printf '%s\\n' \"$input\" | \"$chained\" \"$@\" || exit 1\nfi\n\
             exit 0\n"
        );
        std::fs::write(&hook, script).map_err(Error::git)?;
        make_executable(&hook)?;
        Ok(GuardInstalled { hook, chained })
    }

    /// Removes the guard, restoring any hook it was chained in front of. Returns whether it was installed.
    pub fn uninstall_guard(&self) -> Result<bool, Error> {
        let hooks = self.git.hooks_dir()?;
        let hook = hooks.join("pre-push");
        if !std::fs::read_to_string(&hook).is_ok_and(|script| script.contains(MARKER)) {
            return Ok(false);
        }
        std::fs::remove_file(&hook).map_err(Error::git)?;
        if hooks.join(CHAINED).exists() {
            std::fs::rename(hooks.join(CHAINED), &hook).map_err(Error::git)?;
        }
        Ok(true)
    }

    /// Whether the guard is this repository's `pre-push` hook.
    pub fn guard_installed(&self) -> Result<bool, Error> {
        let hook = self.git.hooks_dir()?.join("pre-push");
        Ok(std::fs::read_to_string(hook).is_ok_and(|script| script.contains(MARKER)))
    }
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<(), Error> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).map_err(Error::git)
}

#[cfg(not(unix))]
fn make_executable(_: &Path) -> Result<(), Error> {
    Ok(())
}
