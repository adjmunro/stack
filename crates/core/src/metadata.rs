//! `stack` metadata stored as blob refs under `refs/stack/`.
//!
//! - `refs/stack/trunks/<name>` → `{"version":1}`
//! - `refs/stack/branches/<name>` → `{"version":1,"parent":"<branch>","base":"<commit>"}`
//!
//! `base` is the commit `<name>` was last based on in its parent.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::Error;
use crate::git::{BlobRef, GitRepo};

pub(crate) const TRUNKS: &str = "refs/stack/trunks/";
pub(crate) const BRANCHES: &str = "refs/stack/branches/";
const VERSION: u32 = 1;

/// A tracked branch's link to its parent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Link {
    pub parent: String,
    pub base: String,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Empty {}

/// All `stack` metadata, as read at one moment.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Metadata {
    pub trunks: BTreeMap<String, Stored<()>>,
    pub branches: BTreeMap<String, Stored<Link>>,
}

impl Metadata {
    pub(crate) fn load(git: &dyn GitRepo) -> Result<Self, Error> {
        let trunks = git
            .blob_refs(TRUNKS)?
            .into_iter()
            .map(|blob| {
                decode::<Empty>(TRUNKS, blob).map(|(name, stored)| {
                    (
                        name,
                        Stored {
                            id: stored.id,
                            value: (),
                        },
                    )
                })
            })
            .collect::<Result<_, _>>()?;
        let branches = git
            .blob_refs(BRANCHES)?
            .into_iter()
            .map(|blob| decode::<Link>(BRANCHES, blob))
            .collect::<Result<_, _>>()?;
        Ok(Self { trunks, branches })
    }

    pub(crate) fn is_trunk(&self, name: &str) -> bool {
        self.trunks.contains_key(name)
    }

    /// Tracked branches whose parent is `name`, sorted.
    pub(crate) fn children(&self, name: &str) -> Vec<String> {
        self.branches
            .iter()
            .filter(|(_, stored)| stored.value.parent == name)
            .map(|(child, _)| child.clone())
            .collect()
    }

    /// `name` followed by its parent, grandparent, and so on. Stops at an untracked name or a repeat.
    pub(crate) fn lineage<'a>(&'a self, name: &'a str) -> Vec<&'a str> {
        let mut seen = BTreeSet::new();
        let mut current = Some(name);
        std::iter::from_fn(|| {
            let name = current.filter(|name| seen.insert(*name))?;
            current = self
                .branches
                .get(name)
                .map(|stored| stored.value.parent.as_str());
            Some(name)
        })
        .collect()
    }
}

pub(crate) fn encode_trunk() -> Vec<u8> {
    encode(Empty {})
}

pub(crate) fn encode_link(link: &Link) -> Vec<u8> {
    encode(link.clone())
}

fn encode<T: Serialize>(value: T) -> Vec<u8> {
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
    fn link_encoding_is_stable() {
        let link = Link {
            parent: "develop".into(),
            base: "abc".into(),
        };
        assert_eq!(
            encode_link(&link),
            b"{\"version\":1,\"parent\":\"develop\",\"base\":\"abc\"}\n"
        );
    }

    #[test]
    fn decode_rejects_other_versions() {
        let blob = BlobRef {
            name: format!("{TRUNKS}develop"),
            id: "0".into(),
            data: br#"{"version":2}"#.to_vec(),
        };
        assert!(matches!(
            decode::<Empty>(TRUNKS, blob),
            Err(Error::CorruptMetadata { .. })
        ));
    }

    #[test]
    fn lineage_stops_at_cycles() {
        let link = |parent: &str| Stored {
            id: String::new(),
            value: Link {
                parent: parent.into(),
                base: String::new(),
            },
        };
        let metadata = Metadata {
            trunks: BTreeMap::new(),
            branches: BTreeMap::from([("a".into(), link("b")), ("b".into(), link("a"))]),
        };
        assert_eq!(metadata.lineage("a"), ["a", "b"]);
    }
}
