//! Core library for `stack`: stacked-branch workflows on top of git.
//!
//! [`Workspace`] is the entry point for every surface (CLI, GUI, agents). Queries return plain,
//! serialisable view-models.

mod error;
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
    Archived, CommitReview, Conflict, Direction, FollowPosition, FollowerSync, Following, Head,
    Imported, ImportedParent, Landed, LostCommit, MarkKind, Moved, Node, Operation, OperationKind,
    OperationState, Outcome, Parent, PreviewedMove, PushOutcome, Pushed, PushedBranch, Recovered,
    RecoveryOutcome, RefChange, RestackPreview, Restacked, ReviewMark, Role, Scope, Skipped,
    Source, Status, Step, SyncOutcome, Tree, Worktree,
};
pub use workspace::{Environment, Marked, Pinned, Workspace};
