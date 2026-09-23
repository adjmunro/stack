//! `stack`: stacked branches on top of git.

use std::error::Error as _;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use stack_core::{Head, Workspace};

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

fn run(cli: &Cli) -> Result<(), stack_core::Error> {
    let directory = cli.directory.clone().unwrap_or_else(|| PathBuf::from("."));
    let workspace = Workspace::discover(directory)?;
    match cli.command {
        Command::Status => {
            let status = workspace.status()?;
            if cli.json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&status).expect("view-models serialise")
                );
            } else {
                println!("{}", describe(&status.head));
            }
        }
    }
    Ok(())
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

fn short(commit: &str) -> &str {
    &commit[..7.min(commit.len())]
}
