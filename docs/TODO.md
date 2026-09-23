# Todo.md

## Instructions
- Living document managed by you, the agent. Append tasks/subtasks to be done, then remove them when complete.
- Tasks not expected to be completed this branch are "long horizon".
- Subtasks to be completed before the branch is merged are "short horizon". This list should be empty again when all branch goals are achieved.
- Cut puffery and exposition. Provide only enough detail & context to complete the task/subtask.
- Tasks/subtasks can contain multiple "blockers", "where", or "what" sections.
- Re-verify prerequistes, relevancy, and accuracy before starting. 

## Format
```
<created-at-datetime>@<commit-sha>:
- Blockers: <NO BLOCKERS|prerequisite task(s)> 
- Where: <filepath(s) & class/function signatures etc>
- What: <what needs doing>

```

---

## Long Horizon Tasks

2026-09-23T12:31Z@89920ef0:
- Blockers: NO BLOCKERS
- Where: crates/core/src/resolve.rs (Resolver::derive), crates/core/src/git.rs (Branch)
- What: Record observed branch tips in SQLite on every read, as extra former tips (same gates) for branches whose reflog is missing or expired. Also first-observed time as the age fallback.

2026-09-23T12:31Z@89920ef0:
- Blockers: NO BLOCKERS
- Where: crates/core/src/resolve.rs, crates/core/src/git.rs
- What: Performance: one walk per branch plus pairwise `merge_base`; unrelated histories (e.g. `gh-pages`) walk everything. Consider one shared walk and the commit-graph. Add a benchmark fixture first.
- What: Prune `refs/stack/branches/*` records for deleted branches during mutations.

2026-09-23T10:37Z@507992a:
- Blockers: NO BLOCKERS
- Where: crates/core (GitRepo port)
- What: In-memory restack via `gix` tree merge. Differential tests against `git rebase` results; property tests (`proptest`) over random stacks.
- What: Commit signing for in-memory commits, or defer signing until push (see IDEAS.md).
- What: Undo of branch moves: gc protection for deleted branches' commits; update the working tree when the checked-out branch moves (refuse if dirty).

2026-09-23T10:37Z@507992a:
- Blockers: NO BLOCKERS
- Where: crates/testkit
- What: Fixture builder for stacks/branch graphs.

2026-09-23T11:05Z@82e6359b:
- Blockers: NO BLOCKERS
- Where: crates/core/src/git.rs (GixRepo::discover), crates/core/tests
- What: In-process core tests open repos with gix defaults, so they read the developer's global git config. Isolate them (e.g. gix open options driven by an environment the testkit controls) without adding a test-only public API.


---

## Short Horizon Subtasks
