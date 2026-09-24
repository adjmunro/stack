//! `stack`: stacked branches on top of git.

use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use serde_json::{Value, to_value};
use stack_core::{
    Error, Head, Marked, Node, Operation, OperationState, Outcome, RecoveryOutcome, Restacked,
    Role, Source, Tree, Workspace,
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
    Tree,
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
    },
    /// Move a branch onto a different parent, taking the branches on it along.
    Move {
        /// Branch to move [default: current branch].
        branch: Option<String>,
        /// Its new parent.
        #[arg(long)]
        onto: String,
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
            print(to_value(&status)?, describe(&status.head));
        }
        Command::Tree => {
            let tree = workspace.tree()?;
            let unborn = matches!(workspace.status()?.head, Head::Branch { commit: None, .. });
            let human = if unborn && tree.trunks.is_empty() && tree.unattached.is_empty() {
                "No commits yet.".to_owned()
            } else {
                render(&tree)
            };
            print(to_value(&tree)?, human);
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
        Command::Restack { branch } => {
            let branch = branch_or_current(&workspace, branch.as_deref())?;
            let restacked = workspace.restack(&branch)?;
            print(to_value(&restacked)?, describe_restack(&restacked));
            if let Some(conflict) = &restacked.conflict {
                return Err(format!("restack stopped at a conflict in {}", conflict.branch).into());
            }
        }
        Command::Move { branch, onto } => {
            let branch = branch_or_current(&workspace, branch.as_deref())?;
            let restacked = workspace.move_branch(&branch, onto)?;
            print(to_value(&restacked)?, describe_restack(&restacked));
            if let Some(conflict) = &restacked.conflict {
                return Err(format!("move stopped at a conflict in {}", conflict.branch).into());
            }
        }
        Command::Undo => {
            let undone = workspace.undo()?;
            print(
                to_value(&undone)?,
                format!("Undid #{}: {}", undone.id, undone.description),
            );
        }
        Command::Redo => {
            let redone = workspace.redo()?;
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

fn describe_restack(restacked: &Restacked) -> String {
    let mut lines: Vec<String> = restacked
        .moved
        .iter()
        .map(|moved| {
            let plural = if moved.replayed == 1 { "" } else { "s" };
            let dropped = match moved.dropped {
                0 => String::new(),
                count => format!("; {count} already in {}", moved.onto),
            };
            format!(
                "Restacked {} onto {} ({} commit{plural}{dropped})",
                moved.name, moved.onto, moved.replayed
            )
        })
        .collect();
    match &restacked.conflict {
        Some(conflict) => {
            let paths = conflict.paths.join(", ");
            lines.push(format!(
                "Conflict: {} \"{}\" ({}) conflicts with {} in {paths}",
                conflict.branch,
                conflict.summary,
                short(&conflict.commit),
                conflict.onto
            ));
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
        None if restacked.moved.is_empty() => lines.push("Everything is up to date.".to_owned()),
        None => {}
    }
    lines.join("\n")
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
fn render(tree: &Tree) -> String {
    let mut lines = Vec::new();
    if tree.trunks.is_empty() {
        lines.push("No trunks. Add one with `stack trunk add <branch>`.".to_owned());
    }
    for trunk in &tree.trunks {
        render_node(trunk, "", "", &mut lines);
    }
    if !tree.trunks.is_empty() && !tree.unattached.is_empty() {
        lines.push(String::new());
        lines.push("Unattached:".to_owned());
        for node in &tree.unattached {
            render_node(node, "", "", &mut lines);
        }
    }
    lines.join("\n")
}

fn render_node(node: &Node, lead: &str, indent: &str, lines: &mut Vec<String>) {
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
            &format!("{indent}{lead}"),
            &format!("{indent}{next}"),
            lines,
        );
    }
}

fn short(commit: &str) -> &str {
    &commit[..7.min(commit.len())]
}
