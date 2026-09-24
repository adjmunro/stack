//! The `GitRepo` port and its adapters.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use gix::refs::transaction::{Change, LogChange, PreviousValue, RefEdit, RefLog};
use gix::refs::{FullName, Target};

use crate::{Environment, Error, Head};

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

/// The parts of a commit restack needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CommitInfo {
    pub tree: String,
    pub parents: Vec<String>,
    pub summary: String,
}

/// The result of [`GitRepo::merge_trees`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Merge {
    Clean { tree: String },
    Conflicted { paths: Vec<String> },
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

    /// Every ref under `prefix` (which must end in `/`) with the object it points at, sorted by name.
    fn refs(&self, prefix: &str) -> Result<Vec<(String, String)>, Error>;

    /// Every ref under `prefix` (which must end in `/`).
    ///
    /// # Errors
    /// [`Error::CorruptMetadata`] if one of them doesn't point at a blob.
    fn blob_refs(&self, prefix: &str) -> Result<Vec<BlobRef>, Error>;

    fn write_blob(&self, data: &[u8]) -> Result<String, Error>;

    /// Applies every update or none. Fails without changes if any ref's current value isn't its `old`.
    /// `message` goes in the reflog of refs that have one (e.g. branches).
    fn update_refs(&self, updates: &[RefUpdate], message: &str) -> Result<(), Error>;

    /// The commit `revision` names (`git rev-parse <revision>^{commit}`).
    ///
    /// # Errors
    /// [`Error::UnknownRevision`] if it names no commit.
    fn resolve_commit(&self, revision: &str) -> Result<String, Error>;

    /// A commit's tree, parents, and subject line.
    fn commit(&self, id: &str) -> Result<CommitInfo, Error>;

    /// Three-way merges commits `ours` and `theirs` over `base` without touching the index or working tree
    /// (`git merge-tree`).
    fn merge_trees(&self, base: &str, ours: &str, theirs: &str) -> Result<Merge, Error>;

    /// Writes a commit with `tree` and `parent`, and `original`'s author and message (`git commit-tree`, which
    /// signs if the user's config says to). The committer is the current user.
    fn copy_commit(&self, original: &str, tree: &str, parent: &str) -> Result<String, Error>;

    /// Whether tracked files in the working tree and index match `HEAD`. Untracked files are ignored.
    fn is_worktree_clean(&self) -> Result<bool, Error>;

    /// Whether the index matches `commit`'s tree.
    fn index_matches(&self, commit: &str) -> Result<bool, Error>;

    /// Moves the index and working tree from commit `from` to commit `to` (`git read-tree -m -u`), without touching
    /// refs. Fails without changes if that would overwrite local changes.
    fn checkout(&self, from: &str, to: &str) -> Result<(), Error>;

    /// Branches checked out in other worktrees of this repository.
    fn checked_out_elsewhere(&self) -> Result<Vec<String>, Error>;

    /// The names of this repository's remotes, sorted.
    fn remotes(&self) -> Result<Vec<String>, Error>;

    /// A git config value as git resolves it (all scopes, includes), or `None` if unset.
    fn config_value(&self, key: &str) -> Result<Option<String>, Error>;

    /// Pushes each branch to the same name on `remote` (`git push --porcelain`), with `--force-with-lease` against
    /// its `lease` (`None`: it must not exist there). Runs the user's `pre-push` hook. Upstreams are untouched.
    ///
    /// # Errors
    /// Only if git can't run or the remote can't be reached; per-branch rejections are in the result.
    fn push(&self, remote: &str, branches: &[PushRef]) -> Result<Vec<PushStatus>, Error>;

    /// Makes `remote`/`branch` the upstream of local `branch`.
    fn set_upstream(&self, branch: &str, remote: &str) -> Result<(), Error>;

    /// Checks out `branch` (`git switch`), creating it at HEAD first if `create`. Git refuses, without changes, if
    /// local changes would be overwritten.
    fn switch(&self, branch: &str, create: bool) -> Result<(), Error>;

    /// Commits the index (`git commit`), staging changes to tracked files first if `all`. Runs the user's hooks.
    fn commit_index(&self, message: &str, all: bool) -> Result<(), Error>;
}

/// A branch to push, and the value its remote-tracking ref had when last fetched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PushRef {
    pub branch: String,
    pub lease: Option<String>,
}

/// What happened to one pushed branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PushStatus {
    pub branch: String,
    pub outcome: crate::PushOutcome,
    /// git's summary, e.g. `[rejected] (stale info)`.
    pub summary: String,
}

/// [`GitRepo`] backed by `gix`.
pub(crate) struct GixRepo {
    repo: gix::ThreadSafeRepository,
    /// `commit.gpgSign`, read on first use.
    signs_commits: std::sync::OnceLock<bool>,
    environment: Environment,
}

