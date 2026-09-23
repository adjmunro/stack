use serde::{Deserialize, Serialize};

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

/// Every trunk with the branches stacked on it, as resolved from the commit graph and recorded parents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Tree {
    /// One root per trunk, sorted by name.
    pub trunks: Vec<Node>,
    /// Branches that share no history with any trunk (or every branch, if there are no trunks), sorted by name.
    pub unattached: Vec<Node>,
}

/// A branch in a [`Tree`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Node {
    pub name: String,
    pub commit: String,
    pub role: Role,
    /// Whether `HEAD` is on this branch.
    pub current: bool,
    /// `None` for trunks and unattached roots.
    pub parent: Option<Parent>,
    /// Branches whose parent is this one, sorted by name.
    pub children: Vec<Node>,
}

/// What a branch does in a stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// Marked with `stack trunk add`; has no parent and is never rebased or pushed by `stack`.
    Trunk,
    /// Marked with `stack trunk add` while stacked on a regular branch; a base for branches leafward of it.
    Limb,
    Branch,
}

/// A branch's resolved parent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Parent {
    pub name: String,
    /// The version of the parent this branch is based on.
    pub offshoot: String,
    pub source: Source,
    /// The parent has moved on from `offshoot`, so a restack would change this branch.
    pub needs_restack: bool,
    /// Pinned, but the commit graph disagrees: the branch no longer contains the parent, or sits on another branch
    /// leafward of it.
    pub contradicted: bool,
    /// A recorded, unpinned parent that the commit graph contradicted and this one replaced.
    pub replaces: Option<String>,
}

/// Where a [`Parent`] came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// Set with `stack pin`; never changed automatically.
    Pinned,
    /// Recorded by `stack`, and still consistent with the commit graph.
    Recorded,
    /// Worked out from the commit graph.
    Derived,
}

/// Whether a mutation changed anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Changed,
    /// The repository already matched the request.
    Unchanged,
}
