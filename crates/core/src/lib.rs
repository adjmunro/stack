//! Core library for `stack`: stacked-branch workflows on top of git.
//!
//! [`Workspace`] is the entry point for every surface (CLI, GUI, agents). Queries return plain,
//! serialisable view-models.

mod error;
mod git;
mod metadata;
mod view;
mod workspace;

pub use error::Error;
pub use view::{Head, Node, Outcome, Status, Tree};
pub use workspace::Workspace;
