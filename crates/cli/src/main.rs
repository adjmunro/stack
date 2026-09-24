//! `stack`: stacked branches on top of git.

mod args;
mod describe;

use args::*;
use describe::*;

use std::io::{BufRead, IsTerminal, Write};

use std::path::{Path, PathBuf};

use std::process::ExitCode;

use clap::{Parser, Subcommand};

use serde_json::{Value, to_value};

use stack_core::{
    CommitRange, CommitReview, Conflict, Direction, Error, FollowPosition, FollowerSync,
    GuardViolation, Head, MarkKind, Marked, Node, Operation, OperationState, Outcome,
    ProposalAction, ProposedBranch, PushOutcome, Pushed, RecoveryOutcome, ResolveOutcome,
    RestackPreview, Restacked, Role, Scope, Source, Step, SyncOutcome, Synced, Tree, TrunkUpdate,
    Workspace, Worktree,
};

type Result<T, E = Box<dyn std::error::Error>> = std::result::Result<T, E>;

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            let mut source = error.source();
            while let Some(cause) = source {
                eprintln!("  caused by: {cause}");
                source = cause.source();
            }
            ExitCode::FAILURE
        }
    }
}

fn run(cli: &Cli) -> Result<()> {
    let directory = cli.directory.clone().unwrap_or_else(|| PathBuf::from("."));
    let print = |value: Value, human: String| {
        if cli.json {
            println!(
                "{}",
                serde_json::to_string_pretty(&value).expect("JSON values serialise")
            );
        } else {
            println!("{human}");
        }
    };
    if let Command::Init { trunks, yes } = &cli.command {
        let (value, human) = init(&directory, trunks, *yes)?;
        print(value, human);
        return Ok(());
    }
    let workspace = discover(&directory)?;
    match &cli.command {
        Command::Init { .. } => unreachable!("handled above"),
        Command::Status => {
            let status = workspace.status()?;
            let mut human = describe(&status.head);
            if let Some(waiting) = workspace.resolving()? {
                human.push_str(&format!(
                    "\nRestacking {}: waiting on a conflict in {} (`stack continue` or `stack abort`)",
                    waiting.target, waiting.branch
                ));
            }
            print(to_value(&status)?, human);
        }
        Command::Tree { check, paths } => {
            let tree = if paths.is_empty() {
                workspace.tree()?
            } else {
                workspace.tree_touching(paths)?
            };
            let preview = if *check {
                Some(workspace.check(None)?)
            } else {
                None
            };
            let unborn = matches!(workspace.status()?.head, Head::Branch { commit: None, .. });
            let human = if unborn && tree.trunks.is_empty() && tree.unattached.is_empty() {
                "No commits yet.".to_owned()
            } else {
                render(&tree, preview.as_ref())
            };
            let value = match &preview {
                Some(preview) => serde_json::json!({ "tree": tree, "preview": preview }),
                None => to_value(&tree)?,
            };
            print(value, human);
        }
        Command::Trunk(TrunkCommand::Add { name }) => {
            let marked = workspace.add_trunk(name)?;
            print(to_value(&marked)?, describe_marked(&marked));
        }
        Command::Trunk(TrunkCommand::Remove { name }) => {
            let role = workspace.remove_trunk(name)?;
            print(to_value(role)?, format!("Removed trunk {name}"));
        }
        Command::Pin { branch, parent } => {
            let branch = branch_or_current(&workspace, branch.as_deref())?;
            let pinned = workspace.pin(&branch, parent.as_deref())?;
            let human = match pinned.outcome {
                Outcome::Changed => format!("Pinned {branch} on {}", pinned.parent),
                Outcome::Unchanged => format!("{branch} is already pinned on {}", pinned.parent),
            };
            print(to_value(&pinned)?, human);
        }
        Command::Unpin { branch } => {
            let branch = branch_or_current(&workspace, branch.as_deref())?;
            let outcome = workspace.unpin(&branch)?;
            print(to_value(outcome)?, format!("Unpinned {branch}"));
        }
        Command::Restack { branch, no_resolve } => {
            let branch = branch_or_current(&workspace, branch.as_deref())?;
            let restacked = workspace.restack(&branch)?;
            finish_restack(&workspace, &branch, restacked, *no_resolve, &print)?;
        }
        Command::Sync { remote, no_fetch } => {
            let synced = workspace.sync(remote.as_deref(), !no_fetch)?;
            print(to_value(&synced)?, describe_sync_all(&synced, !no_fetch));
            note_follower_syncs(&workspace)?;
            if !synced.conflicts.is_empty() {
                return Err("sync stopped at a conflict".into());
            }
        }
        Command::Continue => {
            let target = workspace.resolving()?.map(|waiting| waiting.target);
            let outcome = workspace.continue_restack()?;
            match outcome {
                ResolveOutcome::Stopped { branch, paths } => {
                    print(
                        to_value(ResolveOutcome::Stopped {
                            branch: branch.clone(),
                            paths: paths.clone(),
                        })?,
                        describe_stopped(&branch, &paths),
                    );
                    return Err("restack stopped at a conflict".into());
                }
                ResolveOutcome::Finished(restacked) => {
                    let target = target.unwrap_or_default();
                    finish_restack(&workspace, &target, restacked, false, &print)?;
                }
            }
        }
        Command::Abort => {
            let aborted = workspace.abort_restack()?;
            print(
                to_value(&aborted)?,
                format!(
                    "Stopped resolving {}; branches already restacked stay restacked (`stack undo` reverts them)",
                    aborted.branch
                ),
            );
        }
        Command::Check { branch } => {
            let preview = workspace.check(branch.as_deref())?;
            print(to_value(&preview)?, describe_preview(&preview));
            if !preview.conflicts.is_empty() {
                return Err("a restack would conflict".into());
            }
        }
        Command::Move {
            branch,
            onto,
            no_resolve,
        } => {
            let branch = branch_or_current(&workspace, branch.as_deref())?;
            let restacked = workspace.move_branch(&branch, onto)?;
            finish_restack(&workspace, &branch, restacked, *no_resolve, &print)?;
        }
        Command::Push(line) => {
            let branch = branch_or_current(&workspace, line.branch.as_deref())?;
            let pushed = workspace.push(&branch, line.scope(), line.remote.as_deref())?;
            print(to_value(&pushed)?, describe_push(&pushed));
            if pushed
                .branches
                .iter()
                .any(|branch| branch.outcome == PushOutcome::Rejected)
            {
                return Err("some branches were rejected; fetch, restack, and push again".into());
            }
        }
        Command::Up { steps } => navigate(&workspace, Step::Up(*steps), &print)?,
        Command::Down { steps } => navigate(&workspace, Step::Down(*steps), &print)?,
        Command::Top => navigate(&workspace, Step::Top, &print)?,
        Command::Bottom => navigate(&workspace, Step::Bottom, &print)?,
        Command::Create { name, message, all } => {
            workspace.create(name, message.as_deref(), *all)?;
            print(
                serde_json::json!({ "created": name }),
                format!("Created {name}"),
            );
        }
        Command::Archive { branch } => {
            workspace.archive(branch)?;
            print(
                serde_json::json!({ "archived": branch }),
                format!("Archived {branch}"),
            );
        }
        Command::Unarchive { name } => {
            workspace.unarchive(name)?;
            print(
                serde_json::json!({ "unarchived": name }),
                format!("Restored {name}"),
            );
        }
        Command::Archived => {
            let archived = workspace.archived()?;
            let human = if archived.is_empty() {
                "No archived branches.".to_owned()
            } else {
                archived
                    .iter()
                    .map(|branch| format!("{} {}", branch.name, short(&branch.commit)))
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            print(to_value(&archived)?, human);
        }
        Command::Review { branch } => {
            let branch = branch_or_current(&workspace, branch.as_deref())?;
            let review = workspace.review(&branch)?;
            print(to_value(&review)?, describe_review(&review));
        }
        Command::Mark {
            revisions,
            branch,
            tested,
            flagged,
            note,
        } => {
            let kind = match (tested, flagged) {
                (true, _) => MarkKind::Tested,
                (_, true) => MarkKind::Flagged,
                _ => MarkKind::Reviewed,
            };
            let revisions: Vec<String> = match branch {
                Some(branch) => workspace
                    .review(branch)?
                    .into_iter()
                    .map(|commit| commit.commit)
                    .collect(),
                None if revisions.is_empty() => vec!["HEAD".to_owned()],
                None => revisions.clone(),
            };
            for revision in &revisions {
                workspace.mark(revision, kind, note.as_deref())?;
            }
            let plural = if revisions.len() == 1 { "" } else { "s" };
            let value = serde_json::json!({ "marked": revisions, "kind": kind });
            print(
                value,
                format!(
                    "Marked {} commit{plural} {}",
                    revisions.len(),
                    mark_name(kind)
                ),
            );
        }
        Command::Unmark { revisions, kind } => {
            let revisions = if revisions.is_empty() {
                vec!["HEAD".to_owned()]
            } else {
                revisions.clone()
            };
            let mut removed = 0;
            for revision in &revisions {
                removed += workspace.unmark(revision, kind.map(MarkKind::from))?;
            }
            let plural = if removed == 1 { "" } else { "s" };
            print(
                serde_json::json!({ "removed": removed }),
                format!("Removed {removed} mark{plural}"),
            );
        }
        Command::Worktree(WorktreeCommand::Add { branch, path }) => {
            let worktree = workspace.add_worktree(branch, path.as_deref())?;
            let human = match &worktree.follows {
                Some(_) => format!(
                    "Created follower of {branch} at {} ({branch} is checked out elsewhere)",
                    worktree.path.display()
                ),
                None => format!(
                    "Created worktree for {branch} at {}",
                    worktree.path.display()
                ),
            };
            print(to_value(&worktree)?, human);
        }
        Command::Worktree(WorktreeCommand::List) => {
            let worktrees = workspace.worktrees()?;
            let lines: Vec<String> = worktrees.iter().map(describe_worktree).collect();
            print(to_value(&worktrees)?, lines.join("\n"));
        }
        Command::Worktree(WorktreeCommand::Sync) => {
            let synced = workspace.sync_followers()?;
            let human = if synced.is_empty() {
                "No followers.".to_owned()
            } else {
                synced
                    .iter()
                    .map(describe_sync)
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            print(to_value(&synced)?, human);
        }
        Command::Land { branch } => {
            let landed = workspace.land(branch.as_deref())?;
            let plural = if landed.commits == 1 { "" } else { "s" };
            let holder = landed
                .holder
                .as_ref()
                .map(|holder| format!("; moved {} along", holder.display()))
                .unwrap_or_default();
            print(
                to_value(&landed)?,
                format!(
                    "Landed {} commit{plural} on {}{holder}",
                    landed.commits, landed.branch
                ),
            );
            note_follower_syncs(&workspace)?;
        }
        Command::Lost { limit } => {
            let lost = workspace.lost(*limit)?;
            let human = if lost.is_empty() {
                "Nothing lost.".to_owned()
            } else {
                let mut lines: Vec<String> = lost
                    .iter()
                    .map(|commit| {
                        let how = commit.how.split(':').next().unwrap_or_default();
                        let seen_on = commit.seen_on.trim_start_matches("refs/heads/");
                        format!(
                            "{} {} ({seen_on}, {}: {how})",
                            short(&commit.commit),
                            commit.summary,
                            ago(commit.seen_at)
                        )
                    })
                    .collect();
                lines.push("Restore one with: git branch <name> <commit>".to_owned());
                lines.join("\n")
            };
            print(to_value(&lost)?, human);
        }
        Command::Import(ImportSource::Graphite) => {
            let imported = workspace.import_graphite()?;
            let mut lines: Vec<String> = imported
                .trunks
                .iter()
                .map(|trunk| format!("Added trunk {trunk}"))
                .collect();
            lines.extend(
                imported
                    .parents
                    .iter()
                    .map(|parent| format!("Recorded {} on {}", parent.branch, parent.parent)),
            );
            lines.extend(
                imported
                    .skipped
                    .iter()
                    .map(|skipped| format!("Skipped {}: {}", skipped.branch, skipped.reason)),
            );
            if lines.is_empty() {
                lines.push("Nothing to import.".to_owned());
            }
            print(to_value(&imported)?, lines.join("\n"));
        }
        Command::Pr { line, draft } => {
            let branch = branch_or_current(&workspace, line.branch.as_deref())?;
            let proposed =
                workspace.propose(&branch, line.scope(), line.remote.as_deref(), *draft)?;
            let mut lines = vec![describe_push(&proposed.pushed)];
            lines.extend(proposed.pull_requests.iter().map(describe_proposal));
            print(to_value(&proposed)?, lines.join("\n"));
            if proposed
                .pushed
                .branches
                .iter()
                .any(|branch| branch.outcome == PushOutcome::Rejected)
            {
                return Err("some branches were rejected; fetch, restack, and try again".into());
            }
        }
        Command::Guard(GuardCommand::Install) => {
            let installed = workspace.install_guard(&std::env::current_exe()?)?;
            let chained = if installed.chained {
                " (your existing pre-push hook runs after it)"
            } else {
                ""
            };
            let protected = workspace.protected_branches()?.join(", ");
            print(
                to_value(&installed)?,
                format!(
                    "Installed the push guard at {}{chained}\nProtected: {protected}",
                    installed.hook.display()
                ),
            );
        }
        Command::Guard(GuardCommand::Uninstall) => {
            let removed = workspace.uninstall_guard()?;
            let human = if removed {
                "Removed the push guard"
            } else {
                "The push guard isn't installed"
            };
            print(serde_json::json!({ "removed": removed }), human.to_owned());
        }
        Command::Guard(GuardCommand::Status) => {
            let installed = workspace.guard_installed()?;
            let protected = workspace.protected_branches()?;
            let human = format!(
                "The push guard is {}installed\nProtected: {}",
                if installed { "" } else { "not " },
                if protected.is_empty() {
                    "nothing".to_owned()
                } else {
                    protected.join(", ")
                }
            );
            print(
                serde_json::json!({ "installed": installed, "protected": protected }),
                human,
            );
        }
        Command::Guard(GuardCommand::CheckPush { remote, .. }) => {
            let mut input = String::new();
            std::io::Read::read_to_string(&mut std::io::stdin(), &mut input)?;
            let violations = workspace.check_push(&input)?;
            if !violations.is_empty() && !confirm_violations(remote, &violations)? {
                return Err(
                    "push refused by the stack guard; a human can confirm it at a terminal".into(),
                );
            }
        }
        Command::Delta { a, b, commits } => {
            let b = branch_or_current(&workspace, b.as_deref())?;
            let delta = workspace.delta(a, &b)?;
            if cli.json {
                print(to_value(&delta)?, String::new());
                return Ok(());
            }
            let git = |args: &[&str]| -> Result<()> {
                let status = std::process::Command::new("git")
                    .arg("-C")
                    .arg(&directory)
                    .args(args)
                    .status()?;
                if status.success() {
                    Ok(())
                } else {
                    Err(format!("git {} failed", args[0]).into())
                }
            };
            let range = |range: &CommitRange| format!("{}..{}", range.base, range.tip);
            if *commits {
                git(&["range-diff", &range(&delta.a), &range(&delta.b)])?;
            } else if let Some(tree) = &delta.b_on_a_base {
                git(&["diff", &delta.a.tip, tree])?;
            } else {
                let paths = delta.conflicts.join(", ");
                return Err(format!(
                    "{b}'s changes conflict with {a}'s base in {paths}; try --commits"
                )
                .into());
            }
        }
        Command::Split { branch, points } => {
            let created = workspace.split(branch, points)?;
            print(
                to_value(&created)?,
                format!("Split {branch} into {}", created.join(", ")),
            );
        }
        Command::Undo { to } => {
            let undone = match to {
                Some(id) => workspace.undo_to(*id)?,
                None => vec![workspace.undo()?],
            };
            note_follower_syncs(&workspace)?;
            let lines: Vec<String> = undone
                .iter()
                .map(|operation| format!("Undid #{}: {}", operation.id, operation.description))
                .collect();
            let value = if to.is_some() {
                to_value(&undone)?
            } else {
                to_value(&undone[0])?
            };
            print(value, lines.join("\n"));
        }
        Command::Redo => {
            let redone = workspace.redo()?;
            note_follower_syncs(&workspace)?;
            print(
                to_value(&redone)?,
                format!("Redid #{}: {}", redone.id, redone.description),
            );
        }
        Command::Oplog { limit } => {
            let operations = workspace.oplog(*limit)?;
            let human = if operations.is_empty() {
                "No operations yet.".to_owned()
            } else {
                operations
                    .iter()
                    .map(describe_operation)
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            print(to_value(&operations)?, human);
        }
    }
    Ok(())
}

/// Opens the workspace, reporting any interrupted operation it recovered on stderr.
fn discover(directory: &Path) -> Result<Workspace> {
    let workspace = Workspace::discover(directory)?;
    for recovered in workspace.recovered() {
        let outcome = match recovered.outcome {
            RecoveryOutcome::Completed => "it had finished, and is now marked done",
            RecoveryOutcome::RolledBack => "nothing had changed, and it is now marked failed",
            RecoveryOutcome::Inconsistent => {
                "refs are part-way between before and after; check `stack oplog`"
            }
        };
        eprintln!(
            "note: operation #{} ({}) was interrupted; {outcome}",
            recovered.operation, recovered.description
        );
    }
    Ok(workspace)
}

/// Reports a restack (or move) of `target`; on a conflict, starts git's rebase for the first one unless
/// `no_resolve`, or if that isn't possible, explains how to finish by hand.
fn finish_restack(
    workspace: &Workspace,
    target: &str,
    restacked: Restacked,
    no_resolve: bool,
    print: &dyn Fn(Value, String),
) -> Result<()> {
    let Some(conflict) = restacked.conflicts.first().cloned() else {
        print(to_value(&restacked)?, describe_restack(&restacked, true));
        return note_follower_syncs(workspace);
    };
    if no_resolve {
        print(to_value(&restacked)?, describe_restack(&restacked, true));
        return Err("restack stopped at a conflict".into());
    }
    match workspace.resolve_conflict(target, &conflict) {
        Ok(ResolveOutcome::Stopped { branch, paths }) => {
            let human = format!(
                "{}\n{}",
                describe_restack(&restacked, false),
                describe_stopped(&branch, &paths)
            );
            print(to_value(&restacked)?, human);
            Err("restack stopped at a conflict".into())
        }
        Ok(ResolveOutcome::Finished(next)) => {
            print(to_value(&restacked)?, describe_restack(&restacked, false));
            finish_restack(workspace, target, next, no_resolve, print)
        }
        Err(
            error @ (Error::DirtyWorktree { .. }
            | Error::CheckedOutElsewhere { .. }
            | Error::AlreadyResolving { .. }),
        ) => {
            let human = format!(
                "{}\nCouldn't start resolving it here: {error}",
                describe_restack(&restacked, true)
            );
            print(to_value(&restacked)?, human);
            Err("restack stopped at a conflict".into())
        }
        Err(error) => Err(error.into()),
    }
}

/// Steps from the current branch and checks out where it lands.
fn navigate(workspace: &Workspace, step: Step, print: &dyn Fn(Value, String)) -> Result<()> {
    let current = branch_or_current(workspace, None)?;
    let target = workspace.step(&current, step)?;
    if target == current {
        print(
            serde_json::json!({ "branch": target }),
            format!("Already on {target}"),
        );
    } else {
        workspace.switch(&target)?;
        print(
            serde_json::json!({ "branch": target }),
            format!("Switched to {target}"),
        );
    }
    Ok(())
}

/// Syncs followers after a command moved branches, noting on stderr any that moved or were left behind.
fn note_follower_syncs(workspace: &Workspace) -> Result<()> {
    for sync in workspace.sync_followers()? {
        if !matches!(sync.outcome, SyncOutcome::UpToDate) {
            eprintln!("note: {}", describe_sync(&sync));
        }
    }
    Ok(())
}

/// Asks a human at the terminal to confirm each violation by typing the branch it concerns. Refuses without a
/// terminal, or if `STACK_GUARD_PROMPT=never`.
fn confirm_violations(remote: &str, violations: &[GuardViolation]) -> Result<bool> {
    for violation in violations {
        let what = match violation {
            GuardViolation::Protected {
                branch,
                deleting: true,
            } => format!("delete protected {remote}/{branch}"),
            GuardViolation::Protected {
                branch,
                deleting: false,
            } => format!("push to protected {remote}/{branch}"),
            GuardViolation::NameMismatch {
                local,
                remote: target,
            } => format!("push {local} to {remote}/{target}"),
        };
        eprintln!("stack guard: this would {what}");
    }
    if std::env::var("STACK_GUARD_PROMPT").is_ok_and(|prompt| prompt == "never") {
        return Ok(false);
    }
    let Ok(mut tty) = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
    else {
        return Ok(false);
    };
    let mut reader = std::io::BufReader::new(tty.try_clone()?);
    for violation in violations {
        let branch = match violation {
            GuardViolation::Protected { branch, .. } => branch,
            GuardViolation::NameMismatch { remote, .. } => remote,
        };
        write!(tty, "Type {branch} to allow it: ")?;
        tty.flush()?;
        let mut answer = String::new();
        reader.read_line(&mut answer)?;
        if answer.trim() != branch {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Sets up `stack`, creating a repository first if there isn't one and the user agrees.
fn init(directory: &Path, trunks: &[String], yes: bool) -> Result<(Value, String)> {
    let (workspace, created) = match discover(directory) {
        Err(error) if matches!(error.downcast_ref(), Some(Error::NotARepository { .. })) => {
            if !(yes || confirm_create(directory)?) {
                return Err(error);
            }
            Workspace::create_repository(directory)?;
            (discover(directory)?, true)
        }
        other => (other?, false),
    };
    let trunks: Vec<&str> = trunks.iter().map(String::as_str).collect();
    let marked = workspace.init(&trunks)?;
    let mut lines = Vec::new();
    if created {
        lines.push(format!(
            "Created git repository in {}",
            std::path::absolute(directory)?.display()
        ));
    }
    lines.extend(marked.iter().map(describe_marked));
    if marked.is_empty() {
        lines.push("No trunk found; add one with `stack trunk add <branch>`".to_owned());
    }
    let value = serde_json::json!({ "created_repository": created, "trunks": marked });
    Ok((value, lines.join("\n")))
}

/// Asks on the terminal whether to create a repository at `directory`.
///
/// # Errors
/// Without a terminal, since there is no one to ask.
fn confirm_create(directory: &Path) -> Result<bool> {
    if !std::io::stdin().is_terminal() {
        return Err("not a git repository; run `stack init --yes` to create one".into());
    }
    let shown = std::path::absolute(directory)?;
    eprint!(
        "No git repository at {}. Create one? [y/N] ",
        shown.display()
    );
    std::io::stderr().flush()?;
    let mut answer = String::new();
    std::io::stdin().lock().read_line(&mut answer)?;
    Ok(matches!(answer.trim(), "y" | "Y" | "yes" | "Yes"))
}

fn branch_or_current(workspace: &Workspace, branch: Option<&str>) -> Result<String> {
    match (branch, workspace.status()?.head) {
        (Some(branch), _) => Ok(branch.to_owned()),
        (None, Head::Branch { name, .. }) => Ok(name),
        (None, Head::Detached { .. }) => Err("HEAD is detached; name a branch".into()),
    }
}
