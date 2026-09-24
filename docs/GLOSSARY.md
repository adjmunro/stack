# Glossary.md

Canonical vocabulary for the project. Prefer these names in code, schema, and docs. Names avoid clashes with the project language keywords, identifiers, and common libraries.

---

- **Trunk**: a long-lived branch with no parent, never rebased or pushed by `stack` (e.g. `develop`, `release/1.x`). A repo may have several in parallel; no ancestry is assumed between them.
- **Limb**: a trunk-marked branch whose parent is a regular branch, making it a base for branches leafward of it (e.g. a release checkpoint, or a teammate's branch you build on). Restacked with its parent. Internal term: users mark limbs with `stack trunk add`, and the role is inferred.
- **Stack**: the branches leafward of one trunk or limb, up to the next limbs or leaves.
- **Line**: a branch, its parents rootward up to the nearest trunk or limb (excluded), and its descendants leafward up to and including the next limbs. Siblings of the branch and of its parents are excluded. On a limb, its line is the limb and its descendants up to the next limbs (its own stack). The default scope for push and PRs.
- **Parent / child**: the branch a branch is based on, and the branches based on it.
- **Offshoot**: the commit where a branch splits from its parent.
- **Auto-tracking**: recording a branch's parent, derived from the commit graph, when none is recorded. A recorded parent holds while the branch still contains some version of it (current tip, a former tip, or matching patch-ids); otherwise it is re-derived and the change reported.
- **Pin**: a manually recorded parent. Never changed automatically; flagged if the graph contradicts it.
- **Rootward**: towards the trunk, through parents (Graphite: "downstack"). Continuing past the trunk reaches git's root commit, so the direction is the same.
- **Leafward**: away from the trunk, through children (Graphite: "upstack").
- **Leaf**: a branch with no children.
- **Restack**: rebase a branch and everything leafward of it, through limbs, onto its parent's current tip.
- **Move**: re-parent a branch: replay its own commits onto a new parent, then restack its descendants.
- **Archive**: put a branch away under `refs/stack/archive/`: out of `git branch` and every `stack` view, kept safe from `gc`, restorable.
- **Check**: a restack preview: which branches would move cleanly, conflict, or be blocked. Changes nothing.
- **Workspace**: the core facade over one git repo plus `stack` state. Entry point for every surface.
- **Surface**: a consumer of the core (CLI, GUI, MCP).
- **Metadata**: authoritative `stack` state stored in git refs under `refs/stack/`.
- **Cache**: local, rebuildable, or personal state stored in `.git/stack/` (SQLite).
- **Op log**: the record of every `stack` mutation (SQLite), used for undo and redo. It is also the journal.
- **Journal**: the op log's role as crash recovery: each mutation is recorded as pending before refs change, then marked done.
- **Keep tree**: `refs/stack/keep`, a tree of metadata blobs the op log may restore, so they survive `git gc`.
- **Former tip**: a commit a branch pointed at before (from its reflog). Counts as a version of the branch only if confirmed.
- **Review mark**: a flag (e.g. reviewed, flagged) on a commit, keyed by `patch-id` so it survives rebases that don't change the commit's diff.
- **Fixture**: a throwaway git repo built by tests.
- **Snapshot**: a full capture of a repo's HEAD, refs, config, index, working tree, objects, and `.git/stack/`, diffed to prove what an operation changed.
