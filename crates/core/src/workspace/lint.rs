//! Checking commit subjects against the repository's rules.

use super::*;

/// Conventional Commits: `type(scope)!: subject`.
const DEFAULT_PATTERN: &str =
    r"^(feat|fix|docs|style|refactor|perf|test|build|ci|chore|revert)(\([^)]+\))?!?: \S";
const DEFAULT_MAX_LENGTH: usize = 72;

impl Workspace {
    /// The rules subjects are checked against: `stack.lint.pattern` (a regex) and `stack.lint.maxLength` from git
    /// config, defaulting to Conventional Commits and 72 characters.
    ///
    /// # Errors
    /// [`Error::InvalidLintRule`] if the config values don't parse.
    pub fn lint_rules(&self) -> Result<LintRules, Error> {
        let pattern = self
            .git
            .config_value("stack.lint.pattern")?
            .unwrap_or_else(|| DEFAULT_PATTERN.to_owned());
        let max_length = match self.git.config_value("stack.lint.maxLength")? {
            Some(value) => value.parse().map_err(|_| Error::InvalidLintRule {
                rule: format!("stack.lint.maxLength = {value}"),
            })?,
            None => DEFAULT_MAX_LENGTH,
        };
        Ok(LintRules {
            pattern,
            max_length,
        })
    }

    /// Checks the subject of each of `branch`'s own commits (as in [`Self::review`]) against [`Self::lint_rules`].
    /// Returns only the commits with problems, newest first.
    ///
    /// # Errors
    /// [`Error::UnknownBranch`], or [`Error::InvalidLintRule`].
    pub fn lint(&self, branch: &str) -> Result<Vec<LintFinding>, Error> {
        let rules = self.lint_rules()?;
        let pattern =
            regex::Regex::new(&rules.pattern).map_err(|error| Error::InvalidLintRule {
                rule: format!("stack.lint.pattern: {error}"),
            })?;
        let mut findings = Vec::new();
        for commit in self.review(branch)? {
            let mut problems = Vec::new();
            let length = commit.summary.chars().count();
            if length > rules.max_length {
                problems.push(LintProblem::TooLong {
                    length,
                    max: rules.max_length,
                });
            }
            if !pattern.is_match(&commit.summary) {
                problems.push(LintProblem::NoMatch {
                    pattern: rules.pattern.clone(),
                });
            }
            if !problems.is_empty() {
                findings.push(LintFinding {
                    commit: commit.commit,
                    summary: commit.summary,
                    problems,
                });
            }
        }
        Ok(findings)
    }
}
