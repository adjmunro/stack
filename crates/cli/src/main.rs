//! `stack`: stacked branches on top of git.

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

#[derive(Parser)]
#[command(name = "stack", version, about = "Stacked branches on top of git")]
struct Cli {
    /// Run as if started in <PATH>.
    #[arg(short = 'C', global = true, value_name = "PATH")]
    directory: Option<PathBuf>,

    /// Print machine-readable JSON.
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Set up stack in this repository, creating the repository if needed.
    Init {
        /// Branch to mark as a trunk; repeatable [default: the remote's default branch, or a conventional name].
        #[arg(short, long = "trunk", value_name = "BRANCH")]
        trunks: Vec<String>,
        /// Create a git repository without asking if there isn't one.
        #[arg(short, long)]
        yes: bool,
    },
    /// Show the current branch.
    Status,
    /// Show trunks and the branches stacked on them.
    Tree {
        /// Also preview a restack: mark branches that would conflict, or are blocked behind a conflict.
        #[arg(long)]
        check: bool,
        /// Only branches whose own commits change these paths (git pathspecs), and the branches beneath them.
        #[arg(last = true)]
        paths: Vec<String>,
    },
    /// Manage trunks: the long-lived branches stacks are based on.
    #[command(subcommand)]
    Trunk(TrunkCommand),
    /// Fix a branch's parent, overriding what the commit graph implies.
    Pin {
        /// Branch to pin [default: current branch].
        branch: Option<String>,
        /// Parent to pin it to [default: its current parent].
        #[arg(short, long)]
        parent: Option<String>,
    },
    /// Remove a pin, so the parent is worked out automatically again.
    Unpin {
        /// Branch to unpin [default: current branch].
        branch: Option<String>,
    },
    /// Rebase a branch and everything stacked on it onto their parents' current tips.
    Restack {
        /// Branch to restack; a trunk restacks every stack on it [default: current branch].
        branch: Option<String>,
        /// On a conflict, print how to finish by hand instead of starting git's rebase for you.
        #[arg(long)]
        no_resolve: bool,
    },
    /// Catch up after work lands: fetch, fast-forward trunks, archive merged branches (rehoming what's on them), and
    /// restack.
    Sync {
        /// Remote to sync with [default: origin, else the only remote].
        #[arg(long)]
        remote: Option<String>,
        /// Use what was last fetched instead of fetching.
        #[arg(long)]
        no_fetch: bool,
    },
    /// Carry on after resolving a restack conflict (and `git add`ing the files).
    Continue,
    /// Give up on a restack that's waiting at a conflict.
    Abort,
    /// Preview a restack: which branches would conflict. Changes nothing; fails if any would.
    Check {
        /// Branch to check, with everything on it [default: every trunk].
        branch: Option<String>,
    },
    /// Move a branch onto a different parent, taking the branches on it along.
    Move {
        /// Branch to move [default: current branch].
        branch: Option<String>,
        /// Its new parent.
        #[arg(long)]
        onto: String,
        /// On a conflict, print how to finish by hand instead of starting git's rebase for you.
        #[arg(long)]
        no_resolve: bool,
    },
    /// Push a branch's line: its parents down to the nearest trunk, and the branches on it up to the next trunks.
    Push(LineArgs),
    /// Push a branch's line and open or retarget a pull request for each branch, against its parent (via gh).
    Pr {
        #[command(flatten)]
        line: LineArgs,
        /// Open new pull requests as drafts.
        #[arg(long)]
        draft: bool,
    },
    /// Check out the branch stacked on this one.
    Up {
        /// How many branches to go up.
        #[arg(default_value_t = 1)]
        steps: usize,
    },
    /// Check out this branch's parent.
    Down {
        /// How many branches to go down.
        #[arg(default_value_t = 1)]
        steps: usize,
    },
    /// Check out the top of this stack.
    Top,
    /// Check out the first branch of this stack.
    Bottom,
    /// Create a branch on the current one and check it out; commit too with --message.
    Create {
        name: String,
        /// Commit the staged changes with this message.
        #[arg(short, long)]
        message: Option<String>,
        /// Stage changes to tracked files before committing.
        #[arg(short, long, requires = "message")]
        all: bool,
    },
    /// Put a branch away: hidden from git branch, the tree, restack, and push, but kept safe.
    Archive { branch: String },
    /// Bring an archived branch back.
    Unarchive { name: String },
    /// List archived branches.
    Archived,
    /// List a branch's own commits with their review marks.
    Review {
        /// Branch to review [default: current branch].
        branch: Option<String>,
    },
    /// Mark commits as reviewed (or tested, or flagged). Marks survive rebases until the change itself changes.
    Mark {
        /// Commits to mark [default: HEAD].
        revisions: Vec<String>,
        /// Mark every commit of this branch instead.
        #[arg(long, conflicts_with = "revisions")]
        branch: Option<String>,
        /// Mark as built and tested instead of reviewed.
        #[arg(long, conflicts_with = "flagged")]
        tested: bool,
        /// Flag for another look instead of marking reviewed.
        #[arg(long)]
        flagged: bool,
        /// A note to keep with the mark.
        #[arg(long)]
        note: Option<String>,
    },
    /// Remove review marks.
    Unmark {
        /// Commits to unmark [default: HEAD].
        revisions: Vec<String>,
        /// Remove only this kind of mark.
        #[arg(long, value_enum)]
        kind: Option<KindArg>,
    },
    /// Manage worktrees: hidden sibling directories, and followers of branches checked out elsewhere.
    #[command(subcommand)]
    Worktree(WorktreeCommand),
    /// Land this follower's commits on the branch it follows, moving the worktree that has it checked out along.
    Land {
        /// Branch to land on [default: the branch this worktree follows].
        branch: Option<String>,
    },
    /// List commits you've been on that no branch, tag, or archive reaches any more.
    Lost {
        /// How many to show.
        #[arg(short = 'n', long, default_value_t = 20)]
        limit: usize,
    },
    /// Import stacks from another tool.
    #[command(subcommand)]
    Import(ImportSource),
    /// Guard every push (including plain git push): no pushing to protected branches or under another name without
    /// a human confirming at a terminal.
    #[command(subcommand)]
    Guard(GuardCommand),
    /// Show how two branches' own changes differ, ignoring the difference in their bases.
    Delta {
        a: String,
        /// Branch to compare with [default: current branch].
        b: Option<String>,
        /// Compare commit by commit (git range-diff) instead of the net change.
        #[arg(long)]
        commits: bool,
    },
    /// Revert the latest stack command.
    Undo,
    /// Re-apply the most recently undone command.
    Redo,
    /// List recent stack operations, newest first.
    Oplog {
        /// How many to show.
        #[arg(short = 'n', long, default_value_t = 20)]
        limit: usize,
    },
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum KindArg {
    Reviewed,
    Tested,
    Flagged,
}

impl From<KindArg> for MarkKind {
    fn from(kind: KindArg) -> Self {
        match kind {
            KindArg::Reviewed => MarkKind::Reviewed,
            KindArg::Tested => MarkKind::Tested,
            KindArg::Flagged => MarkKind::Flagged,
        }
    }
}

/// Which branches around one branch to cover, and where to push them.
#[derive(clap::Args)]
struct LineArgs {
    /// Branch whose line to cover [default: current branch].
    branch: Option<String>,
    /// Only the branch and its parents.
    #[arg(long, conflicts_with = "leafward")]
    rootward: bool,
    /// Only the branch and the branches on it.
    #[arg(long)]
    leafward: bool,
    /// Carry on past trunks stacked above, to the leaves.
    #[arg(short, long)]
    all: bool,
    /// Remote to push to [default: the branch's upstream remote, else origin, else the only remote].
    #[arg(long)]
    remote: Option<String>,
}

impl LineArgs {
    fn scope(&self) -> Scope {
        let direction = match (self.rootward, self.leafward) {
            (true, _) => Direction::Rootward,
            (_, true) => Direction::Leafward,
            _ => Direction::Both,
        };
        Scope {
            direction,
            through_limbs: self.all,
        }
    }
}

#[derive(Subcommand)]
enum GuardCommand {
    /// Install the guard as this repository's pre-push hook (an existing hook still runs, after it).
    Install,
    /// Remove the guard, restoring any hook it ran in front of.
    Uninstall,
    /// Show whether the guard is installed and which branches it protects.
    Status,
    /// Check a push's ref updates from stdin, as a pre-push hook; the hook runs this.
    #[command(hide = true)]
    CheckPush { remote: String, url: Option<String> },
}

#[derive(Subcommand)]
enum ImportSource {
    /// Graphite's trunk and recorded parents. Graphite's own data is left as it is.
    Graphite,
}

#[derive(Subcommand)]
enum WorktreeCommand {
    /// Add a worktree for a branch; a follower if the branch is checked out elsewhere.
    Add {
        branch: String,
        /// Where to put it [default: a hidden sibling of the main worktree, .<repo>-<branch>].
        #[arg(long)]
        path: Option<PathBuf>,
    },
    /// List worktrees and what followers follow.
    List,
    /// Bring followers up to date with their branches.
    Sync,
}

#[derive(Subcommand)]
enum TrunkCommand {
    /// Mark an existing branch as a trunk.
    Add { name: String },
    /// Unmark a trunk. The branch itself is untouched.
    Remove { name: String },
}

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
        Command::Undo => {
            let undone = workspace.undo()?;
            note_follower_syncs(&workspace)?;
            print(
                to_value(&undone)?,
                format!("Undid #{}: {}", undone.id, undone.description),
            );
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

fn describe_sync_all(synced: &Synced, fetched: bool) -> String {
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

fn describe_stopped(branch: &str, paths: &[String]) -> String {
    format!(
        "Resolving {branch}: fix the conflicts in {}, `git add` them, then run `stack continue` (or `stack abort`)",
        paths.join(", ")
    )
}

/// Describes a restack; with `manual`, each conflict comes with the commands to finish it by hand.
fn describe_restack(restacked: &Restacked, manual: bool) -> String {
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

fn describe_preview(preview: &RestackPreview) -> String {
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

fn mark_name(kind: MarkKind) -> &'static str {
    match kind {
        MarkKind::Reviewed => "reviewed",
        MarkKind::Tested => "tested",
        MarkKind::Flagged => "flagged",
    }
}

fn describe_review(review: &[CommitReview]) -> String {
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

fn describe_worktree(worktree: &Worktree) -> String {
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

fn describe_sync(sync: &FollowerSync) -> String {
    let path = sync.path.display();
    match &sync.outcome {
        SyncOutcome::Moved { to, .. } => {
            format!("Moved follower {path} to {} ({})", sync.branch, short(to))
        }
        SyncOutcome::UpToDate => format!("{path} is up to date with {}", sync.branch),
        SyncOutcome::Skipped { reason } => format!("Left {path}: {reason}"),
    }
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

/// "5 minutes ago", "3 hours ago", "2 days ago".
fn ago(seconds_since_epoch: i64) -> String {
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

fn describe_proposal(proposal: &ProposedBranch) -> String {
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

fn describe_push(pushed: &Pushed) -> String {
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

fn describe_conflict(conflict: &Conflict) -> String {
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
fn count(replayed: usize, dropped: usize, onto: &str) -> String {
    let plural = if replayed == 1 { "" } else { "s" };
    match dropped {
        0 => format!("{replayed} commit{plural}"),
        dropped => format!("{replayed} commit{plural}; {dropped} already in {onto}"),
    }
}

fn describe_operation(operation: &Operation) -> String {
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

fn describe_marked(marked: &Marked) -> String {
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

fn branch_or_current(workspace: &Workspace, branch: Option<&str>) -> Result<String> {
    match (branch, workspace.status()?.head) {
        (Some(branch), _) => Ok(branch.to_owned()),
        (None, Head::Branch { name, .. }) => Ok(name),
        (None, Head::Detached { .. }) => Err("HEAD is detached; name a branch".into()),
    }
}

fn describe(head: &Head) -> String {
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
fn render(tree: &Tree, preview: Option<&RestackPreview>) -> String {
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

fn render_node(
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

fn short(commit: &str) -> &str {
    &commit[..7.min(commit.len())]
}
