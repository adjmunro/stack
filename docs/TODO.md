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

2026-09-23T10:37Z@507992a:
- Blockers: NO BLOCKERS
- Where: crates/core
- What: SQLite at `--git-common-dir`/stack; journal (intent → ref transaction → complete) with start-up recovery; op log + undo/redo.
- What: Fault-injection tests: abort between each journal step, assert recovery leaves the repo in a before or after snapshot, never between.

2026-09-23T10:37Z@507992a:
- Blockers: NO BLOCKERS
- Where: crates/core (GitRepo port)
- What: In-memory restack via `gix` tree merge. Differential tests against `git rebase` results; property tests (`proptest`) over random stacks.
- What: Commit signing for in-memory commits, or defer signing until push (see IDEAS.md).

2026-09-23T10:37Z@507992a:
- Blockers: NO BLOCKERS
- Where: crates/testkit
- What: Fixture builder for stacks/branch graphs.

2026-09-23T11:05Z@82e6359b:
- Blockers: NO BLOCKERS
- Where: crates/core/src/git.rs (GixRepo::discover), crates/core/tests
- What: In-process core tests open repos with gix defaults, so they read the developer's global git config. Isolate them (e.g. gix open options driven by an environment the testkit controls) without adding a test-only public API.
- What: Extend `Snapshot` to cover `.git/stack/` once SQLite lands.


---

## Short Horizon Subtasks
