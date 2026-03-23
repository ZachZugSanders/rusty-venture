# rusty-venture-actions

## Purpose
All concrete `Action` implementations live here. This crate is the "what to do" layer — it knows about Docker, git commands, file system analysis, and LLM report generation. The core crate knows nothing about these details.

## Sub-modules

### `src/container/`
Docker lifecycle management:
- `spawn.rs` — `SpawnContainerAction`: creates + starts a container from a `ContainerConfig`
- `exec.rs` — `exec_output(ctx, container_id, argv)`: runs a command inside a container and captures stdout as a `String`. This is the primitive used by nearly every analysis action.
- `guard.rs` — `ContainerGuard`: RAII wrapper that drops a detached tokio task to stop+remove the container on drop.
- `cache_image.rs` — `CacheRepoImageAction`: commits a running clone container as a local Docker image for reuse.

### `src/repo/`
All repository analysis actions. See `src/repo/CLAUDE.md` for the full breakdown.

### `src/graph.rs`
Converts maturity scan results into a `GraphNode`/`GraphEdge` structure for the 3D visualisation frontend.

## Container image roles
| Image constant | Variable | Network | Volume | Purpose |
|---|---|---|---|---|
| `RUNNER_CLONE_IMAGE` | `rv-clone-{id}` | ON | RW | git clone |
| `RUNNER_ANALYSIS_IMAGE` | `rv-analysis-{id}` | OFF | RO | static analysis |
| `RUNNER_EXECUTION_IMAGE` | `rv-exec-{id}` | ON | RW | build + test |

## Shared volume convention
Every run creates a named Docker volume `rusty-venture-{run_id}`. The clone container writes the repo to `/workspace`; the analysis/execution containers mount it read-only or read-write respectively. The volume is always removed after the run, even on failure.

## Adding a new analysis action
1. Create `src/repo/my_action.rs` implementing `Action<Input=(), Output=()>` that writes to a `CTX_*` key.
2. Add `pub mod my_action;` and `pub use my_action::...` in `src/repo/mod.rs`.
3. Add a `DagNode` for it in the Phase 2 DAG inside `run_repo_analysis` — it gets free parallelism.
4. Extract the result in the post-workflow section and pass it to `compute_maturity`.
