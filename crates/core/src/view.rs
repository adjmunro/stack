use serde::Serialize;

/// Summary of the repository's current state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Status {
    pub head: Head,
}

/// What `HEAD` points at. Commit ids are full hex SHAs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Head {
    /// On a local branch. `commit` is `None` when the branch has no commits yet.
    Branch {
        name: String,
        commit: Option<String>,
    },
    /// Detached at a commit.
    Detached { commit: String },
}
