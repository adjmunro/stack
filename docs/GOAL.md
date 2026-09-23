# Goal.md

`stack` is a layer on top of git that makes stacked, multi-branch workflows (restack, move, absorb, review tracking, conflict foresight) easy where plain git makes them hard.

- Git stays the backend. The repo remains a normal git repo; people without `stack` see only ordinary branches and commits.
- `stack` state is hidden from other users unless they also use `stack` and opt in to syncing it.
- One core library (`stack-core`) with thin surfaces on top:
  - a CLI (`stack`) usable in any repo;
  - a GUI, likely inside [ghostrealm](../../ghostrealm) (winit + wgpu), linking the core directly;
  - agent access via CLI `--json` first, and possibly an MCP server later.
- Correctness over features: an operation must change exactly what it claims, and nothing else. Every mutation is undoable.
