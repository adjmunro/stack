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

2026-09-23T12:00Z@585d2fb8
- Category: branch roles and scopes
- Detail: Trunks have no parent and are never rebased or pushed by `stack`. Limbs have a parent and act as bases for branches leafward of them.
- Detail: Restack runs leafward from a branch, through limbs.
- Detail: Push defaults to the current branch's line: parents rootward up to the nearest trunk or limb, plus descendants leafward up to the next limbs. Trunks, limbs, and siblings are excluded. `--rootward` and `--leafward` narrow it to one direction.
- Reason: A limb may belong to someone else, so ranges never push it. Restack must pass through limbs because their commits sit on the branch being moved.
