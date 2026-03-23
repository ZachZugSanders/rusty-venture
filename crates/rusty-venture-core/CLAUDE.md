# rusty-venture-core

## Purpose
The foundational crate. Defines all traits and engines. Zero business logic — no Docker, no LLM calls, no file I/O. Everything else depends on this; it depends on nothing internal.

## Key types

### `Action` trait (`src/action.rs`)
```rust
trait Action: Send + Sync {
    type Input: Send + Sync + 'static;
    type Output: Send + Sync + 'static;
    fn name(&self) -> &str;
    async fn execute(&self, ctx: &ExecutionContext, input: Self::Input) -> Result<Self::Output, CoreError>;
}
```
The `AnyAction` blanket impl type-erases `Action` so it can be stored in a `Vec<Box<dyn AnyAction>>`. This is how the workflow engine holds heterogeneous steps.

### `ExecutionContext` (`src/context.rs`)
A cheaply-cloneable `Arc<RwLock<HashMap<String, Box<dyn Any + Send + Sync>>>>`. Actions write typed values under string keys (`CTX_*` constants) and read them back with `ctx.require::<T>(key)` or `ctx.get::<T>(key)`. Cloning the context is O(1) — all clones share the same map, which is how the DAG engine passes context into parallel tasks.

### `WorkflowEngine` + `Workflow` (`src/workflow.rs`)
Sequential step runner. Iterates over `Vec<Step>`, calls `execute_step` for each, handles retry logic and `OnFailure` policy (Abort / Continue / LlmRemediate). Use this for ordered pipelines where step N depends on step N-1.

### `DagEngine` + `DagWorkflow` (`src/dag.rs`)
Parallel step runner using Kahn's topological sort + `tokio::task::JoinSet`. Nodes with no unmet `depends_on` start immediately. As each node completes, its successors' in-degrees are decremented and newly-ready nodes are spawned. Use this when steps are mutually independent — the analysis phase runs six actions in parallel this way.

### `RetryStrategy` (`src/retry.rs`)
`Fixed { max_attempts, delay }` or `None`. Applied per-step by the workflow engine.

## What NOT to put here
- Docker / bollard calls → `rusty-venture-actions`
- HTTP / axum → `rusty-venture-server`
- LLM REST calls → `rusty-venture-llm`
- Database → `rusty-venture-store`
