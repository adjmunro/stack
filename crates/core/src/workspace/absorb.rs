//! Absorbing staged changes into the commits that last touched the same lines.

use super::*;

/// One file's part of a `-U0` patch: its header lines and hunks.
struct FilePatch {
    header: Vec<Vec<u8>>,
    path: String,
    hunks: Vec<Hunk>,
}

struct Hunk {
    /// The hunk's lines, `@@` header first.
    lines: Vec<Vec<u8>>,
    old_start: usize,
    old_count: usize,
}

impl Workspace {
    /// Folds each staged hunk into the commit in the current stack that last changed the lines it changes (by
    /// `git blame`), as [`Self::amend_into`] would. Hunks that only add lines, change lines from several commits, or
    /// change lines from outside the stack stay staged, as do hunks whose amend would conflict. The working tree
    /// isn't touched. Each target commit is its own journalled amend, nearest to HEAD first.
    ///
    /// # Errors
    /// [`Error::DetachedHead`] or [`Error::NothingStaged`].
    pub fn absorb(&self) -> Result<Absorbed, Error> {
        let Head::Branch {
            name: current,
            commit: head,
        } = self.git.head()?
        else {
            return Err(Error::DetachedHead);
        };
        let head = head.ok_or(Error::NothingStaged)?;
        let original = self.git.commit(&head)?.tree;
        let files = parse_patch(&self.git.staged_patch()?);
        if files.is_empty() {
            return Err(Error::NothingStaged);
        }

        // The stack's own commits, nearest to HEAD first.
        let (_, resolution) = self.resolve()?;
        let mut stack_commits = Vec::new();
        for branch in resolution.lineage(&current) {
            let entry = &resolution.branches[branch];
            let Some(parent) = &entry.parent else { break };
            stack_commits.extend(
                self.git
                    .commits_excluding(&entry.tip, std::slice::from_ref(&parent.offshoot))?,
            );
        }

        let mut absorbed = Absorbed::default();
        let mut groups: BTreeMap<usize, Vec<(usize, usize)>> = BTreeMap::new();
        let mut left: Vec<(usize, usize)> = Vec::new();
        for (file_index, file) in files.iter().enumerate() {
            for (hunk_index, hunk) in file.hunks.iter().enumerate() {
                let reason = if hunk.old_count == 0 {
                    Some("only adds lines")
                } else {
                    let mut blamed =
                        self.git
                            .blame(&head, &file.path, hunk.old_start, hunk.old_count)?;
                    blamed.sort();
                    blamed.dedup();
                    match blamed.as_slice() {
                        [commit] => match stack_commits.iter().position(|own| own == commit) {
                            Some(position) => {
                                groups
                                    .entry(position)
                                    .or_default()
                                    .push((file_index, hunk_index));
                                None
                            }
                            None => Some("changes lines from outside this stack"),
                        },
                        _ => Some("changes lines from several commits"),
                    }
                };
                if let Some(reason) = reason {
                    left.push((file_index, hunk_index));
                    absorbed.left.push(StagedHunk {
                        path: file.path.clone(),
                        lines: hunk.range(),
                        reason: Some(reason.into()),
                    });
                }
            }
        }

        for (position, hunks) in groups {
            let target = &stack_commits[position];
            let changed = self
                .git
                .apply_to_tree(&original, &assemble(&files, &hunks))?;
            match self.amend_change(&current, target, &original, &changed, target) {
                Ok(amended) => {
                    for (file_index, hunk_index) in &hunks {
                        let (file, hunk) =
                            (&files[*file_index], &files[*file_index].hunks[*hunk_index]);
                        absorbed.absorbed.push(AbsorbedHunk {
                            path: file.path.clone(),
                            lines: hunk.range(),
                            commit: target.clone(),
                        });
                    }
                    absorbed.amended.push(amended);
                }
                Err(Error::AmendConflict { .. }) => {
                    for (file_index, hunk_index) in &hunks {
                        let (file, hunk) =
                            (&files[*file_index], &files[*file_index].hunks[*hunk_index]);
                        let reason = Some("would conflict with later commits".to_owned());
                        absorbed.left.push(StagedHunk {
                            path: file.path.clone(),
                            lines: hunk.range(),
                            reason,
                        });
                    }
                    left.extend(hunks);
                }
                Err(error) => return Err(error),
            }
        }

        // Re-stage what wasn't absorbed, on top of the new HEAD.
        if !absorbed.amended.is_empty() {
            let Head::Branch {
                commit: Some(new_head),
                ..
            } = self.git.head()?
            else {
                return Ok(absorbed);
            };
            let new_tree = self.git.commit(&new_head)?.tree;
            let index = if left.is_empty() {
                new_tree
            } else {
                let leftover = self
                    .git
                    .apply_to_tree(&original, &assemble(&files, &left))?;
                match self.git.merge_trees(&original, &new_tree, &leftover)? {
                    Merge::Clean { tree } => tree,
                    // Still in the working tree; just no longer staged.
                    Merge::Conflicted { .. } => new_tree,
                }
            };
            self.git.set_index(&index)?;
        }
        Ok(absorbed)
    }
}

