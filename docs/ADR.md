# ADR.md

## Instructions

- This files is a living document (managed by YOU) of important, high-level decisions (e.g. architecture, libraries chosen etc) that affect the project.
- Append to the tail of `## Decision Registry`.
- You can use `~` around the header to ~revoke~ decisions when they are redundant or no longer relevant.

## Format

- Record the datetime and current SHA at time of writing as context for posterity.
- Each element can have multiple bullet points for each category, decision detail, and reason.

```markdown
<created-at-datetime>@<commit-sha>
- Category: <scope>
- [Status: REVOKED: <why> | SUPERSEDED BY <[<datetime>@<commit-sha:0..8>]>]
- Detail: <what-was-decided>
- Reason: <why>
```

---

## Decision Registry

2026-09-23T10:37Z@7b510daf
- Category: architecture
- Detail: Cargo workspace. `stack-core` (library) exposes a `Workspace` facade with ports (`GitRepo`, metadata, cache) and adapters behind them. CLI, GUI, and MCP are thin surfaces over it.
- Detail: Queries return plain `serde` view-models, shared by CLI `--json`, MCP, and GUI.
- Detail: Core types are `Send`, hold no global state, and push change events rather than requiring polling.
- Reason: The same core must serve a CLI, a GUI embedded in ghostrealm, and agents.

2026-09-23T10:37Z@7b510daf
- Category: git backend
- Status: SUPERSEDED BY [2026-09-23T20:13Z@c8897497] for restack's tree merges and commit creation only
- Detail: `gix` for reads and object creation (including in-memory tree merges for restack). `git` CLI for working-tree/index changes, fetch/push, hooks, and signing.
- Detail: All git access goes through the `GitRepo` port so each operation can switch backend.
- Reason: `gix` is pure Rust, fast, and in-process. Object writes are content-addressed and safe from a library; index, working tree, credentials, and hooks need exact `git` parity.
- Reason: User signs commits (SSH). Neither library signs, so commits created in memory must be signed explicitly.

2026-09-23T10:37Z@7b510daf
- Category: state storage
- Detail: Authoritative metadata (stack parents, trunks, op-log snapshots) lives in git refs under `refs/stack/`.
- Detail: Local, rebuildable, or personal data (review marks, conflict-check cache, indexes, UI state, journal, detailed op history) lives in SQLite at `.git/stack/` (via `--git-common-dir`, so linked worktrees share it).
- Detail: Mutations follow a journal pattern: record intent in SQLite, apply one atomic git ref transaction, mark complete. On start-up, incomplete intents are resolved by comparing refs to their recorded old/new values.
- Reason: Refs keep objects safe from gc and update atomically with branches. SQLite is transactional internally but cannot share a transaction with git, so it records intent rather than truth.
- Reason: Neither `refs/stack/*` nor `.git/stack/` is pushed or fetched by default, so other users never see `stack` state unless they opt in.

2026-09-23T10:37Z@7b510daf
- Category: testing
- Detail: Tests run against real fixture repos in temp dirs, built with the real `git` CLI in an isolated environment (no global/system config, fixed identities and dates).
- Detail: Mutation tests diff a full repo snapshot before and after, asserting the change is exactly the expected one.
- Reason: Mishandling a real repo is catastrophic. Tests must prove an operation does no more and no less than intended.

2026-09-23T11:05Z@82e6359b
- Category: state storage
- Detail: One ref per item, pointing directly at a JSON blob: `refs/stack/trunks/<name>` → `{"version":1}`; `refs/stack/branches/<name>` → `{"version":1,"parent":"<branch>","base":"<commit>"}`.
- Detail: `base` is the merge base of the branch and its parent when tracked. Restack will use it to find the branch's own commits.
- Detail: Unknown `version`s are rejected as corrupt, never rewritten.
- Reason: Blob refs don't appear in `git log --all`, survive `gc`, and pass `fsck` (verified by tests). One ref per item keeps compare-and-swap updates independent and allows per-branch sync later.

2026-09-23T12:00Z@585d2fb8
- Category: parent resolution
- Detail: Auto-tracking replaces `track`/`untrack`. Recorded parents (refs) hold intent; the commit graph holds current state; the two are reconciled on every read.
- Detail: A recorded parent holds while the branch contains some version of it: the parent's current tip, a former tip (git reflog, or tips `stack` has observed, kept in SQLite), or commits with matching patch-ids. Otherwise it is re-derived from the graph and the change reported.
- Detail: Unrecorded parents are derived as the nearest branch whose tip is an ancestor, never walking past a trunk. Branches on the same commit: the older is the parent; a child that can't choose takes the oldest. Age: first reflog entry, then first observed by `stack`, then name.
- Detail: `stack pin` records a parent manually. Pins never change automatically; a contradicted pin is flagged.
- Detail: Reads never write refs. Derived parents are recorded by the next `stack` mutation.
- Detail: Rename metadata `base` to `offshoot` (format unreleased; version stays 1).
- Reason: Graphite-style explicit tracking drifts when other tools change the repo. Pure derivation loses intent (empty branches, content-changing amends, parallel branches with identical patch-ids).

