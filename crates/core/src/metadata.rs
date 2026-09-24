//! `stack` metadata stored as blob refs under `refs/stack/`.
//!
//! - `refs/stack/trunks/<name>` → `{"version":1,"role":"trunk"|"limb"}`
//! - `refs/stack/branches/<name>` → `{"version":1,"parent":"<branch>","offshoot":"<commit>","pinned":<bool>}`

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::git::{BlobRef, GitRepo};
use crate::{Error, Role};

pub(crate) const TRUNKS: &str = "refs/stack/trunks/";
pub(crate) const BRANCHES: &str = "refs/stack/branches/";
/// Archived branches: `refs/stack/archive/<name>` points at the branch's last commit.
pub(crate) const ARCHIVE: &str = "refs/stack/archive/";
const VERSION: u32 = 1;

/// A branch marked with `stack trunk add`. `role` is inferred when marked: [`Role::Trunk`] or [`Role::Limb`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Mark {
    pub role: Role,
}

/// A recorded parent. `offshoot` is the version of the parent the branch was last known to be based on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Link {
    pub parent: String,
    pub offshoot: String,
    pub pinned: bool,
}

/// A value read from a ref, with the ref's blob id for compare-and-swap updates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Stored<T> {
    pub id: String,
    pub value: T,
}

#[derive(Serialize, Deserialize)]
struct Record<T> {
    version: u32,
    #[serde(flatten)]
    value: T,
}

/// All `stack` metadata, as read at one moment.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Metadata {
    pub marks: BTreeMap<String, Stored<Mark>>,
    pub links: BTreeMap<String, Stored<Link>>,
}

impl Metadata {
    pub(crate) fn load(git: &dyn GitRepo) -> Result<Self, Error> {
        let marks = git
            .blob_refs(TRUNKS)?
            .into_iter()
            .map(|blob| decode(TRUNKS, blob))
            .collect::<Result<_, _>>()?;
        let links = git
            .blob_refs(BRANCHES)?
            .into_iter()
            .map(|blob| decode(BRANCHES, blob))
            .collect::<Result<_, _>>()?;
        Ok(Self { marks, links })
    }
}

pub(crate) fn encode<T: Serialize>(value: &T) -> Vec<u8> {
    let mut data = serde_json::to_vec(&Record {
        version: VERSION,
        value,
    })
    .expect("metadata serialises");
    data.push(b'\n');
    data
}

fn decode<T: for<'de> Deserialize<'de>>(
    prefix: &str,
    blob: BlobRef,
) -> Result<(String, Stored<T>), Error> {
    let corrupt = |reason: String| Error::CorruptMetadata {
        reference: blob.name.clone(),
        reason,
    };
    let record: Record<T> =
        serde_json::from_slice(&blob.data).map_err(|error| corrupt(error.to_string()))?;
    if record.version != VERSION {
        return Err(corrupt(format!("unsupported version {}", record.version)));
    }
    let name = blob
        .name
        .strip_prefix(prefix)
        .expect("ref listed under prefix")
        .to_owned();
    Ok((
        name,
        Stored {
            id: blob.id,
            value: record.value,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodings_are_stable() {
        let link = Link {
            parent: "develop".into(),
            offshoot: "abc".into(),
            pinned: true,
        };
        assert_eq!(
            encode(&link),
            b"{\"version\":1,\"parent\":\"develop\",\"offshoot\":\"abc\",\"pinned\":true}\n"
        );
        assert_eq!(
            encode(&Mark { role: Role::Limb }),
            b"{\"version\":1,\"role\":\"limb\"}\n"
        );
    }

    #[test]
    fn decode_rejects_other_versions() {
        let blob = BlobRef {
            name: format!("{TRUNKS}develop"),
            id: "0".into(),
            data: br#"{"version":2,"role":"trunk"}"#.to_vec(),
        };
        assert!(matches!(
            decode::<Mark>(TRUNKS, blob),
            Err(Error::CorruptMetadata { .. })
        ));
    }
}
