# rusty-venture-improve/src/change

## Purpose
Container-based repository repair pipeline. Takes a list of `RepairAction`s, schedules them based on file-ownership conflicts, applies them inside a running Docker container, and manages the git branching strategy — including LLM-assisted merge conflict resolution.

## Module map

| File | Type/Function | Role |
|---|---|---|
| `repair.rs` | `RepairAction` trait | Unit of work — declares files it touches, applies changes |
| `scheduler.rs` | `ChangeScheduler` | Groups actions into sequential/parallel batches by file overlap |
| `git.rs` | `ContainerGit` | Typed git operations inside a container via `exec_in_container` |
| `apply.rs` | `ApplyChangesWorkflow` | Drives the full branching strategy across all batches |
| `merge.rs` | `merge_with_llm_resolution` | Merges a branch; uses LLM to resolve any conflicts |

## Branching strategy

```
base_branch  ──────────────────────────────────────────────────────►
               │  parallel-0   │  parallel-1
               ├── rv/change/xyz/p0-0 ──► (merge back, LLM resolves conflicts)
               ├── rv/change/xyz/p0-1 ──► (merge back, LLM resolves conflicts)
               │
               └── sequential actions commit directly to base_branch
```

- **Parallel batch**: disjoint file sets → each action gets its own branch, runs concurrently via `join_all`, then merges back.
- **Sequential batch**: overlapping file sets → actions commit directly to `base_branch` in order.
- Single-action batches are always sequential (no branching overhead).

## Implementing a new RepairAction

```rust
struct AddLicenseAction;

#[async_trait]
impl RepairAction for AddLicenseAction {
    fn name(&self) -> &str { "add-license" }
    fn affected_files(&self) -> Vec<String> { vec!["LICENSE".into()] }

    async fn apply(&self, git: &ContainerGit, ctx: &ExecutionContext) -> Result<(), CoreError> {
        git.write_file("LICENSE", LICENSE_TEXT).await
    }
}
```

**Contract**: `apply` writes files but does NOT call `git add` or `git commit` — `ApplyChangesWorkflow` handles all git staging and committing.

## Key invariants
- `ContainerGit` uses argv arrays throughout — no shell string interpolation, no injection risk.
- File writes use `sh -c 'echo <base64> | base64 -d > path'` to avoid needing stdin support in bollard exec.
- `ChangeScheduler` uses prefix-overlap detection, not just exact pattern matching, so `src/**` correctly conflicts with `src/main.rs`.
- Results are stored in context under `CTX_APPLIED_CHANGES` after the workflow completes.
