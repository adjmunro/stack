//! Human-readable output.

use super::*;

pub(crate) fn describe_sync_all(synced: &Synced, fetched: bool) -> String {
    let mut lines = Vec::new();
    if let (Some(remote), true) = (&synced.remote, fetched) {
        lines.push(format!("Fetched {remote}"));
    }
    let remote = synced.remote.as_deref().unwrap_or("the remote");
    for trunk in &synced.trunks {
        let name = &trunk.name;
        match &trunk.outcome {
            TrunkUpdate::FastForwarded { to, .. } => {
                lines.push(format!(
                    "Fast-forwarded {name} to {remote}/{name} ({})",
                    short(to)
                ));
            }
            TrunkUpdate::Ahead => lines.push(format!(
                "{name} has commits {remote} doesn't; left as it is"
            )),
            TrunkUpdate::Diverged => lines.push(format!(
                "{name} has diverged from {remote}/{name}; left as it is"
            )),
            TrunkUpdate::UpToDate | TrunkUpdate::NoUpstream => {}
        }
    }
    let restack = Restacked {
        outcome: Outcome::Unchanged,
        moved: synced.moved.clone(),
        conflicts: synced.conflicts.clone(),
        blocked: synced.blocked.clone(),
    };
    let moves = describe_restack(&restack, true);
    let nothing_moved = synced.moved.is_empty() && synced.conflicts.is_empty();
    lines.extend(synced.archived.iter().map(|branch| {
        format!("Archived merged {branch} (`stack unarchive {branch}` restores it)")
    }));
    lines.extend(
        synced
            .kept
            .iter()
            .map(|kept| format!("Kept merged {}: {}", kept.branch, kept.reason)),
    );
    if !nothing_moved {
        lines.push(moves);
    } else if synced.archived.is_empty()
        && synced.kept.is_empty()
        && !lines.iter().any(|line| line.starts_with("Fast-forwarded"))
    {
        lines.push("Everything is up to date.".to_owned());
    }
    lines.join("\n")
}

pub(crate) fn describe_stopped(branch: &str, paths: &[String]) -> String {
    format!(
        "Resolving {branch}: fix the conflicts in {}, `git add` them, then run `stack continue` (or `stack abort`)",
        paths.join(", ")
    )
}

/// Describes a restack; with `manual`, each conflict comes with the commands to finish it by hand.
pub(crate) fn describe_restack(restacked: &Restacked, manual: bool) -> String {
    let mut lines: Vec<String> = restacked
        .moved
        .iter()
        .map(|moved| {
            format!(
                "Restacked {} onto {} ({})",
                moved.name,
                moved.onto,
                count(moved.replayed, moved.dropped, &moved.onto)
            )
        })
        .collect();
    for conflict in &restacked.conflicts {
        lines.push(describe_conflict(conflict));
        if !manual {
            continue;
        }
        lines.push(format!(
            "Left {} and the branches on it as they were. To finish:",
            conflict.branch
        ));
        lines.push(format!(
            "  git rebase --onto {} {} {}",
            conflict.onto,
            short(&conflict.offshoot),
            conflict.branch
        ));
        lines.push(format!("  stack restack {}", conflict.branch));
    }
    if restacked.moved.is_empty() && restacked.conflicts.is_empty() {
        lines.push("Everything is up to date.".to_owned());
    }
    lines.join("\n")
}

pub(crate) fn describe_preview(preview: &RestackPreview) -> String {
    let mut lines: Vec<String> = preview
        .clean
        .iter()
        .map(|moved| {
            format!(
                "{} restacks cleanly onto {} ({})",
                moved.name,
                moved.onto,
                count(moved.replayed, moved.dropped, &moved.onto)
            )
        })
        .collect();
    lines.extend(preview.conflicts.iter().map(describe_conflict));
    lines.extend(preview.blocked.iter().map(|branch| {
        format!("{branch} can't be checked until the conflict below it is resolved")
    }));
    if lines.is_empty() {
        lines.push("Everything is up to date.".to_owned());
    }
    lines.join("\n")
}

pub(crate) fn mark_name(kind: MarkKind) -> &'static str {
    match kind {
        MarkKind::Reviewed => "reviewed",
        MarkKind::Tested => "tested",
        MarkKind::Flagged => "flagged",
    }
}

