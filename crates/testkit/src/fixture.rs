use std::cell::Cell;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;

use crate::Snapshot;

/// Seconds since the epoch for the first git call; each call advances the clock by one second.
const EPOCH: u64 = 1_700_000_000;

/// A throwaway git repo in a temp dir, deleted on drop. Its trunk is `develop`.
///
/// All methods panic on failure, with the failing command and its output.
pub struct Fixture {
    dir: TempDir,
    clock: Cell<u64>,
}

impl Fixture {
    /// Creates an empty repo with an unborn `develop` branch.
    pub fn new() -> Self {
        let dir = tempfile::Builder::new()
            .prefix("stack-fixture-")
            .tempdir()
            .expect("create temp dir");
        fs::create_dir(dir.path().join("home")).expect("create fixture home");
        fs::create_dir(dir.path().join("repo")).expect("create fixture repo");
        let fixture = Self {
            dir,
            clock: Cell::new(EPOCH),
        };
        fixture.git(&["init", "--quiet", "--initial-branch=develop"]);
        fixture
    }

    /// The repo's working-tree root.
    pub fn path(&self) -> PathBuf {
        self.dir.path().join("repo")
    }

    /// Runs `git` in the repo and returns trimmed stdout.
    pub fn git(&self, args: &[&str]) -> String {
        let output = self.command("git").args(args).output().expect("spawn git");
        assert!(
            output.status.success(),
            "git {args:?} failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout)
            .expect("git stdout is UTF-8")
            .trim()
            .to_owned()
    }

    /// Writes `contents` to `path` (relative to the repo root), creating parent directories.
    pub fn write(&self, path: &str, contents: &str) {
        let path = self.path().join(path);
        fs::create_dir_all(path.parent().expect("file has a parent")).expect("create parent dirs");
        fs::write(path, contents).expect("write file");
    }

    /// Writes a file, stages it, and commits. Returns the new commit's full SHA.
    pub fn commit(&self, path: &str, contents: &str, message: &str) -> String {
        self.write(path, contents);
        self.git(&["add", "--", path]);
        self.git(&["commit", "--quiet", "--message", message]);
        self.git(&["rev-parse", "HEAD"])
    }

    /// Builds a command for `program` that runs in the repo with the fixture's isolated environment.
    ///
    /// Use this to run the `stack` binary under test.
    pub fn command(&self, program: impl AsRef<std::ffi::OsStr>) -> Command {
        let now = self.clock.get();
        self.clock.set(now + 1);
        let date = format!("{now} +0000");
        let home = self.dir.path().join("home");

        let mut command = Command::new(program);
        command
            .current_dir(self.path())
            // Clearing the environment drops inherited GIT_DIR, GIT_WORK_TREE, etc., which would otherwise
            // redirect git (and gix) at whatever repo the test runner was launched from.
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("HOME", &home)
            .env("XDG_CONFIG_HOME", home.join(".config"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", home.join(".gitconfig"))
            .env("GIT_CEILING_DIRECTORIES", self.dir.path())
            .env("GIT_AUTHOR_NAME", "Fixture Author")
            .env("GIT_AUTHOR_EMAIL", "author@fixture.invalid")
            .env("GIT_AUTHOR_DATE", &date)
            .env("GIT_COMMITTER_NAME", "Fixture Committer")
            .env("GIT_COMMITTER_EMAIL", "committer@fixture.invalid")
            .env("GIT_COMMITTER_DATE", &date)
            .env("LC_ALL", "C");
        command
    }

    /// Captures the repo's current state.
    pub fn snapshot(&self) -> Snapshot {
        Snapshot::capture(self)
    }

    /// Panics with a readable diff if anything in the repo changed since `before`.
    pub fn assert_unchanged(&self, before: &Snapshot) {
        let diff = before.diff(&self.snapshot());
        assert!(diff.is_empty(), "repo changed unexpectedly:\n{diff}");
    }

    pub(crate) fn root(&self) -> &Path {
        self.dir.path()
    }
}

impl Default for Fixture {
    fn default() -> Self {
        Self::new()
    }
}
