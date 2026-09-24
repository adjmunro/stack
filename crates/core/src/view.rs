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

/// Which branches around the current one an operation covers. See the glossary's "line".
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scope {
    pub direction: Direction,
    /// Continue leafward past limbs to the leaves.
    pub through_limbs: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    /// Parents rootward and descendants leafward.
    #[default]
    Both,
    /// The branch and its parents, up to the nearest trunk or limb.
    Rootward,
    /// The branch and its descendants.
    Leafward,
}

/// A step through a stack for [`crate::Workspace::step`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Step {
    /// To the parent, `n` times. May land on a trunk.
    Down(usize),
    /// To the only child, `n` times.
    Up(usize),
    /// Up through only children to a leaf.
    Top,
    /// Down to the first branch of the stack: the nearest limb, or else the branch just leafward of the trunk.
    Bottom,
}

/// A branch put away with [`crate::Workspace::archive`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Archived {
    pub name: String,
    pub commit: String,
}

/// A personal note on a commit's change, kept across rebases until the change itself changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReviewMark {
    pub kind: MarkKind,
    pub note: Option<String>,
    /// Seconds since the epoch.
    pub marked_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarkKind {
    /// Read and approved.
    Reviewed,
    /// Built and tested.
    Tested,
    /// Needs another look.
    Flagged,
}

/// A commit with its review marks, for [`crate::Workspace::review`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CommitReview {
    pub commit: String,
    pub summary: String,
    pub marks: Vec<ReviewMark>,
}

/// A worktree of the repository.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Worktree {
    pub path: std::path::PathBuf,
    /// The commit checked out; `None` before the first commit.
    pub head: Option<String>,
    /// The branch checked out; `None` when detached.
    pub branch: Option<String>,
    /// For a follower: the branch it follows and how it stands against it.
    pub follows: Option<Following>,
    /// Whether this is the worktree the workspace was opened from.
    pub current: bool,
}

/// A follower worktree's branch and position. See the glossary's "follower".
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Following {
    pub branch: String,
    pub position: FollowPosition,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FollowPosition {
    /// At the branch's tip.
    UpToDate,
    /// Behind or off the branch's tip (e.g. it moved on or was restacked); a sync brings it up to date.
    Behind,
    /// Has commits of its own on top of the branch's tip, ready to land.
    Ahead,
    /// The branch no longer exists.
    Orphaned,
}

/// What a follower sync did to one follower.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FollowerSync {
    pub path: std::path::PathBuf,
    pub branch: String,
    pub outcome: SyncOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SyncOutcome {
    Moved {
        from: String,
        to: String,
    },
    UpToDate,
    /// Left alone, e.g. because it has commits to land or local changes that clash.
    Skipped {
        reason: String,
    },
}

/// The result of [`crate::Workspace::land`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Landed {
    pub branch: String,
    pub old: String,
    pub new: String,
    /// Commits added to the branch.
    pub commits: usize,
    /// The worktree that has the branch checked out, whose files moved with it.
    pub holder: Option<std::path::PathBuf>,
}

/// A commit you've been on that no ref reaches any more, from [`crate::Workspace::lost`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LostCommit {
    pub commit: String,
    pub summary: String,
    /// The ref whose reflog last saw it, e.g. `refs/heads/feat/a`, or `HEAD` if no branch did at that moment.
    pub seen_on: String,
    /// The reflog message of that sighting, e.g. `commit (amend): …` or `reset: moving to …`.
    pub how: String,
    /// Seconds since the epoch.
    pub seen_at: i64,
}

/// The result of an import such as [`crate::Workspace::import_graphite`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Imported {
    pub outcome: Outcome,
    /// Branches newly marked as trunks.
    pub trunks: Vec<String>,
    /// Parents newly recorded.
    pub parents: Vec<ImportedParent>,
    pub skipped: Vec<Skipped>,
}

impl Default for Imported {
    fn default() -> Self {
        Self {
            outcome: Outcome::Unchanged,
            trunks: Vec::new(),
            parents: Vec::new(),
            skipped: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ImportedParent {
    pub branch: String,
    pub parent: String,
}

/// Something an import left out, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Skipped {
    pub branch: String,
    pub reason: String,
}

/// The result of [`crate::Workspace::propose`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Proposed {
    pub pushed: Pushed,
    /// One entry per pushed branch, rootward first.
    pub pull_requests: Vec<ProposedBranch>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProposedBranch {
    pub branch: String,
    /// The branch it targets: its parent.
    pub base: String,
    pub number: Option<u64>,
    pub url: Option<String>,
    pub action: ProposalAction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProposalAction {
    Created,
    /// An open pull request whose base was changed to the branch's current parent.
    Retargeted {
        from: String,
    },
    UpToDate,
    /// Left alone, e.g. because its pull request is merged or closed, or its push was rejected.
    Skipped {
        reason: String,
    },
}

/// Something a push would do that the guard stops. See [`crate::Workspace::check_push`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GuardViolation {
    /// Pushing to (or deleting) a protected branch.
    Protected { branch: String, deleting: bool },
    /// Pushing `local` (a branch, or a commit) to a remote branch with another name.
    NameMismatch { local: String, remote: String },
}

/// The result of [`crate::Workspace::install_guard`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GuardInstalled {
    pub hook: std::path::PathBuf,
    /// Whether an existing `pre-push` hook runs after the guard.
    pub chained: bool,
}

/// Two branches' own changes, from [`crate::Workspace::delta`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Delta {
    pub a: CommitRange,
    pub b: CommitRange,
    /// A tree: `b`'s own changes replayed onto `a`'s base. Diff it against `a.tip` to see only how the changes
    /// differ. `None` if replaying conflicts.
    pub b_on_a_base: Option<String>,
    /// Paths that conflict when replaying `b` onto `a`'s base.
    pub conflicts: Vec<String>,
}

/// A branch's own commits: `base..tip`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CommitRange {
    pub branch: String,
    pub base: String,
    pub tip: String,
}

/// What commit subjects must look like, for [`crate::Workspace::lint`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LintRules {
    /// A regex subjects must match.
    pub pattern: String,
    /// The longest subject allowed, in characters.
    pub max_length: usize,
}

/// A commit whose subject breaks the rules.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LintFinding {
    pub commit: String,
    pub summary: String,
    pub problems: Vec<LintProblem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LintProblem {
    TooLong { length: usize, max: usize },
    NoMatch { pattern: String },
}

/// The result of [`crate::Workspace::amend_into`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Amended {
    /// The commit amended.
    pub commit: String,
    /// What it became.
    pub rewritten: String,
    /// The branch it belongs to.
    pub branch: String,
    /// Every branch moved, the amended one first.
    pub restacked: Restacked,
}

/// The result of [`crate::Workspace::push`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Pushed {
    pub remote: String,
    /// One entry per branch in the scope, rootward first.
    pub branches: Vec<PushedBranch>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PushedBranch {
    pub name: String,
    pub outcome: PushOutcome,
    /// git's summary, e.g. `[rejected] (stale info)`.
    pub summary: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PushOutcome {
    Created,
    FastForwarded,
    /// Replaced a remote branch that had been rewritten locally (e.g. by a restack), which the lease allowed.
    Forced,
    UpToDate,
    /// Refused, e.g. because the remote branch changed since it was last fetched.
    Rejected,
}
