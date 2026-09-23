//! The `GitRepo` port and its adapters.

use std::path::Path;

use gix::refs::transaction::{Change, LogChange, PreviousValue, RefEdit, RefLog};
use gix::refs::{FullName, Target};

use crate::{Error, Head};

/// A ref that points directly at a blob, with the blob's contents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BlobRef {
    pub name: String,
    pub id: String,
    pub data: Vec<u8>,
}

/// One ref change within an atomic [`GitRepo::update_refs`] transaction.
///
/// `old` is the value the ref must have (`None`: must not exist); `new` is the value to set (`None`: delete).
/// At least one of them must be `Some`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RefUpdate {
    pub name: String,
    pub old: Option<String>,
    pub new: Option<String>,
}

/// All git access from the core goes through this trait. Object ids are full hex SHAs.
pub(crate) trait GitRepo: Send + Sync {
    fn head(&self) -> Result<Head, Error>;

    /// The commit at `refs/heads/<name>`, or `None` if there is no such branch.
    fn branch_tip(&self, name: &str) -> Result<Option<String>, Error>;

    /// The best common ancestor of two commits, or `None` if their histories are unrelated.
    fn merge_base(&self, one: &str, two: &str) -> Result<Option<String>, Error>;

    /// Every ref under `prefix` (which must end in `/`).
    ///
    /// # Errors
    /// [`Error::CorruptMetadata`] if one of them doesn't point at a blob.
    fn blob_refs(&self, prefix: &str) -> Result<Vec<BlobRef>, Error>;

    fn write_blob(&self, data: &[u8]) -> Result<String, Error>;

    /// Applies every update or none. Fails without changes if any ref's current value isn't its `old`.
    fn update_refs(&self, updates: &[RefUpdate]) -> Result<(), Error>;
}

/// [`GitRepo`] backed by `gix`.
pub(crate) struct GixRepo {
    repo: gix::ThreadSafeRepository,
}

impl GixRepo {
    pub(crate) fn discover(path: &Path) -> Result<Self, Error> {
        let repo =
            gix::ThreadSafeRepository::discover(path).map_err(|source| Error::NotARepository {
                path: path.to_owned(),
                source: source.into(),
            })?;
        Ok(Self { repo })
    }
}

impl GitRepo for GixRepo {
    fn head(&self) -> Result<Head, Error> {
        let repo = self.repo.to_thread_local();
        let head = repo.head().map_err(Error::git)?;
        let commit = if head.is_unborn() {
            None
        } else {
            Some(repo.head_id().map_err(Error::git)?.to_string())
        };
        Ok(match (head.referent_name(), commit) {
            (Some(name), commit) => Head::Branch {
                name: name.shorten().to_string(),
                commit,
            },
            (None, Some(commit)) => Head::Detached { commit },
            (None, None) => unreachable!("an unborn HEAD always names a branch"),
        })
    }

    fn branch_tip(&self, name: &str) -> Result<Option<String>, Error> {
        let repo = self.repo.to_thread_local();
        let full_name = full_name(&format!("refs/heads/{name}"))?;
        match repo.try_find_reference(&full_name).map_err(Error::git)? {
            Some(reference) => Ok(Some(
                reference
                    .into_fully_peeled_id()
                    .map_err(Error::git)?
                    .to_string(),
            )),
            None => Ok(None),
        }
    }

    fn merge_base(&self, one: &str, two: &str) -> Result<Option<String>, Error> {
        let repo = self.repo.to_thread_local();
        match repo.merge_base(object_id(one)?, object_id(two)?) {
            Ok(id) => Ok(Some(id.to_string())),
            Err(gix::repository::merge_base::Error::NotFound { .. }) => Ok(None),
            Err(error) => Err(Error::git(error)),
        }
    }