pub(crate) fn describe_review(review: &[CommitReview]) -> String {
    if review.is_empty() {
        return "No commits of its own.".to_owned();
    }
    let lines: Vec<String> = review
        .iter()
        .map(|commit| {
            let marks: Vec<String> = commit
                .marks
                .iter()
                .map(|mark| match &mark.note {
                    Some(note) => format!("{}: {note}", mark_name(mark.kind)),
                    None => mark_name(mark.kind).to_owned(),
                })
                .collect();
            let marks = if marks.is_empty() {
                String::new()
            } else {
                format!(" [{}]", marks.join(", "))
            };
            format!("{} {}{marks}", short(&commit.commit), commit.summary)
        })
        .collect();
    lines.join("\n")
}

pub(crate) fn describe_worktree(worktree: &Worktree) -> String {
    let marker = if worktree.current { "* " } else { "" };
    let path = worktree.path.display();
    match (&worktree.branch, &worktree.follows, &worktree.head) {
        (Some(branch), _, _) => format!("{marker}{path} {branch}"),
        (None, Some(following), _) => {
            let position = match following.position {
                FollowPosition::UpToDate => "up to date",
                FollowPosition::Behind => "behind; run `stack worktree sync`",
                FollowPosition::Ahead => "has commits; run `stack land`",
                FollowPosition::Orphaned => "its branch is gone",
            };
            format!("{marker}{path} following {} ({position})", following.branch)
        }
        (None, None, Some(head)) => format!("{marker}{path} detached at {}", short(head)),
        (None, None, None) => format!("{marker}{path}"),
    }
}

pub(crate) fn describe_sync(sync: &FollowerSync) -> String {
    let path = sync.path.display();
    match &sync.outcome {
        SyncOutcome::Moved { to, .. } => {
            format!("Moved follower {path} to {} ({})", sync.branch, short(to))
        }
        SyncOutcome::UpToDate => format!("{path} is up to date with {}", sync.branch),
        SyncOutcome::Skipped { reason } => format!("Left {path}: {reason}"),
    }
}

/// "5 minutes ago", "3 hours ago", "2 days ago".
pub(crate) fn ago(seconds_since_epoch: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs() as i64);
    let elapsed = (now - seconds_since_epoch).max(0);
    let (count, unit) = match elapsed {
        0..60 => return "just now".to_owned(),
        60..3_600 => (elapsed / 60, "minute"),
        3_600..86_400 => (elapsed / 3_600, "hour"),
        _ => (elapsed / 86_400, "day"),
    };
    let plural = if count == 1 { "" } else { "s" };
    format!("{count} {unit}{plural} ago")
}

pub(crate) fn describe_proposal(proposal: &ProposedBranch) -> String {
    let number = proposal
        .number
        .map(|number| format!("#{number}"))
        .unwrap_or_default();
    let (branch, base) = (&proposal.branch, &proposal.base);
    let url = proposal.url.as_deref().unwrap_or_default();
    match &proposal.action {
        ProposalAction::Created => format!("Opened {number} for {branch} into {base}: {url}"),
        ProposalAction::Retargeted { from } => {
            format!("Retargeted {number} ({branch}) from {from} to {base}")
        }
        ProposalAction::UpToDate => format!("{number} ({branch} into {base}) is up to date"),
        ProposalAction::Skipped { reason } => format!("Skipped {branch}: {reason}"),
    }
}

pub(crate) fn describe_push(pushed: &Pushed) -> String {
    if pushed.branches.is_empty() {
        return "Nothing to push.".to_owned();
    }
    let remote = &pushed.remote;
    let lines: Vec<String> = pushed
        .branches
        .iter()
        .map(|branch| {
            let name = &branch.name;
            match branch.outcome {
                PushOutcome::Created => format!("Pushed {name} to {remote} (new)"),
                PushOutcome::FastForwarded => format!("Pushed {name} to {remote}"),
                PushOutcome::Forced => {
                    format!("Pushed {name} to {remote} (replaced its rewritten remote branch)")
                }
                PushOutcome::UpToDate => format!("{name} is up to date on {remote}"),
                PushOutcome::Rejected => format!("Rejected {name}: {}", branch.summary),
            }
        })
        .collect();
    lines.join("\n")
}

pub(crate) fn describe_conflict(conflict: &Conflict) -> String {
    format!(
        "Conflict: {} \"{}\" ({}) conflicts with {} in {}",
        conflict.branch,
        conflict.summary,
        short(&conflict.commit),
        conflict.onto,
        conflict.paths.join(", ")
    )
}