impl Hunk {
    /// Where it is in the old file: `line 3`, `lines 3-5`, `after line 3`, or `at the top`.
    fn range(&self) -> String {
        match (self.old_start, self.old_count) {
            (0, 0) => "at the top".to_owned(),
            (start, 0) => format!("after line {start}"),
            (start, 1) => format!("line {start}"),
            (start, count) => format!("lines {start}-{}", start + count - 1),
        }
    }
}

/// Splits a `git diff -U0` patch into files and hunks. Files without hunks (binary, mode-only) are skipped.
fn parse_patch(patch: &[u8]) -> Vec<FilePatch> {
    let mut files: Vec<FilePatch> = Vec::new();
    for line in patch.split_inclusive(|byte| *byte == b'\n') {
        if line.starts_with(b"diff --git ") {
            files.push(FilePatch {
                header: vec![line.to_vec()],
                path: String::new(),
                hunks: Vec::new(),
            });
            continue;
        }
        let Some(file) = files.last_mut() else {
            continue;
        };
        if line.starts_with(b"@@ ") {
            let (old_start, old_count) = parse_hunk_header(line);
            file.hunks.push(Hunk {
                lines: vec![line.to_vec()],
                old_start,
                old_count,
            });
        } else if let Some(hunk) = file.hunks.last_mut() {
            hunk.lines.push(line.to_vec());
        } else {
            // The new path, or the old one for a deleted file (whose new side is /dev/null).
            if let Some(path) = line
                .strip_prefix(b"--- a/")
                .or_else(|| line.strip_prefix(b"+++ b/"))
            {
                file.path = String::from_utf8_lossy(path).trim_end().to_owned();
            }
            file.header.push(line.to_vec());
        }
    }
    files.retain(|file| !file.hunks.is_empty() && !file.path.is_empty());
    files
}

/// `@@ -a[,b] +c[,d] @@` → `(a, b)`, where `b` defaults to 1.
fn parse_hunk_header(line: &[u8]) -> (usize, usize) {
    let text = String::from_utf8_lossy(line);
    let old = text
        .split_whitespace()
        .nth(1)
        .unwrap_or("-0")
        .trim_start_matches('-');
    let (start, count) = old.split_once(',').unwrap_or((old, "1"));
    (start.parse().unwrap_or(0), count.parse().unwrap_or(1))
}

/// A patch holding just `hunks` (`(file, hunk)` indexes), with their files' headers.
fn assemble(files: &[FilePatch], hunks: &[(usize, usize)]) -> Vec<u8> {
    let mut patch = Vec::new();
    for (file_index, file) in files.iter().enumerate() {
        let chosen: Vec<&Hunk> = hunks
            .iter()
            .filter(|(index, _)| *index == file_index)
            .map(|(_, hunk)| &file.hunks[*hunk])
            .collect();
        if chosen.is_empty() {
            continue;
        }
        file.header
            .iter()
            .for_each(|line| patch.extend_from_slice(line));
        chosen
            .iter()
            .flat_map(|hunk| &hunk.lines)
            .for_each(|line| patch.extend_from_slice(line));
    }
    patch
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hunk_headers_default_their_counts_to_one() {
        assert_eq!(parse_hunk_header(b"@@ -2 +2 @@ a\n"), (2, 1));
        assert_eq!(parse_hunk_header(b"@@ -5,3 +5,0 @@\n"), (5, 3));
        assert_eq!(parse_hunk_header(b"@@ -7,0 +8,2 @@\n"), (7, 0));
    }
}
