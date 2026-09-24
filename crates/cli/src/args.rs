//! Command-line arguments.

use super::*;

#[derive(Parser)]
#[command(name = "stack", version, about = "Stacked branches on top of git")]
pub(crate) struct Cli {
    /// Run as if started in <PATH>.
    #[arg(short = 'C', global = true, value_name = "PATH")]
    pub(crate) directory: Option<PathBuf>,

    /// Print machine-readable JSON.
    #[arg(long, global = true)]
    pub(crate) json: bool,

    #[command(subcommand)]
    pub(crate) command: Command,
}

#[derive(Subcommand)]
pub(crate) enum Command {
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
        /// Branches to move (each onto the new parent, so they become siblings) [default: current branch].
        branches: Vec<String>,
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
    /// Split a branch into a stack: new branches at some of its own commits, stacked in order beneath it.
    Split {
        branch: String,
        /// Where to cut, as COMMIT=NAME (e.g. `HEAD~2=feat/part-1`); repeatable.
        #[arg(required = true, value_parser = parse_point)]
        points: Vec<(String, String)>,
    },
    /// Line branches up into one stack, in the order given: each moves onto the one before it.
    Chain {
        #[arg(required = true, num_args = 2..)]
        branches: Vec<String>,
        /// On a conflict, print how to finish by hand instead of starting git's rebase for you.
        #[arg(long)]
        no_resolve: bool,
    },
    /// Check a branch's commit subjects against the rules (stack.lint.pattern, stack.lint.maxLength).
    Lint {
        /// Branch to check [default: current branch].
        branch: Option<String>,
    },
    /// Revert the latest stack command.
    Undo {
        /// Revert every command from this op log id on instead (see `stack oplog`).
        #[arg(long, value_name = "ID")]
        to: Option<i64>,
    },
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
pub(crate) enum KindArg {
    Reviewed,
    Tested,
    Flagged,
}

/// Which branches around one branch to cover, and where to push them.
#[derive(clap::Args)]
pub(crate) struct LineArgs {
    /// Branch whose line to cover [default: current branch].
    pub(crate) branch: Option<String>,
    /// Only the branch and its parents.
    #[arg(long, conflicts_with = "leafward")]
    pub(crate) rootward: bool,
    /// Only the branch and the branches on it.
    #[arg(long)]
    pub(crate) leafward: bool,
    /// Carry on past trunks stacked above, to the leaves.
    #[arg(short, long)]
    pub(crate) all: bool,
    /// Remote to push to [default: the branch's upstream remote, else origin, else the only remote].
    #[arg(long)]
    pub(crate) remote: Option<String>,
}

impl LineArgs {
    pub(crate) fn scope(&self) -> Scope {
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
pub(crate) enum GuardCommand {
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
pub(crate) enum ImportSource {
    /// Graphite's trunk and recorded parents. Graphite's own data is left as it is.
    Graphite,
}

#[derive(Subcommand)]
pub(crate) enum WorktreeCommand {
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
pub(crate) enum TrunkCommand {
    /// Mark an existing branch as a trunk.
    Add { name: String },
    /// Unmark a trunk. The branch itself is untouched.
    Remove { name: String },
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

/// Parses a split point, `COMMIT=NAME`.
fn parse_point(point: &str) -> std::result::Result<(String, String), String> {
    match point.split_once('=') {
        Some((commit, name)) if !commit.is_empty() && !name.is_empty() => {
            Ok((commit.to_owned(), name.to_owned()))
        }
        _ => Err(format!("expected COMMIT=NAME, got {point:?}")),
    }
}
