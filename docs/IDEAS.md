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
- Archive branches.
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

