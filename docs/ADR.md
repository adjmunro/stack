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
