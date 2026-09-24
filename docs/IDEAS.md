# Ideas.md

This is a human-managed file. Do not edit it unless removing completed ideas.
This file contains high-level project goals to draw from for discussion & planning. 
You should remove bullets point when they are or become complete and fully implemented.

---

- Mark commits as “read” / a green highlight after review and/or build & test run. Keep the highlight after rebase unless the commit delta actually changes. Red to flag things for later? Maybe even flag specific hunks of files of commits.
- Better “view parents/children of [branch]”. Maybe that’s the default for the trunk view.
- Synthetic octomerge branches with conflict resolution patches. Flattens when merged?
- Stack adaptors for both graphite and GitHub stack commands.
- I want to read the commit message and changes at the same time?
- Re-stacking, moving, absorbing (with interactive override / attribution), modify (specific commits, not just HEAD), multiple trunks
- Reduce / filter commit tree by changes to specific file(s).
- Passive conflict checks per branch or commit. And/or ahead-of-time resolution (e.g. when I make the branch, maybe I can see the conflict then and can decide/suggest the correct resolution now rather than later when it actually happens)
- Encrypt/Decrypt sensitive files w/ asymmetric keys?
- Vertical vs horizontal tree view
- “Choose neither” resolution
- Separate topological & behavioural changes?
- Track by signature, not by line? (Or line-content hash? Such as with hash-based edits for LLMs)
- Auto-save commits
- LLM generated messages & smart squash message rewrites
- Commit header linter / customisable rules
- Easier git hooks for toolchain checks
- Diff commit / branch deltas, not full historical delta (i.e. I often want to see the difference between two similar branches, but there can be a lot of noise if the base is very different)
- Swap between parallel branches and serial stacks
- When resolving conflicts, click & drag to include a selected hunk instead of / as well as whole hunk and individual line options
- Import filtering for diffs (Or some kind of “in `*.kt` ignore lines that start with `import *`)
- Indexer / find usages, implementation, declaration, injection etc
- I _badly_ need better worktree implementations. Or perhaps the GUI can provide a better facade? I need to be able to more freely switch between worktrees and branches owned by worktrees. The no-mulitple-workspace-checkouts thing drives me nutts. Maybe i need a bare repo and make everything a worktree or something.
  - Follower worktrees (borrow-checker style): the worktree that has the branch checked out owns it; other worktrees (and plain detached HEADs) follow it as read-only, detached, and move with each new commit on the branch. A "follower HEAD" / observer.
  - A write mutex between worktrees: still one writer and any number of readers. The main workspace holds it by preference (so it never shows a detached HEAD). When a follower wants to commit, it asks for the lock, commits, and hands it back for the main workspace to reclaim. Only needed when the branch is checked out in the main workspace; otherwise the follower just commits.
- Defer local commit signing until push.
- Perfect undo & redo operations with extensive history.
- Multiple/different custom staging areas?
- Split a branch into a stack from within `stack` (e.g. `a..d` → `b`, `c`, `d`, picking the commits for each), with tracking following automatically.
