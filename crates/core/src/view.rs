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

/// Every trunk with the branches stacked on it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Tree {
    /// One root per trunk, sorted by name.
    pub trunks: Vec<Node>,
    /// Tracked branches whose parent is no longer a trunk or tracked branch, sorted by name.
    pub orphans: Vec<Node>,
}

/// A branch in a [`Tree`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Node {
    pub name: String,
    /// The branch tip, or `None` if the branch no longer exists.
    pub commit: Option<String>,
    /// Whether `HEAD` is on this branch.
    pub current: bool,
    /// Tracked branches stacked directly on this one, sorted by name.
    pub children: Vec<Node>,
}

/// Whether a mutation changed anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Changed,
    /// The repository already matched the request.
    Unchanged,
}
