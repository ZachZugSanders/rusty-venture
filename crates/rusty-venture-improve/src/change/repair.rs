use async_trait::async_trait;
use rusty_venture_core::{context::ExecutionContext, CoreError};

use super::git::ContainerGit;

/// A single improvement action that can be applied to a cloned repository
/// inside a container.
///
/// Implementors must declare which files they will touch via `affected_files()`
/// so the [`crate::change::scheduler::ChangeScheduler`] can group them into
/// sequential chains or parallel branches automatically.
///
/// ## Contract
/// - `apply` is called while the container's git working tree is checked out on
///   the correct branch for this action.  The action writes files, modifies
///   code, etc. but does **not** call `git add` or `git commit` — the
///   scheduler/workflow handles all git operations.
/// - `affected_files` returns glob patterns relative to the repo root.
///   Use `**/*.rs` for broad patterns or `Cargo.toml` for exact files.
///   Overlapping globs between two actions cause them to be scheduled
///   sequentially; disjoint sets allow parallel execution on separate branches.
#[async_trait]
pub trait RepairAction: Send + Sync {
    /// Human-readable name shown in logs and progress events.
    fn name(&self) -> &str;

    /// Glob patterns (relative to repo root) for every file this action may
    /// create or modify.  Used by `ChangeScheduler` to detect conflicts.
    ///
    /// Be conservative: if in doubt, include the file.
    fn affected_files(&self) -> Vec<String>;

    /// Apply the repair.  Write files using `git.write_file(path, content)`.
    /// Do not call `git add` or `git commit` — the workflow does that.
    async fn apply(&self, git: &ContainerGit, ctx: &ExecutionContext) -> Result<(), CoreError>;

    /// Optional: a short, human-readable description of what this action does.
    fn description(&self) -> String {
        format!("Apply repair action: {}", self.name())
    }

    /// Optional: the commit message to use when this action's changes are
    /// committed.  Defaults to `"fix: <name>"`.
    fn commit_message(&self) -> String {
        format!("fix: {}", self.name())
    }
}
