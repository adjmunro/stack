//! Importing stacks from other tools.

use super::*;

/// Graphite's per-branch metadata ref prefix.
const GRAPHITE_BRANCHES: &str = "refs/branch-metadata/";

impl Workspace {
    /// Imports Graphite's stacks: its trunks (from `.git/.graphite_repo_config`) become trunks, and its recorded
    /// parents (`refs/branch-metadata/*`) become recorded (unpinned) parents, so the commit graph still wins where
    /// Graphite's tracking has gone stale. Graphite's own data is left untouched; parents `stack` has already
    /// recorded are kept. One journalled command, so undo reverts it.
    ///
    /// # Errors
    /// [`Error::CorruptMetadata`] if Graphite's metadata can't be read.
    pub fn import_graphite(&self) -> Result<Imported, Error> {
        let metadata = Metadata::load(&*self.git)?;
        let mut imported = Imported::default();
        let mut updates = Vec::new();

        let mut trunks: Vec<String> = metadata.marks.keys().cloned().collect();
        for trunk in graphite_trunks(&self.git.common_dir())? {
            if metadata.marks.contains_key(&trunk) {
                continue;
            }
            if self.git.branch_tip(&trunk)?.is_none() {
                imported.skipped.push(Skipped {
                    branch: trunk,
                    reason: "trunk doesn't exist".into(),
                });
                continue;
            }
            let blob = self
                .git
                .write_blob(&metadata::encode(&Mark { role: Role::Trunk }))?;
            updates.push(RefUpdate {
                name: format!("{}{trunk}", metadata::TRUNKS),
                old: None,
                new: Some(blob),
            });
            trunks.push(trunk.clone());
            imported.trunks.push(trunk);
        }

        for blob in self.git.blob_refs(GRAPHITE_BRANCHES)? {
            let branch = blob.name.trim_start_matches(GRAPHITE_BRANCHES).to_owned();
            let skip = |reason: &str| Skipped {
                branch: branch.clone(),
                reason: reason.into(),
            };
            let json: serde_json::Value =
                serde_json::from_slice(&blob.data).map_err(|error| Error::CorruptMetadata {
                    reference: blob.name.clone(),
                    reason: error.to_string(),
                })?;
            let Some(parent) = json
                .get("parentBranchName")
                .and_then(|value| value.as_str())
            else {
                imported.skipped.push(skip("no parent recorded"));
                continue;
            };
            let revision = json
                .get("parentBranchRevision")
                .and_then(|value| value.as_str());
            let (Some(tip), Some(parent_tip)) =
                (self.git.branch_tip(&branch)?, self.git.branch_tip(parent)?)
            else {
                imported
                    .skipped
                    .push(skip("it or its parent no longer exists"));
                continue;
            };
            if trunks.contains(&branch) {
                imported.skipped.push(skip("it's a trunk"));
                continue;
            }
            if metadata.links.contains_key(&branch) {
                imported
                    .skipped
                    .push(skip("stack already records its parent"));
                continue;
            }
            // Graphite's recorded fork point, if the branch is still built on it; otherwise where they meet.
            let offshoot = match revision {
                Some(revision) if self.git.is_ancestor(revision, &tip).unwrap_or(false) => {
                    Some(revision.to_owned())
                }
                _ => self.git.merge_base(&tip, &parent_tip)?,
            };
            let Some(offshoot) = offshoot else {
                imported
                    .skipped
                    .push(skip("it shares no history with its parent"));
                continue;
            };
            let link = Link {
                parent: parent.to_owned(),
                offshoot,
                pinned: false,
            };
            let blob = self.git.write_blob(&metadata::encode(&link))?;
            updates.push(RefUpdate {
                name: format!("{}{branch}", metadata::BRANCHES),
                old: None,
                new: Some(blob),
            });
            imported.parents.push(ImportedParent {
                branch,
                parent: parent.to_owned(),
            });
        }

        imported.outcome = if updates.is_empty() {
            Outcome::Unchanged
        } else {
            self.journal.transact(
                &*self.git,
                OperationKind::Command,
                "import graphite",
                None,
                updates,
            )?;
            Outcome::Changed
        };
        Ok(imported)
    }
}

/// The trunks in Graphite's repo config: `{"trunk": "main"}`, or `{"trunks": [{"name": "main"}, …]}`.
fn graphite_trunks(common_dir: &Path) -> Result<Vec<String>, Error> {
    let path = common_dir.join(".graphite_repo_config");
    let Ok(data) = std::fs::read(&path) else {
        return Ok(Vec::new());
    };
    let json: serde_json::Value =
        serde_json::from_slice(&data).map_err(|error| Error::CorruptMetadata {
            reference: path.display().to_string(),
            reason: error.to_string(),
        })?;
    let mut trunks: Vec<String> = json
        .get("trunk")
        .and_then(|trunk| trunk.as_str())
        .map(str::to_owned)
        .into_iter()
        .collect();
    if let Some(list) = json.get("trunks").and_then(|trunks| trunks.as_array()) {
        let names = list.iter().filter_map(|trunk| {
            trunk
                .get("name")
                .and_then(|name| name.as_str())
                .or(trunk.as_str())
        });
        trunks.extend(names.map(str::to_owned));
    }
    trunks.sort();
    trunks.dedup();
    Ok(trunks)
}