~2026-09-23T12:00Z@585d2fb8~
- Category: branch roles and scopes
- Status: SUPERSEDED BY [2026-09-23T12:09Z@687b03d3]
- Detail: Trunks have no parent and are never rebased or pushed by `stack`. Limbs have a parent and act as bases for branches leafward of them.
- Detail: Restack runs leafward from a branch, through limbs.
- Detail: Push defaults to the current branch's line: parents rootward up to the nearest trunk or limb, plus descendants leafward up to the next limbs. Trunks, limbs, and siblings are excluded. `--rootward` and `--leafward` narrow it to one direction.
- Reason: A limb may belong to someone else, so ranges never push it. Restack must pass through limbs because their commits sit on the branch being moved.

~2026-09-23T12:09Z@687b03d3~
- Category: branch roles and scopes
- Status: SUPERSEDED BY [2026-09-23T12:17Z@34a9d190]
- Detail: `stack trunk add` marks both trunks and limbs; the role is inferred. A marked branch whose parent is a trunk (or that has none) is a trunk; one whose parent is a regular branch is a limb.
- Detail: Trunks are never rebased or pushed by `stack`. Limbs are restacked with their parent.
- Detail: Restack runs leafward from a branch, through limbs.
- Detail: Push defaults to the current branch's line: the branch, its parents rootward up to the nearest trunk or limb (excluded), and all descendants leafward (limbs included). Siblings of the branch and its parents are excluded. `--rootward` and `--leafward` narrow it to one direction.
- Reason: A rootward limb may be someone else's branch; a leafward one is almost certainly yours.
- Reason: A release branch cut from a trunk looks the same as a checkpoint on a trunk. Treating both as trunks means the failure mode is "not auto-restacked", never "release branch rebased onto develop".

2026-09-23T12:17Z@34a9d190
- Category: branch roles and scopes
- Detail: `stack trunk add` marks both trunks and limbs; the role is inferred. A marked branch whose parent is a trunk (or that has none) is a trunk; one whose parent is a regular branch is a limb.
- Detail: Trunks are never rebased or pushed by `stack`. Limbs are restacked with their parent.
- Detail: Restack runs leafward from a branch through every descendant, limbs included, except archived, backup, and ignored branches.
- Detail: Push and PR commands default to the current branch's line: the branch, its parents rootward up to the nearest trunk or limb (excluded), and its descendants leafward up to and including the next limbs. Siblings of the branch and its parents are excluded.
- Detail: On a limb, the line is the limb and its descendants up to the next limbs (its own stack); nothing rootward.
- Detail: `--all`/`-a` extends the leafward part past limbs to the leaves. `--rootward` and `--leafward` narrow the line to one direction.
- Detail: Pushing branches and opening PRs are separate commands, so a wide push (e.g. a backup) never opens a wave of PRs.
- Reason: A rootward limb may be someone else's; a leafward limb is yours but starts a separate batch of review work.
- Reason: A release branch cut from a trunk looks the same as a checkpoint on a trunk. Treating both as trunks means the failure mode is "not auto-restacked", never "release branch rebased onto develop".

2026-09-23T13:02Z@17e63722
- Category: op log
- Detail: One SQLite store (`<common dir>/stack/stack.db`, rusqlite with bundled SQLite) is both the op log and the crash journal. Tables: `operation` (kind command/undo/redo, state pending/done/failed, undone flag) and `ref_update` (old/new per ref).
- Detail: Created by the first mutation; reads never create or write it. Opening a workspace resolves pending operations: all refs new → done; all old → failed; otherwise left pending and reported.
- Detail: Undo/redo follow editor semantics: undo reverts the latest command not undone; redo re-applies the most recently undone command until a new command runs. Both refuse if a ref changed outside `stack` since.
- Detail: Metadata blobs a mutation stops referencing are added to a tree at `refs/stack/keep` in the same ref transaction, so undo works after `gc`. Trees, like blobs, stay out of `git log --all`.
- Detail: Commits (for restack) will rely on branch reflogs for gc protection, since git logs every branch update; deleted branches need another mechanism.
- Reason: Bundled SQLite gives one known version everywhere at the cost of a C toolchain at build time.

