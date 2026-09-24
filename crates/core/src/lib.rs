//! Core library for `stack`: stacked-branch workflows on top of git.
//!
//! [`Workspace`] is the entry point for every surface (CLI, GUI, agents). Queries return plain,
//! serialisable view-models.

mod error;
mod forge;
mod git;
mod journal;
mod line;
mod metadata;
mod resolve;
mod restack;
mod store;
mod view;
mod workspace;

pub use error::Error;
pub use view::{
    Archived, CommitRange, CommitReview, Conflict, Delta, Direction, FollowPosition, FollowerSync,
    Following, GuardInstalled, GuardViolation, Head, Imported, ImportedParent, Landed, LostCommit,
    MarkKind, Moved, Node, Operation, OperationKind, OperationState, Outcome, Parent,
    PreviewedMove, ProposalAction, Proposed, ProposedBranch, PushOutcome, Pushed, PushedBranch,
    Recovered, RecoveryOutcome, RefChange, RestackPreview, Restacked, ReviewMark, Role, Scope,
    Skipped, Source, Status, Step, SyncOutcome, Tree, Worktree,
};
pub use workspace::{
    Environment, Marked, Pinned, ResolveOutcome, Resolving, Synced, TrunkSync, TrunkUpdate,
    Workspace,
};
