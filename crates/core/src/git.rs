//! The `GitRepo` port and its adapters.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use gix::refs::transaction::{Change, LogChange, PreviousValue, RefEdit, RefLog};
use gix::refs::{FullName, Target};

use crate::{Error, Head};

/// A local branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Branch {
    pub name: String,
    pub tip: String,
    /// Seconds since the epoch of the branch's first reflog entry, if it has a reflog.
    pub created: Option<i64>,
    /// Commits the branch pointed at before, from its reflog, oldest first. Excludes the current tip and the commit
    /// it was created from with `git branch`/`git switch --create`.
    pub former: Vec<FormerTip>,
}

/// A commit a branch used to point at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FormerTip {
    pub id: String,
    /// The reflog message of the update that moved the branch off this commit, e.g. `commit (amend): …`.
    pub left_by: String,
}

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
    /// The directory shared by all worktrees (`git rev-parse --git-common-dir`).
    fn common_dir(&self) -> PathBuf;

    fn head(&self) -> Result<Head, Error>;

    /// The object a ref points at directly (no peeling), or `None` if it doesn't exist.
    ///
    /// # Errors
    /// [`Error::Git`] for a symbolic ref.
    fn ref_value(&self, name: &str) -> Result<Option<String>, Error>;

    /// The ids of the entries of a tree.
    fn tree_entries(&self, tree: &str) -> Result<Vec<String>, Error>;

    /// Writes a flat tree holding `blobs`, each named by its own id.
    fn write_blob_tree(&self, blobs: &[String]) -> Result<String, Error>;

    /// The commit at `refs/heads/<name>`, or `None` if there is no such branch.
    fn branch_tip(&self, name: &str) -> Result<Option<String>, Error>;

    /// The local branch matching the default branch of remote `remote` (its `refs/remotes/<remote>/HEAD`), if both
    /// exist.
    fn remote_default_branch(&self, remote: &str) -> Result<Option<String>, Error>;

    /// Every local branch, sorted by name.
    fn branches(&self) -> Result<Vec<Branch>, Error>;

    /// The best common ancestor of two commits, or `None` if their histories are unrelated.
    fn merge_base(&self, one: &str, two: &str) -> Result<Option<String>, Error>;

    /// Whether `ancestor` is reachable from `descendant`. A commit is its own ancestor.
    fn is_ancestor(&self, ancestor: &str, descendant: &str) -> Result<bool, Error> {
        Ok(ancestor == descendant
            || self.merge_base(ancestor, descendant)?.as_deref() == Some(ancestor))
    }

    /// Commits reachable from `tip` but not from any of `hidden` (`git rev-list tip --not hidden...`).
    fn commits_excluding(&self, tip: &str, hidden: &[String]) -> Result<Vec<String>, Error>;

    /// The stable `git patch-id` of each commit's diff against its first parent, keyed by commit. Merge commits and
    /// empty commits have none.
    fn patch_ids(&self, commits: &[String]) -> Result<HashMap<String, String>, Error>;

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
    fn common_dir(&self) -> PathBuf {
        self.repo.to_thread_local().common_dir().to_owned()
    }

    fn ref_value(&self, name: &str) -> Result<Option<String>, Error> {
        let repo = self.repo.to_thread_local();
        let Some(reference) = repo
            .try_find_reference(&full_name(name)?)
            .map_err(Error::git)?
        else {
            return Ok(None);
        };
        match reference.target() {
            gix::refs::TargetRef::Object(id) => Ok(Some(id.to_string())),
            gix::refs::TargetRef::Symbolic(_) => {
                Err(Error::git(format!("{name} is a symbolic ref")))
            }
        }
    }

    fn tree_entries(&self, tree: &str) -> Result<Vec<String>, Error> {
        let repo = self.repo.to_thread_local();
        let tree = repo.find_tree(object_id(tree)?).map_err(Error::git)?;
        let decoded = tree.decode().map_err(Error::git)?;
        Ok(decoded
            .entries
            .iter()
            .map(|entry| entry.oid.to_string())
            .collect())
    }

    fn write_blob_tree(&self, blobs: &[String]) -> Result<String, Error> {
        let repo = self.repo.to_thread_local();
        let mut entries = blobs
            .iter()
            .map(|blob| {
                Ok(gix::objs::tree::Entry {
                    mode: gix::objs::tree::EntryKind::Blob.into(),
                    filename: blob.as_str().into(),
                    oid: object_id(blob)?,
                })
            })
            .collect::<Result<Vec<_>, Error>>()?;
        entries.sort();
        entries.dedup();
        let id = repo
            .write_object(gix::objs::Tree { entries })
            .map_err(Error::git)?;
        Ok(id.to_string())
    }

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

    fn remote_default_branch(&self, remote: &str) -> Result<Option<String>, Error> {
        let repo = self.repo.to_thread_local();
        let head = full_name(&format!("refs/remotes/{remote}/HEAD"))?;
        let Some(reference) = repo.try_find_reference(&head).map_err(Error::git)? else {
            return Ok(None);
        };
        let gix::refs::TargetRef::Symbolic(target) = reference.target() else {
            return Ok(None);
        };
        let prefix = format!("refs/remotes/{remote}/");
        let Some(name) = target
            .as_bstr()
            .to_string()
            .strip_prefix(&prefix)
            .map(str::to_owned)
        else {
            return Ok(None);
        };
        Ok(self.branch_tip(&name)?.map(|_| name))
    }

    fn branches(&self) -> Result<Vec<Branch>, Error> {
        let repo = self.repo.to_thread_local();
        let platform = repo.references().map_err(Error::git)?;
        let mut branches = Vec::new();
        for reference in platform.local_branches().map_err(Error::git)? {
            let reference = reference.map_err(Error::git)?;
            let name = reference.name().shorten().to_string();
            let mut entries = Vec::new();
            if let Some(lines) = reference.log_iter().all().map_err(Error::git)? {
                for line in lines {
                    let line = line.map_err(Error::git)?;
                    let seconds = line.signature.seconds();
                    entries.push((
                        line.new_oid().to_string(),
                        line.message.to_string(),
                        seconds,
                    ));
                }
            }
            let tip = reference
                .into_fully_peeled_id()
                .map_err(Error::git)?
                .to_string();
            branches.push(Branch {
                created: entries.first().map(|(_, _, seconds)| *seconds),
                former: former_tips(&entries, &tip),
                name,
                tip,
            });
        }
        branches.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(branches)
    }

    fn commits_excluding(&self, tip: &str, hidden: &[String]) -> Result<Vec<String>, Error> {
        let repo = self.repo.to_thread_local();
        let hidden = hidden
            .iter()
            .map(|id| object_id(id))
            .collect::<Result<Vec<_>, _>>()?;
        let walk = repo
            .rev_walk([object_id(tip)?])
            .with_hidden(hidden)
            .all()
            .map_err(Error::git)?;
        walk.map(|info| Ok(info.map_err(Error::git)?.id.to_string()))
            .collect()
    }

    fn patch_ids(&self, commits: &[String]) -> Result<HashMap<String, String>, Error> {
        if commits.is_empty() {
            return Ok(HashMap::new());
        }
        let git_dir = self.repo.to_thread_local().git_dir().to_owned();
        let revisions = commits
            .iter()
            .map(|commit| format!("{commit}\n"))
            .collect::<String>();
        let log = run_git(
            &git_dir,
            &[
                "log",
                "--stdin",
                "--no-walk=unsorted",
                "--patch",
                "--no-color",
                "--no-ext-diff",
                "--no-textconv",
                "--format=commit %H",
            ],
            revisions.as_bytes(),
        )?;
        let ids = run_git(&git_dir, &["patch-id", "--stable"], &log)?;
        Ok(String::from_utf8_lossy(&ids)
            .lines()
            .filter_map(|line| line.split_once(' '))
            .map(|(patch, commit)| (commit.to_owned(), patch.to_owned()))
            .collect())
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

/// Former tips from reflog `entries` of `(new id, message, seconds)`, oldest first.
fn former_tips(entries: &[(String, String, i64)], tip: &str) -> Vec<FormerTip> {
    let mut former: Vec<FormerTip> = Vec::new();
    for (index, pair) in entries.windows(2).enumerate() {
        let [(id, message, _), (_, left_by, _)] = pair else {
            unreachable!("windows(2)")
        };
        // The commit a branch was created from belongs to whatever it was created from.
        if (index == 0 && message.starts_with("branch: Created from")) || id == tip {
            continue;
        }
        former.retain(|existing| existing.id != *id);
        former.push(FormerTip {
            id: id.clone(),
            left_by: left_by.clone(),
        });
    }
    former
}

/// Runs `git --git-dir=<git_dir> <args>` with `stdin`, returning stdout.
fn run_git(git_dir: &Path, args: &[&str], stdin: &[u8]) -> Result<Vec<u8>, Error> {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let mut child = Command::new("git")
        .arg("--git-dir")
        .arg(git_dir)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(Error::git)?;
    let mut input = child.stdin.take().expect("piped stdin");
    let stdin = stdin.to_vec();
    // Written from another thread so a large stdout can't deadlock against a full stdin pipe.
    let writer = std::thread::spawn(move || input.write_all(&stdin));
    let output = child.wait_with_output().map_err(Error::git)?;
    writer
        .join()
        .expect("stdin writer doesn't panic")
        .map_err(Error::git)?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(Error::git(format!(
            "git {}: {}",
            args.join(" "),
            stderr.trim()
        )))
    }
}

/// Creates an empty repository at `path` with `git init`, honouring the user's git config (e.g.
/// `init.defaultBranch`).
pub(crate) fn create_repository(path: &Path) -> Result<(), Error> {
    let output = std::process::Command::new("git")
        .arg("init")
        .arg("--quiet")
        .arg(path)
        .output()
        .map_err(Error::git)?;
    if output.status.success() {
        Ok(())
    } else {
        Err(Error::git(
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        ))
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
    fn former_tips_skip_creation_point_and_current_tip() {
        let entry = |id: &str, message: &str| (id.to_owned(), message.to_owned(), 0);
        let entries = [
            entry("base", "branch: Created from HEAD"),
            entry("one", "commit: one"),
            entry("two", "commit (amend): one"),
            entry("one", "reset: moving to one"),
            entry("three", "commit: three"),
        ];

        let former = former_tips(&entries, "three");

        let pairs: Vec<(&str, &str)> = former
            .iter()
            .map(|tip| (tip.id.as_str(), tip.left_by.as_str()))
            .collect();
        assert_eq!(
            pairs,
            [("two", "reset: moving to one"), ("one", "commit: three")]
        );
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