2026-09-23T20:13Z@c8897497
- Category: restack
- Detail: Each commit is replayed with `git merge-tree --write-tree --merge-base` and written with `git commit-tree` (author and message copied; `-S` added when `commit.gpgSign` is set, since commit-tree ignores it). Nothing touches the index or working tree until the plan is complete.
- Detail: All branch moves and parent records apply in one journalled ref transaction. A conflict leaves that branch and its descendants untouched and restacks the rest; the user finishes with `git rebase --onto <parent> <offshoot> <branch>` then `stack restack <branch>`.
- Detail: If the checked-out branch moves, the journal moves the index and working tree first (`git read-tree -m -u`), refusing if tracked files are dirty; recovery moves it back if the refs never changed. Branches checked out in other worktrees are refused.
- Detail: Commits already in the parent (the replayed tree equals the parent's) are dropped. Merge commits are refused for now.
- Reason: git's own merge (ort) and commit machinery give results identical to `git rebase` (a differential test checks trees and messages) and honour the user's signing and merge config. One process per step is fast enough for stacks; the `GitRepo` port allows an in-process merge later.

2026-09-23T20:13Z@c8897497
- Category: environment
- Detail: `Workspace::discover_with(path, Environment::Exactly(vars))` runs every `git` subprocess with exactly `vars`. The default inherits the process environment minus variables that redirect git (`GIT_INDEX_FILE`, etc.).
- Reason: Tests must not inherit the developer's git config (it signed fixture commits with the developer's key). GUIs need it too: macOS GUI apps don't inherit the login shell's `PATH` or `SSH_AUTH_SOCK`.

2026-09-24T07:54Z@d42437b4
- Category: restack
- Detail: Replays merge trees (`git merge-tree --write-tree --merge-base=<tree> <tree> <tree>`), so previews (`stack check`) create no commits and never sign. Requires a git that accepts trees there (2.45+ is known to).
- Detail: A restack reports every conflicting branch and the branches blocked behind each; independent stacks still restack.
- Reason: Passive conflict checks (IDEAS) must be safe to run often: no signing prompts, no refs, no journal. They still write unreferenced tree objects, which `git gc` removes.

2026-09-24T07:59Z@72b3919e
- Category: push
- Detail: `stack push` pushes the line (see glossary) to the same branch names in one `git push --porcelain`, with `--force-with-lease=<branch>:<last fetched value>` per branch (empty: must not exist remotely). The user's `pre-push` hook runs.
- Detail: Remote: explicit, else the branch's upstream remote, else `origin`, else the only remote. Upstreams are set only for branches without one; pushing to a second remote (a backup) changes nothing locally.
- Detail: A branch whose upstream has a different name is refused. Rejections (e.g. stale lease) are reported per branch and fail the command.
- Reason: Leases make force-pushing restacked branches safe: a remote branch someone else changed since the last fetch is never overwritten.

2026-09-24T08:05Z@84e11653
- Category: archive
- Detail: Archiving moves `refs/heads/<b>` to `refs/stack/archive/<b>` (a commit ref) in one journalled transaction. Refused for trunks, branches with branches on them, and checked-out branches.
- Detail: Archived branches keep their recorded parent; restack's pruning skips them.
- Reason: Leaving `refs/heads/` hides the branch from `git branch`, IDEs, and every `stack` view, while the archive ref keeps its commits from `gc`. The cost: like `git stash`, archived commits appear in `git log --all`.

2026-09-24T08:08Z@250e88a9
- Category: review marks
- Detail: Marks (reviewed, tested, flagged with an optional note) are keyed by the commit's stable `git patch-id` (`patch:<id>`), or by commit id for merges and empty commits (`commit:<sha>`). Stored in the local SQLite store (schema v3), outside the op log.
- Reason: A patch-id changes only when the commit's diff changes, so marks survive restacks, rebases, and rewording, and lapse exactly when the reviewed change changes. Marks are personal, so they aren't shared through refs.

2026-09-24T08:16Z@ff9c9c61
- Category: worktrees
- Detail: Default worktree path: a hidden sibling of the main worktree, `.<repo>-<branch>` (`/` → `-`).
- Detail: A worktree added for a branch checked out elsewhere is a follower: detached at the tip, registered in the store (`follower` table, schema v4). Followers are synced after `stack` moves branches and on `stack worktree sync`, but only when their `HEAD` is a version of the branch (an ancestor of its tip, or a former tip); a follower with commits of its own is left for `stack land`.
- Detail: `land` fast-forwards the branch to the follower's `HEAD` in one journalled transaction, moving the holder worktree's files first (`git read-tree -m -u`, which carries non-clashing local changes and refuses clashing ones). Refused unless the follower's commits sit on the branch's current tip.
- Detail: The journal records which worktree a checkout moved, so recovery moves that one back.
- Reason: The main workspace keeps its checkout (no detached HEAD in the IDE) while other worktrees borrow the branch to commit; the fast-forward check guarantees the follower built on exactly what the holder has.
