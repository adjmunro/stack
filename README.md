# stack

Stacked branches on top of git. The repo stays a normal git repo: `stack` works out how your branches stack from the commit graph, keeps its own state in refs and a local database that other people never see, and can undo everything it does.

- **No bookkeeping.** Parents are derived from the graph, including after you amend, rebase, or move branches with plain git, your IDE, or a GUI. Pin a parent only when the graph can't tell.
- **Safe by construction.** Every change is journalled (crash-safe) and undoable. Pushes use leases. An optional push guard stops anyone, including agents, from pushing to protected branches or under another name without a human confirming.
- **A library first.** `stack-core` is the engine; the CLI is a thin layer on it, and a GUI can be another.

## Install

Needs Rust and git 2.45 or newer.

```bash
cargo install --path crates/cli
```

## Quick start

```bash
stack init                        # marks origin's default branch (or main/master/develop) as a trunk
stack create feat/a -am "feat: a" # branch + commit
stack create feat/b -am "feat: b"
stack tree                        # develop ← feat/a ← feat/b
stack restack develop             # after develop moves on: rebase every stack on it
stack pr                          # push feat/a and feat/b; open PRs against their parents
stack sync                        # after they merge: fast-forward develop, archive merged branches, restack
```

## Commands

| Area | Commands |
|---|---|
| Set up | `init`, `trunk add/remove`, `import graphite` |
| See | `tree [--check] [-- <paths>]`, `status`, `check`, `delta <a> [b]`, `review`, `lost` |
| Rewrite | `restack`, `move --onto`, `continue`, `abort`, `sync` |
| Share | `push`, `pr` (both take `--rootward`, `--leafward`, `--all`) |
| Move around | `up`, `down`, `top`, `bottom`, `create` |
| Tidy | `archive`, `unarchive`, `archived`, `pin`, `unpin` |
| Review | `mark [--tested \| --flagged --note …]`, `unmark` |
| Worktrees | `worktree add/list/sync`, `land` |
| Safety | `undo`, `redo`, `oplog`, `guard install/uninstall/status` |

Every command takes `-C <path>` and `--json`. See `stack <command> --help`.

## Concepts

- **Trunk**: a long-lived branch (`develop`, `release/1.x`) that `stack` never rebases or pushes. A trunk marked on top of a stack is a **limb**: a base for the branches above it that is restacked with its parent.
- **Line**: what `push` and `pr` cover: the branch, its parents down to the nearest trunk or limb, and the branches above it up to the next limbs.
- **Follower**: a worktree detached at a branch checked out elsewhere, kept at its tip; `land` moves its commits onto the branch without the main workspace giving up its checkout.

Full vocabulary: [docs/GLOSSARY.md](docs/GLOSSARY.md). Design decisions: [docs/ADR.md](docs/ADR.md).

## Development

```bash
cargo test --workspace
```

Tests run against throwaway repos with a sealed git environment, and check a full before-and-after snapshot of each repo, so an operation that changes anything unexpected fails.
