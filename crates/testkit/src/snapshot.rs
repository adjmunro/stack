use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::path::Path;

use crate::Fixture;

/// Everything about a repo an operation could change: HEAD, refs, config, index, working tree, object database, and
/// `stack`'s own files in `.git/stack/`.
///
/// Captured with read-only `git` plumbing, independent of the `gix` code under test.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    head: String,
    refs: BTreeMap<String, String>,
    config: String,
    index: BTreeMap<String, String>,
    worktree: BTreeMap<String, Vec<u8>>,
    objects: BTreeSet<String>,
    stack: BTreeMap<String, Vec<u8>>,
}

impl Snapshot {
    pub(crate) fn capture(fixture: &Fixture) -> Self {
        let head = match fixture
            .command("git")
            .args(["symbolic-ref", "--quiet", "HEAD"])
            .output()
        {
            Ok(output) if output.status.success() => {
                String::from_utf8_lossy(&output.stdout).trim().to_owned()
            }
            _ => fixture.git(&["rev-parse", "HEAD"]),
        };
        let refs = lines(&fixture.git(&["for-each-ref", "--format=%(refname) %(objectname)"]))
            .map(|line| split_pair(line, ' '))
            .collect();
        let index = lines(
            &fixture
                .git(&["ls-files", "--stage", "-z"])
                .replace('\0', "\n"),
        )
        .map(|line| {
            let (meta, path) = split_pair(line, '\t');
            (path, meta)
        })
        .collect();
        let objects = lines(&fixture.git(&[
            "cat-file",
            "--batch-all-objects",
            "--batch-check=%(objectname)",
        ]))
        .map(str::to_owned)
        .collect();
        let git_dir = fixture.root().join("repo/.git");
        let config = fs::read_to_string(git_dir.join("config")).expect("read .git/config");
        let mut worktree = BTreeMap::new();
        walk(&fixture.path(), &fixture.path(), &mut worktree);
        let stack_dir = git_dir.join("stack");
        let mut stack = BTreeMap::new();
        if stack_dir.is_dir() {
            walk(&stack_dir, &stack_dir, &mut stack);
        }
        Self {
            head,
            refs,
            config,
            index,
            worktree,
            objects,
            stack,
        }
    }

    /// Describes every difference from `self` to `after`.
    pub fn diff(&self, after: &Snapshot) -> SnapshotDiff {
        SnapshotDiff {
            head: (self.head != after.head).then(|| (self.head.clone(), after.head.clone())),
            refs: diff_maps(&self.refs, &after.refs),
            config: self.config != after.config,
            index: diff_maps(&self.index, &after.index).into_keys().collect(),
            worktree: diff_maps(&self.worktree, &after.worktree)
                .into_keys()
                .collect(),
            objects_added: after.objects.difference(&self.objects).cloned().collect(),
            objects_removed: self.objects.difference(&after.objects).cloned().collect(),
            stack: diff_maps(&self.stack, &after.stack).into_keys().collect(),
        }
    }
}

/// Differences between two [`Snapshot`]s. Ref values are `None` where the ref is absent.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SnapshotDiff {
    pub head: Option<(String, String)>,
    pub refs: BTreeMap<String, (Option<String>, Option<String>)>,
    pub config: bool,
    pub index: BTreeSet<String>,
    pub worktree: BTreeSet<String>,
    pub objects_added: BTreeSet<String>,
    pub objects_removed: BTreeSet<String>,
    /// Files under `.git/stack/` that were added, removed, or changed.
    pub stack: BTreeSet<String>,
}

impl SnapshotDiff {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

impl fmt::Display for SnapshotDiff {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some((before, after)) = &self.head {
            writeln!(f, "HEAD: {before} -> {after}")?;
        }
        for (name, (before, after)) in &self.refs {
            let show = |value: &Option<String>| value.clone().unwrap_or_else(|| "(none)".into());
            writeln!(f, "ref {name}: {} -> {}", show(before), show(after))?;
        }
        if self.config {
            writeln!(f, ".git/config changed")?;
        }
        for path in &self.index {
            writeln!(f, "index: {path}")?;
        }
        for path in &self.worktree {
            writeln!(f, "worktree: {path}")?;
        }
        for path in &self.stack {
            writeln!(f, "stack: {path}")?;
        }
        writeln!(
            f,
            "objects: +{} -{}",
            self.objects_added.len(),
            self.objects_removed.len()
        )
    }
}

fn lines(text: &str) -> impl Iterator<Item = &str> {
    text.lines().filter(|line| !line.is_empty())
}

fn split_pair(line: &str, separator: char) -> (String, String) {
    let (left, right) = line
        .split_once(separator)
        .unwrap_or_else(|| panic!("malformed git output: {line:?}"));
    (left.to_owned(), right.to_owned())
}

fn diff_maps<V: Clone + PartialEq>(
    before: &BTreeMap<String, V>,
    after: &BTreeMap<String, V>,
) -> BTreeMap<String, (Option<V>, Option<V>)> {
    before
        .keys()
        .chain(after.keys())
        .filter(|key| before.get(*key) != after.get(*key))
        .map(|key| {
            (
                key.clone(),
                (before.get(key).cloned(), after.get(key).cloned()),
            )
        })
        .collect()
}

fn walk(root: &Path, dir: &Path, files: &mut BTreeMap<String, Vec<u8>>) {
    for entry in fs::read_dir(dir).expect("read worktree dir") {
        let path = entry.expect("read worktree entry").path();
        if path == root.join(".git") {
            continue;
        }
        if path.is_dir() {
            walk(root, &path, files);
        } else {
            let relative = path
                .strip_prefix(root)
                .expect("path under root")
                .to_string_lossy()
                .into_owned();
            files.insert(relative, fs::read(&path).expect("read worktree file"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshots_of_an_untouched_repo_are_equal() {
        let fixture = Fixture::new();
        fixture.commit("a.txt", "a", "feat: a");
        let before = fixture.snapshot();
        fixture.assert_unchanged(&before);
    }

    #[test]
    fn diff_reports_exactly_what_a_commit_changed() {
        let fixture = Fixture::new();
        let first = fixture.commit("a.txt", "a", "feat: a");
        let before = fixture.snapshot();

        let second = fixture.commit("b.txt", "b", "feat: b");
        let diff = before.diff(&fixture.snapshot());

        assert_eq!(diff.head, None);
        assert_eq!(
            diff.refs,
            BTreeMap::from([("refs/heads/develop".into(), (Some(first), Some(second)))])
        );
        assert!(!diff.config);
        assert_eq!(diff.index, BTreeSet::from(["b.txt".into()]));
        assert_eq!(diff.worktree, BTreeSet::from(["b.txt".into()]));
        // blob, tree, commit
        assert_eq!(diff.objects_added.len(), 3);
        assert!(diff.objects_removed.is_empty());
    }

    #[test]
    fn identical_scripts_produce_identical_shas() {
        let script = || Fixture::new().commit("a.txt", "a", "feat: a");
        assert_eq!(script(), script());
    }
}