/// "2 commits", "1 commit; 1 already in develop".
pub(crate) fn count(replayed: usize, dropped: usize, onto: &str) -> String {
    let plural = if replayed == 1 { "" } else { "s" };
    match dropped {
        0 => format!("{replayed} commit{plural}"),
        dropped => format!("{replayed} commit{plural}; {dropped} already in {onto}"),
    }
}

pub(crate) fn describe_operation(operation: &Operation) -> String {
    let mut notes = Vec::new();
    if operation.undone {
        notes.push("undone");
    }
    match operation.state {
        OperationState::Pending => notes.push("interrupted"),
        OperationState::Failed => notes.push("failed"),
        OperationState::Done => {}
    }
    let notes = if notes.is_empty() {
        String::new()
    } else {
        format!(" ({})", notes.join(", "))
    };
    format!("#{} {}{notes}", operation.id, operation.description)
}

pub(crate) fn describe_marked(marked: &Marked) -> String {
    let name = &marked.name;
    let stacked = marked
        .parent
        .as_ref()
        .map(|parent| format!(" (stacked on {parent})"))
        .unwrap_or_default();
    match marked.outcome {
        Outcome::Changed => format!("Added trunk {name}{stacked}"),
        Outcome::Unchanged => format!("{name} is already a trunk{stacked}"),
    }
}

pub(crate) fn describe(head: &Head) -> String {
    match head {
        Head::Branch {
            name,
            commit: Some(commit),
        } => format!("On branch {name} ({})", short(commit)),
        Head::Branch { name, commit: None } => format!("On branch {name} (no commits yet)"),
        Head::Detached { commit } => format!("HEAD detached at {}", short(commit)),
    }
}

/// Draws the tree with box-drawing guides. The current branch is marked `*`.
pub(crate) fn render(tree: &Tree, preview: Option<&RestackPreview>) -> String {
    let mut lines = Vec::new();
    if tree.trunks.is_empty() {
        lines.push("No trunks. Add one with `stack trunk add <branch>`.".to_owned());
    }
    for trunk in &tree.trunks {
        render_node(trunk, preview, "", "", &mut lines);
    }
    if !tree.trunks.is_empty() && !tree.unattached.is_empty() {
        lines.push(String::new());
        lines.push("Unattached:".to_owned());
        for node in &tree.unattached {
            render_node(node, preview, "", "", &mut lines);
        }
    }
    lines.join("\n")
}

pub(crate) fn render_node(
    node: &Node,
    preview: Option<&RestackPreview>,
    lead: &str,
    indent: &str,
    lines: &mut Vec<String>,
) {
    let marker = if node.current { "* " } else { "" };
    let mut notes = Vec::new();
    if node.role == Role::Limb {
        notes.push("[trunk]".to_owned());
    }
    if let Some(parent) = &node.parent {
        match (parent.source, parent.contradicted) {
            (Source::Pinned, false) => notes.push("[pinned]".to_owned()),
            (Source::Pinned, true) => notes.push("[pinned; graph disagrees]".to_owned()),
            _ => {}
        }
        if let Some(previous) = &parent.replaces {
            notes.push(format!("(was on {previous})"));
        }
        if parent.needs_restack {
            notes.push("(needs restack)".to_owned());
        }
    }
    if let Some(preview) = preview {
        if let Some(conflict) = preview
            .conflicts
            .iter()
            .find(|conflict| conflict.branch == node.name)
        {
            notes.push(format!(
                "(restack conflicts in {})",
                conflict.paths.join(", ")
            ));
        } else if preview.blocked.contains(&node.name) {
            notes.push("(restack blocked below)".to_owned());
        }
    }
    let notes = notes
        .iter()
        .map(|note| format!(" {note}"))
        .collect::<String>();
    lines.push(format!(
        "{lead}{marker}{} {}{notes}",
        node.name,
        short(&node.commit)
    ));
    for (index, child) in node.children.iter().enumerate() {
        let last = index + 1 == node.children.len();
        let (lead, next) = if last {
            ("└─ ", "   ")
        } else {
            ("├─ ", "│  ")
        };
        render_node(
            child,
            preview,
            &format!("{indent}{lead}"),
            &format!("{indent}{next}"),
            lines,
        );
    }
}

pub(crate) fn short(commit: &str) -> &str {
    &commit[..7.min(commit.len())]
}