impl GixRepo {
    /// `git <args>` pointed explicitly at this repository, so inherited `GIT_DIR`, `GIT_INDEX_FILE`, etc. can't
    /// redirect it. Other environment (signing agents, identity overrides) passes through.
    fn git(&self, args: &[&str]) -> Command {
        let repo = self.repo.to_thread_local();
        let mut command = Command::new("git");
        match &self.environment {
            Environment::Inherit => {
                for variable in [
                    "GIT_INDEX_FILE",
                    "GIT_OBJECT_DIRECTORY",
                    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
                    "GIT_COMMON_DIR",
                    "GIT_NAMESPACE",
                ] {
                    command.env_remove(variable);
                }
            }
            Environment::Exactly(variables) => {
                command
                    .env_clear()
                    .envs(variables.iter().map(|(key, value)| (key, value)));
            }
        }
        command.arg("--git-dir").arg(repo.git_dir());
        if let Some(workdir) = repo.workdir() {
            command.arg("--work-tree").arg(workdir).current_dir(workdir);
        }
        command.args(args);
        command
    }

    fn signs_commits(&self) -> Result<bool, Error> {
        if let Some(signs) = self.signs_commits.get() {
            return Ok(*signs);
        }
        let mut command = self.git(&["config", "--type=bool", "--get", "commit.gpgsign"]);
        let output = run(&mut command, b"")?;
        let signs = match output.status.code() {
            Some(0) => String::from_utf8_lossy(&output.stdout).trim() == "true",
            // Unset.
            Some(1) => false,
            _ => return Err(command_error(&command, &output)),
        };
        Ok(*self.signs_commits.get_or_init(|| signs))
    }

