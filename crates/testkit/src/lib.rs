//! Throwaway git repos ([`Fixture`]) and full-repo [`Snapshot`]s for proving what an operation changed.
//!
//! Every `git` call runs with a cleared environment: no system or global config, no inherited `GIT_*`
//! variables, a fixed identity, and a deterministic clock. Identical fixture scripts produce identical SHAs.

mod fixture;
mod snapshot;

pub use fixture::Fixture;
pub use snapshot::{Snapshot, SnapshotDiff};
