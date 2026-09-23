//! `stack`: stacked branches on top of git.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use serde_json::{Value, to_value};
use stack_core::{Head, Node, Outcome, Tree, Workspace};

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
    /// Show the current branch.
    Status,
    /// Show trunks and the branches stacked on them.
    Tree,
    /// Manage trunks: the long-lived branches stacks are based on.
    #[command(subcommand)]
    Trunk(TrunkCommand),
    /// Record a branch's parent. Re-run to change it.
    Track {
        /// Branch to track [default: current branch].
        branch: Option<String>,
        /// Trunk or tracked branch to stack it on.
        #[arg(short, long)]
        parent: String,
    },
    /// Forget a branch's parent. The branch itself is untouched.
    Untrack {
        /// Branch to untrack [default: current branch].
        branch: Option<String>,
    },
}

#[derive(Subcommand)]
enum TrunkCommand {
    /// Make an existing branch a trunk.
    Add { name: String },
    /// Stop treating a branch as a trunk. The branch itself is untouched.
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
    let workspace = Workspace::discover(directory)?;
    let print = |value: Value, human: String| {
        if cli.json {
            println!(
                "{}",
                serde_json::to_string_pretty(&value).expect("JSON values serialise")
            );
        } else {
            print!("{human}");
        }
    };
    match &cli.command {
        Command::Status => {
            let status = workspace.status()?;
            print(to_value(&status)?, format!("{}\n", describe(&status.head)));
        }
        Command::Tree => {
            let tree = workspace.tree()?;
            print(to_value(&tree)?, render(&tree));
        }
        Command::Trunk(TrunkCommand::Add { name }) => {
            let outcome = workspace.add_trunk(name)?;
            print(
                to_value(outcome)?,
                confirm(
                    outcome,
                    format!("Added trunk {name}"),
                    format!("{name} is already a trunk"),
                ),
            );
        }
        Command::Trunk(TrunkCommand::Remove { name }) => {
            let outcome = workspace.remove_trunk(name)?;
            print(
                to_value(outcome)?,
                confirm(outcome, format!("Removed trunk {name}"), String::new()),
            );
        }
        Command::Track { branch, parent } => {
            let branch = branch_or_current(&workspace, branch.as_deref())?;
            let outcome = workspace.track(&branch, parent)?;
            let unchanged = format!("{branch} is already tracked on {parent}");
            print(
                to_value(outcome)?,
                confirm(outcome, format!("Tracked {branch} on {parent}"), unchanged),
            );
        }
        Command::Untrack { branch } => {
            let branch = branch_or_current(&workspace, branch.as_deref())?;
            let outcome = workspace.untrack(&branch)?;
            print(
                to_value(outcome)?,
                confirm(outcome, format!("Untracked {branch}"), String::new()),
            );
        }
    }
    Ok(())
}

fn branch_or_current(workspace: &Workspace, branch: Option<&str>) -> Result<String> {
    match (branch, workspace.status()?.head) {
        (Some(branch), _) => Ok(branch.to_owned()),
        (None, Head::Branch { name, .. }) => Ok(name),
        (None, Head::Detached { .. }) => Err("HEAD is detached; name a branch".into()),
    }
}

fn confirm(outcome: Outcome, changed: String, unchanged: String) -> String {
    match outcome {
        Outcome::Changed => format!("{changed}\n"),
        Outcome::Unchanged => format!("{unchanged}\n"),
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
    let mut out = String::new();
    if tree.trunks.is_empty() && tree.orphans.is_empty() {
        out.push_str("No trunks. Add one with `stack trunk add <branch>`.\n");
    }
    for trunk in &tree.trunks {
        render_node(trunk, "", "", &mut out);
    }
    if !tree.orphans.is_empty() {
        out.push_str("\nParent missing:\n");
        for orphan in &tree.orphans {
            render_node(orphan, "", "", &mut out);
        }
    }
    out
}

fn render_node(node: &Node, lead: &str, indent: &str, out: &mut String) {
    let marker = if node.current { "* " } else { "" };
    let commit = node
        .commit
        .as_deref()
        .map_or_else(|| "(deleted)".to_owned(), |commit| short(commit).to_owned());
    out.push_str(&format!("{lead}{marker}{} {commit}\n", node.name));
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
            out,
        );
    }
}

fn short(commit: &str) -> &str {
    &commit[..7.min(commit.len())]
}
