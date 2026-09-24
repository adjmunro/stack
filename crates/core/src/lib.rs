//! Core library for `stack`: stacked-branch workflows on top of git.
//!
//! [`Workspace`] is the entry point for every surface (CLI, GUI, agents). Queries return plain,
//! serialisable view-models.

mod error;
mod git;
mod journal;
mod metadata;
mod resolve;
mod restack;
mod store;
mod view;
mod workspace;

pub use error::Error;
pub use view::{
    Conflict, Head, Moved, Node, Operation, OperationKind, OperationState, Outcome, Parent,
    PreviewedMove, Recovered, RecoveryOutcome, RefChange, RestackPreview, Restacked, Role, Source,
    Status, Tree,
};
pub use workspace::{Environment, Marked, Pinned, Workspace};