    fn blob_refs(&self, prefix: &str) -> Result<Vec<BlobRef>, Error> {
        let repo = self.repo.to_thread_local();
        let platform = repo.references().map_err(Error::git)?;
        let mut refs = Vec::new();
        for reference in platform.prefixed(prefix).map_err(Error::git)? {
            let reference = reference.map_err(Error::git)?;
            let name = reference.name().as_bstr().to_string();
            let corrupt = |reason: &str| Error::CorruptMetadata {
                reference: name.clone(),
                reason: reason.into(),
            };
            let id = reference
                .try_id()
                .ok_or_else(|| corrupt("is a symbolic ref"))?
                .detach();
            let object = repo.find_object(id).map_err(Error::git)?;
            if object.kind != gix::object::Kind::Blob {
                return Err(corrupt(&format!("points at a {}, not a blob", object.kind)));
            }
            refs.push(BlobRef {
                id: id.to_string(),
                data: object.detach().data,
                name,
            });
        }
        Ok(refs)
    }

    fn write_blob(&self, data: &[u8]) -> Result<String, Error> {
        let repo = self.repo.to_thread_local();
        Ok(repo.write_blob(data).map_err(Error::git)?.to_string())
    }

    fn update_refs(&self, updates: &[RefUpdate]) -> Result<(), Error> {
        let repo = self.repo.to_thread_local();
        let edits = updates
            .iter()
            .map(|update| {
                let expected = match &update.old {
                    Some(old) => PreviousValue::MustExistAndMatch(Target::Object(object_id(old)?)),
                    None => PreviousValue::MustNotExist,
                };
                let change = match &update.new {
                    Some(new) => Change::Update {
                        log: LogChange::default(),
                        expected,
                        new: Target::Object(object_id(new)?),
                    },
                    // gix panics on a delete that expects the ref not to exist.
                    None if update.old.is_none() => {
                        return Err(Error::git(format!(
                            "{}: update has neither old nor new value",
                            update.name
                        )));
                    }
                    None => Change::Delete {
                        expected,
                        log: RefLog::AndReference,
                    },
                };
                Ok(RefEdit {
                    change,
                    name: full_name(&update.name)?,
                    deref: false,
                })
            })
            .collect::<Result<Vec<_>, Error>>()?;
        repo.edit_references(edits).map_err(Error::git)?;
        Ok(())
    }
}

fn full_name(name: &str) -> Result<FullName, Error> {
    FullName::try_from(name).map_err(|_| Error::InvalidRefName { name: name.into() })
}

fn object_id(hex: &str) -> Result<gix::ObjectId, Error> {
    gix::ObjectId::from_hex(hex.as_bytes()).map_err(Error::git)
}

#[cfg(test)]
mod tests {
    use stack_testkit::Fixture;

    use super::*;

    fn repo(fixture: &Fixture) -> GixRepo {
        GixRepo::discover(&fixture.path()).unwrap()
    }

    #[test]
    fn update_refs_rejects_stale_old_value() {
        let fixture = Fixture::new();
        let repo = repo(&fixture);
        let blob = repo.write_blob(b"x").unwrap();
        let other = repo.write_blob(b"y").unwrap();
        repo.update_refs(&[RefUpdate {
            name: "refs/stack/test".into(),
            old: None,
            new: Some(blob.clone()),
        }])
        .unwrap();
        let before = fixture.snapshot();

        let stale = RefUpdate {
            name: "refs/stack/test".into(),
            old: Some(other),
            new: None,
        };
        assert!(repo.update_refs(&[stale]).is_err());
        fixture.assert_unchanged(&before);
    }

    #[test]
    fn update_refs_rejects_empty_update_without_panicking() {
        let fixture = Fixture::new();
        let before = fixture.snapshot();

        let empty = RefUpdate {
            name: "refs/stack/test".into(),
            old: None,
            new: None,
        };
        assert!(repo(&fixture).update_refs(&[empty]).is_err());
        fixture.assert_unchanged(&before);
    }

    #[test]
    fn update_refs_is_all_or_nothing() {
        let fixture = Fixture::new();
        let repo = repo(&fixture);
        let blob = repo.write_blob(b"x").unwrap();
        let before = fixture.snapshot();

        let good = RefUpdate {
            name: "refs/stack/good".into(),
            old: None,
            new: Some(blob.clone()),
        };
        let bad = RefUpdate {
            name: "refs/stack/bad".into(),
            old: Some(blob.clone()),
            new: Some(blob),
        };
        assert!(repo.update_refs(&[good, bad]).is_err());
        fixture.assert_unchanged(&before);
    }
}
