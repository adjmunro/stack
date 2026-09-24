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

/// A recorded `stack` mutation, for the op log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Operation {
    pub id: i64,
    pub kind: OperationKind,
    /// What was run, e.g. `pin feat/b on feat/a`.
    pub description: String,
    /// For an undo or redo, the command it applied to.
    pub target: Option<i64>,
    pub state: OperationState,
    /// For a command, whether it is currently undone.
    pub undone: bool,
    /// Seconds since the epoch.
    pub started_at: i64,
    /// Every ref the operation changed, sorted by name.
    pub changes: Vec<RefChange>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationKind {
    Command,
    Undo,
    Redo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationState {
    /// Recorded but not finished: interrupted, or still running.
    Pending,
    Done,
    /// Nothing was changed.
    Failed,
}

/// One ref's change. `None` means the ref didn't (or doesn't) exist.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RefChange {
    pub name: String,
    pub old: Option<String>,
    pub new: Option<String>,
}

/// An interrupted operation found and resolved when the workspace was opened.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Recovered {
    pub operation: i64,
    pub description: String,
    pub outcome: RecoveryOutcome,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryOutcome {
    /// Every ref already had its new value; the operation is now marked done.
    Completed,
    /// Every ref still had its old value; the operation is now marked failed.
    RolledBack,
    /// Refs are a mix of old and new values, or neither. Left pending for a human to resolve.
    Inconsistent,
}

/// The result of [`crate::Workspace::restack`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Restacked {
    /// [`Outcome::Unchanged`] if every branch was already up to date and recorded.
    pub outcome: Outcome,
    /// Branches moved onto their parent's current tip, parents first.
    pub moved: Vec<Moved>,
    /// Branches that hit a conflict. Each was left as it was, with everything leafward of it; everything else was
    /// restacked.
    pub conflicts: Vec<Conflict>,
    /// Branches left as they were because a branch rootward of them conflicted, sorted.
    pub blocked: Vec<String>,
}

/// What [`crate::Workspace::check`] found a restack would do. Nothing is changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RestackPreview {
    /// Branches that would move cleanly, parents first.
    pub clean: Vec<PreviewedMove>,
    pub conflicts: Vec<Conflict>,
    /// Branches that can't be checked because a branch rootward of them conflicts, sorted.
    pub blocked: Vec<String>,
}

/// A branch a restack would move cleanly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PreviewedMove {
    pub name: String,
    pub onto: String,
    pub replayed: usize,
    pub dropped: usize,
}

/// A branch moved by a restack.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Moved {
    pub name: String,
    /// The parent it was moved onto.
    pub onto: String,
    pub old: String,
    pub new: String,
    /// Commits copied onto the parent.
    pub replayed: usize,
    /// Commits left out because the parent already has their changes.
    pub dropped: usize,
}

/// A commit that couldn't be replayed cleanly during a restack.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Conflict {
    pub branch: String,
    pub commit: String,
    pub summary: String,
    /// Paths with conflicts.
    pub paths: Vec<String>,
    /// The parent branch it was being moved onto.
    pub onto: String,
    /// The commit it was based on; `git rebase --onto <onto> <offshoot> <branch>` resolves it by hand.
    pub offshoot: String,
}
