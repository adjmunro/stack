# Glossary.md

Canonical vocabulary for the project. Prefer these names in code, schema, and docs. Names avoid clashes with the project language keywords, identifiers, and common libraries.

---

- **Trunk**: a long-lived branch with no parent, never rebased or pushed by `stack` (e.g. `develop`, `release/1.x`). A repo may have several in parallel; no ancestry is assumed between them.
- **Limb**: a branch with a parent that is also a base for branches leafward of it (e.g. a release checkpoint, or a teammate's branch you build on). Restacked with its parent; never pushed as part of a range.
- **Stack**: the branches leafward of one trunk or limb, up to the next limbs or leaves.
- **Line**: a branch's parents rootward up to the nearest trunk or limb, plus its descendants leafward up to the next limbs; trunks, limbs, and siblings excluded. The default scope for push.
- **Parent / child**: the branch a branch is based on, and the branches based on it.
- **Offshoot**: the commit where a branch splits from its parent.
- **Auto-tracking**: recording a branch's parent, derived from the commit graph, when none is recorded. A recorded parent holds while the branch still contains some version of it (current tip, a former tip, or matching patch-ids); otherwise it is re-derived and the change reported.
- **Pin**: a manually recorded parent. Never changed automatically; flagged if the graph contradicts it.
- **Rootward**: towards the trunk, through parents (Graphite: "downstack"). Continuing past the trunk reaches git's root commit, so the direction is the same.
- **Leafward**: away from the trunk, through children (Graphite: "upstack").
- **Leaf**: a branch with no children.
- **Restack**: rebase a branch and everything leafward of it, through limbs, onto its parent's current tip.
- **Workspace**: the core facade over one git repo plus `stack` state. Entry point for every surface.
- **Surface**: a consumer of the core (CLI, GUI, MCP).
- **Metadata**: authoritative `stack` state stored in git refs under `refs/stack/`.
- **Cache**: local, rebuildable, or personal state stored in `.git/stack/` (SQLite).
- **Op log**: the record of every `stack` mutation, used for undo.
- **Journal**: intent records written before a mutation, used to recover from a crash mid-operation.
- **Review mark**: a flag (e.g. reviewed, flagged) on a commit, keyed by `patch-id` so it survives rebases that don't change the commit's diff.
- **Fixture**: a throwaway git repo built by tests.
- **Snapshot**: a full capture of a repo's refs, reachable objects, index, and working tree, diffed to prove what an operation changed.
