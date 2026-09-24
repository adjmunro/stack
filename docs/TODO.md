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

2026-09-24T07:54Z@d42437b4:
- Blockers: NO BLOCKERS
- Where: crates/core/src/git.rs (GixRepo::merge_trees), crates/core/src/restack.rs
- What: Run preview merges against a temporary object directory (`GIT_OBJECT_DIRECTORY` + alternates) so `check` writes nothing at all; then show conflict previews in `stack tree`.

2026-09-23T12:31Z@89920ef0:
- Blockers: NO BLOCKERS
- Where: crates/core/src/resolve.rs (Resolver::derive), crates/core/src/git.rs (Branch)
- What: Record observed branch tips in SQLite on every read, as extra former tips (same gates) for branches whose reflog is missing or expired. Also first-observed time as the age fallback.

2026-09-23T12:31Z@89920ef0:
- Blockers: NO BLOCKERS
- Where: crates/core/src/resolve.rs, crates/core/src/git.rs
- What: Performance: one walk per branch plus pairwise `merge_base`; unrelated histories (e.g. `gh-pages`) walk everything. Consider one shared walk and the commit-graph. Add a benchmark fixture first.

2026-09-23T20:13Z@c8897497:
- Blockers: NO BLOCKERS
- Where: crates/core/src/restack.rs, crates/core/src/journal.rs
- What: Resolve conflicts inside `stack` (stop, let the user resolve, `stack continue`) instead of handing off to `git rebase`.
- What: Replay merge commits.
- What: Deferred signing until push (see IDEAS.md), as an option.
- What: gc protection for commits of branches `stack` deletes (moved branches are covered by their reflogs).
- What: Property tests (`proptest`) over random stacks: restack then undo restores the snapshot; restack matches `git rebase`.

2026-09-23T10:37Z@507992a:
- Blockers: NO BLOCKERS
- Where: crates/testkit
- What: Fixture builder for stacks/branch graphs.

2026-09-23T11:05Z@82e6359b:
- Blockers: NO BLOCKERS
- Where: crates/core/src/git.rs (GixRepo::discover), crates/core/tests
- What: `git` subprocesses in tests are sealed via `Environment::Exactly`, but gix itself still reads the developer's global config in-process (e.g. reflog identity). Feed the environment to gix too (open options / config overrides).


---

## Short Horizon Subtasks