    pub(crate) fn discover(path: &Path, environment: Environment) -> Result<Self, Error> {
        let repo =
            gix::ThreadSafeRepository::discover(path).map_err(|source| Error::NotARepository {
                path: path.to_owned(),
                source: source.into(),
            })?;
        Ok(Self {
            repo,
            signs_commits: std::sync::OnceLock::new(),
            environment,
        })
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
        let revisions = commits
            .iter()
            .map(|commit| format!("{commit}\n"))
            .collect::<String>();
        let log = run_ok(
            &mut self.git(&[
                "log",
                "--stdin",
                "--no-walk=unsorted",
                "--patch",
                "--no-color",
                "--no-ext-diff",
                "--no-textconv",
                "--format=commit %H",
            ]),
            revisions.as_bytes(),
        )?;
        let ids = run_ok(&mut self.git(&["patch-id", "--stable"]), &log)?;
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

    fn refs(&self, prefix: &str) -> Result<Vec<(String, String)>, Error> {
        let repo = self.repo.to_thread_local();
        let platform = repo.references().map_err(Error::git)?;
        let mut refs = Vec::new();
        for reference in platform.prefixed(prefix).map_err(Error::git)? {
            let reference = reference.map_err(Error::git)?;
            if let Some(id) = reference.try_id() {
                refs.push((reference.name().as_bstr().to_string(), id.to_string()));
            }
        }
        refs.sort();
        Ok(refs)
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

    fn resolve_commit(&self, revision: &str) -> Result<String, Error> {
        let repo = self.repo.to_thread_local();
        let unknown = || Error::UnknownRevision {
            revision: revision.into(),
        };
        let id = repo.rev_parse_single(revision).map_err(|_| unknown())?;
        let commit = id
            .object()
            .map_err(Error::git)?
            .peel_to_commit()
            .map_err(|_| unknown())?;
        Ok(commit.id.to_string())
    }

    fn commit(&self, id: &str) -> Result<CommitInfo, Error> {
        let repo = self.repo.to_thread_local();
        let commit = repo.find_commit(object_id(id)?).map_err(Error::git)?;
        let decoded = commit.decode().map_err(Error::git)?;
        Ok(CommitInfo {
            tree: decoded.tree().to_string(),
            parents: decoded.parents().map(|parent| parent.to_string()).collect(),
            summary: decoded.message_summary().to_string(),
        })
    }

    fn merge_trees(&self, base: &str, ours: &str, theirs: &str) -> Result<Merge, Error> {
        let base = format!("--merge-base={base}");
        let mut command = self.git(&[
            "merge-tree",
            "--write-tree",
            "--name-only",
            "--no-messages",
            &base,
            ours,
            theirs,
        ]);
        let output = run(&mut command, b"")?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut lines = stdout.lines();
        match output.status.code() {
            Some(0) => {
                let tree = lines
                    .next()
                    .ok_or_else(|| command_error(&command, &output))?;
                Ok(Merge::Clean {
                    tree: tree.to_owned(),
                })
            }
            Some(1) => Ok(Merge::Conflicted {
                paths: lines
                    .skip(1)
                    .filter(|line| !line.is_empty())
                    .map(str::to_owned)
                    .collect(),
            }),
            _ => Err(command_error(&command, &output)),
        }
    }

    fn copy_commit(&self, original: &str, tree: &str, parent: &str) -> Result<String, Error> {
        let repo = self.repo.to_thread_local();
        let commit = repo.find_commit(object_id(original)?).map_err(Error::git)?;
        let decoded = commit.decode().map_err(Error::git)?;
        let author = decoded.author().map_err(Error::git)?;
        let mut command = self.git(&["commit-tree", tree, "-p", parent]);
        // commit-tree is plumbing: it ignores commit.gpgSign, so honour it here as porcelain commands do.
        if self.signs_commits()? {
            command.arg("-S");
        }
        command
            .env("GIT_AUTHOR_NAME", author.name.to_string())
            .env("GIT_AUTHOR_EMAIL", author.email.to_string())
            .env("GIT_AUTHOR_DATE", author.time);
        let stdout = run_ok(&mut command, decoded.message)?;
        Ok(String::from_utf8_lossy(&stdout).trim().to_owned())
    }

    fn is_worktree_clean(&self) -> Result<bool, Error> {
        if self.repo.to_thread_local().workdir().is_none() {
            return Ok(true);
        }
        let mut command = self.git(&[
            "--no-optional-locks",
            "status",
            "--porcelain",
            "--untracked-files=no",
        ]);
        Ok(run_ok(&mut command, b"")?.is_empty())
    }

    fn index_matches(&self, commit: &str) -> Result<bool, Error> {
        let mut command = self.git(&["diff-index", "--cached", "--quiet", commit, "--"]);
        let output = run(&mut command, b"")?;
        match output.status.code() {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            _ => Err(command_error(&command, &output)),
        }
    }

    fn checkout(&self, from: &str, to: &str) -> Result<(), Error> {
        run_ok(&mut self.git(&["read-tree", "-m", "-u", from, to]), b"").map(|_| ())
    }

    fn checked_out_elsewhere(&self) -> Result<Vec<String>, Error> {
        let repo = self.repo.to_thread_local();
        let Some(here) = repo.workdir().map(|workdir| workdir.canonicalize()) else {
            return Ok(Vec::new());
        };
        let here = here.map_err(Error::git)?;
        let listing = run_ok(&mut self.git(&["worktree", "list", "--porcelain"]), b"")?;
        let listing = String::from_utf8_lossy(&listing);
        let mut branches = Vec::new();
        for block in listing.split("\n\n") {
            let mut path = None;
            let mut branch = None;
            for line in block.lines() {
                if let Some(value) = line.strip_prefix("worktree ") {
                    path = Some(PathBuf::from(value));
                } else if let Some(value) = line.strip_prefix("branch refs/heads/") {
                    branch = Some(value.to_owned());
                }
            }
            let elsewhere =
                path.is_some_and(|path| path.canonicalize().map_or(true, |path| path != here));
            if let (true, Some(branch)) = (elsewhere, branch) {
                branches.push(branch);
            }
        }
        Ok(branches)
    }

    fn remotes(&self) -> Result<Vec<String>, Error> {
        let listing = run_ok(&mut self.git(&["remote"]), b"")?;
        let mut remotes: Vec<String> = String::from_utf8_lossy(&listing)
            .lines()
            .map(str::to_owned)
            .collect();
        remotes.sort();
        Ok(remotes)
    }

    fn config_value(&self, key: &str) -> Result<Option<String>, Error> {
        let mut command = self.git(&["config", "--get", key]);
        let output = run(&mut command, b"")?;
        match output.status.code() {
            Some(0) => Ok(Some(
                String::from_utf8_lossy(&output.stdout)
                    .trim_end()
                    .to_owned(),
            )),
            Some(1) => Ok(None),
            _ => Err(command_error(&command, &output)),
        }
    }

    fn push(&self, remote: &str, branches: &[PushRef]) -> Result<Vec<PushStatus>, Error> {
        let mut args = vec!["push".to_owned(), "--porcelain".to_owned()];
        for branch in branches {
            let lease = branch.lease.as_deref().unwrap_or("");
            args.push(format!(
                "--force-with-lease=refs/heads/{}:{lease}",
                branch.branch
            ));
        }
        args.push(remote.to_owned());
        args.extend(
            branches
                .iter()
                .map(|branch| format!("refs/heads/{0}:refs/heads/{0}", branch.branch)),
        );
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let mut command = self.git(&args);
        let output = run(&mut command, b"")?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let statuses: Vec<PushStatus> = stdout.lines().filter_map(parse_push_line).collect();
        if statuses.is_empty() && !output.status.success() {
            return Err(command_error(&command, &output));
        }
        Ok(statuses)
    }

    fn switch(&self, branch: &str, create: bool) -> Result<(), Error> {
        let args: &[&str] = if create {
            &["switch", "--quiet", "--create", branch]
        } else {
            &["switch", "--quiet", branch]
        };
        run_ok(&mut self.git(args), b"").map(|_| ())
    }

    fn commit_index(&self, message: &str, all: bool) -> Result<(), Error> {
        let mut args = vec!["commit", "--quiet", "--file=-"];
        if all {
            args.push("--all");
        }
        run_ok(&mut self.git(&args), message.as_bytes()).map(|_| ())
    }

    fn set_upstream(&self, branch: &str, remote: &str) -> Result<(), Error> {
        let upstream = format!("--set-upstream-to={remote}/{branch}");
        run_ok(
            &mut self.git(&["branch", "--quiet", &upstream, branch]),
            b"",
        )
        .map(|_| ())
    }

    fn update_refs(&self, updates: &[RefUpdate], message: &str) -> Result<(), Error> {
        let repo = self.repo.to_thread_local();
        let checked_out = repo
            .head_name()
            .map_err(Error::git)?
            .map(|name| name.as_bstr().to_string());
        let edits = updates
            .iter()
            .map(|update| {
                let expected = match &update.old {
                    Some(old) => PreviousValue::MustExistAndMatch(Target::Object(object_id(old)?)),
                    None => PreviousValue::MustNotExist,
                };
                let change = match &update.new {
                    Some(new) => Change::Update {
                        log: LogChange {
                            message: message.into(),
                            ..LogChange::default()
                        },
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
                // Updating the checked-out branch through HEAD also records the move in HEAD's reflog, as git does.
                let through_head =
                    update.new.is_some() && checked_out.as_deref() == Some(update.name.as_str());
                Ok(RefEdit {
                    change,
                    name: full_name(if through_head { "HEAD" } else { &update.name })?,
                    deref: through_head,
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

/// Parses one ref line of `git push --porcelain`: `<flag>\t<from>:<to>\t<summary>`.
fn parse_push_line(line: &str) -> Option<PushStatus> {
    let mut fields = line.splitn(3, '\t');
    let flag = fields.next()?;
    let (_, to) = fields.next()?.split_once(':')?;
    let summary = fields.next().unwrap_or_default().to_owned();
    let outcome = match flag {
        "*" => crate::PushOutcome::Created,
        " " => crate::PushOutcome::FastForwarded,
        "+" => crate::PushOutcome::Forced,
        "=" => crate::PushOutcome::UpToDate,
        "!" => crate::PushOutcome::Rejected,
        _ => return None,
    };
    let branch = to.strip_prefix("refs/heads/")?.to_owned();
    Some(PushStatus {
        branch,
        outcome,
        summary,
    })
}

/// Runs `command` with `stdin`, returning its output whatever the exit status.
fn run(command: &mut Command, stdin: &[u8]) -> Result<Output, Error> {
    let mut child = command
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
    let written = writer.join().expect("stdin writer doesn't panic");
    // A command may exit without reading all of stdin; only report that if it also failed.
    if !output.status.success() {
        written.map_err(Error::git)?;
    }
    Ok(output)
}

/// As [`run`], failing on a non-zero exit status. Returns stdout.
fn run_ok(command: &mut Command, stdin: &[u8]) -> Result<Vec<u8>, Error> {
    let output = run(command, stdin)?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(command_error(command, &output))
    }
}

fn command_error(command: &Command, output: &Output) -> Error {
    let args: Vec<_> = command
        .get_args()
        .map(|arg| arg.to_string_lossy())
        .collect();
    let stderr = String::from_utf8_lossy(&output.stderr);
    Error::git(format!("git {}: {}", args.join(" "), stderr.trim()))
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
        GixRepo::discover(&fixture.path(), Environment::Exactly(fixture.environment())).unwrap()
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
        repo.update_refs(
            &[RefUpdate {
                name: "refs/stack/test".into(),
                old: None,
                new: Some(blob.clone()),
            }],
            "",
        )
        .unwrap();
        let before = fixture.snapshot();

        let stale = RefUpdate {
            name: "refs/stack/test".into(),
            old: Some(other),
            new: None,
        };
        assert!(repo.update_refs(&[stale], "").is_err());
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
        assert!(repo(&fixture).update_refs(&[empty], "").is_err());
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
        assert!(repo.update_refs(&[good, bad], "").is_err());
        fixture.assert_unchanged(&before);
    }
}
