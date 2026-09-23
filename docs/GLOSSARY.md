# Glossary.md

Canonical vocabulary for the project. Prefer these names in code, schema, and docs. Names avoid clashes with the project language keywords, identifiers, and common libraries.

---

- **Trunk**: a long-lived base branch that stacks target (e.g. `develop`). A repo may have several.
- **Stack**: a chain of branches, each based on its parent, rooted on a trunk.
- **Parent / child**: the branch a branch is based on, and the branches based on it.
- **Rootward**: towards the trunk, through parents (Graphite: "downstack").
- **Leafward**: away from the trunk, through children (Graphite: "upstack").
- **Leaf**: a branch with no children.
- **Restack**: rebase a branch (and its descendants) onto its parent's current tip.
- **Workspace**: the core facade over one git repo plus `stack` state. Entry point for every surface.
- **Surface**: a consumer of the core (CLI, GUI, MCP).
- **Metadata**: authoritative `stack` state stored in git refs under `refs/stack/`.
- **Cache**: local, rebuildable, or personal state stored in `.git/stack/` (SQLite).
- **Op log**: the record of every `stack` mutation, used for undo.
- **Journal**: intent records written before a mutation, used to recover from a crash mid-operation.
- **Review mark**: a flag (e.g. reviewed, flagged) on a commit, keyed by `patch-id` so it survives rebases that don't change the commit's diff.
- **Fixture**: a throwaway git repo built by tests.
- **Snapshot**: a full capture of a repo's refs, reachable objects, index, and working tree, diffed to prove what an operation changed.
