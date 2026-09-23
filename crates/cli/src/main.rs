//! `stack`: stacked branches on top of git.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use serde_json::{Value, to_value};
use stack_core::{Head, Node, Outcome, Role, Source, Tree, Workspace};

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
    let workspace = Workspace::discover(directory)?;
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
    match &cli.command {
        Command::Status => {
            let status = workspace.status()?;
            print(to_value(&status)?, describe(&status.head));
        }
        Command::Tree => {
            let tree = workspace.tree()?;
            print(to_value(&tree)?, render(&tree));
        }
        Command::Trunk(TrunkCommand::Add { name }) => {
            let marked = workspace.add_trunk(name)?;
            let stacked = marked
                .parent
                .as_ref()
                .map(|parent| format!(" (stacked on {parent})"))
                .unwrap_or_default();
            let human = match marked.outcome {
                Outcome::Changed => format!("Added trunk {name}{stacked}"),
                Outcome::Unchanged => format!("{name} is already a trunk{stacked}"),
            };
            print(to_value(&marked)?, human);
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
