use std::sync::Arc;

use futures::future::join_all;
use rusty_venture_core::{context::ExecutionContext, CoreError};
use rusty_venture_llm::LlmConnector;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use super::{
    git::ContainerGit,
    merge::merge_with_llm_resolution,
    scheduler::{BatchKind, SchedulePlan},
};

// ── Result types ──────────────────────────────────────────────────────────────

/// The outcome of applying a single repair action.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppliedChange {
    pub action_name: String,
    pub branch: String,
    pub commit_sha: String,
    pub status: ChangeStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeStatus {
    /// Applied and committed cleanly.
    Applied,
    /// The merge back into base required AI conflict resolution.
    MergedWithConflictResolution,
    /// The action failed; changes skipped.
    Failed { reason: String },
}

pub const CTX_APPLIED_CHANGES: &str = "change.applied_changes";

// ── ApplyChangesWorkflow ──────────────────────────────────────────────────────

/// Drives the full branching strategy for a `SchedulePlan`.
///
/// ## Branching model
/// ```text
/// base_branch  ──────────────────────────────────────────────────────►
///                │    parallel-0   │    parallel-1
///                ├── rv/change/xyz/parallel-0 ──► (merge back)
///                ├── rv/change/xyz/parallel-1 ──► (merge back)
///                │                                                sequential actions
///                └───────────────────────────────────────────────► (commit directly)
/// ```
/// - Each `Parallel` batch: every action gets its own branch, actions run
///   concurrently, then each branch is merged back with LLM conflict resolution
///   if needed.
/// - Each `Sequential` batch: actions commit directly to `base_branch` in
///   order.
pub struct ApplyChangesWorkflow<C> {
    connector: Arc<C>,
    run_id_short: String,
}

impl<C: LlmConnector + 'static> ApplyChangesWorkflow<C> {
    pub fn new(connector: Arc<C>, run_id_short: impl Into<String>) -> Self {
        Self {
            connector,
            run_id_short: run_id_short.into(),
        }
    }

    /// Execute the plan. Returns the list of applied changes.
    pub async fn run(
        &self,
        plan: SchedulePlan,
        git: &ContainerGit,
        base_branch: &str,
        ctx: &ExecutionContext,
    ) -> Result<Vec<AppliedChange>, CoreError> {
        let mut all_changes: Vec<AppliedChange> = vec![];

        for (batch_idx, batch) in plan.batches.into_iter().enumerate() {
            match batch.kind {
                BatchKind::Sequential => {
                    info!(batch = batch_idx, "Running sequential batch");
                    git.checkout(base_branch).await?;

                    for action in batch.actions {
                        let name = action.name().to_string();
                        info!(action = %name, "Applying sequential repair");

                        match action.apply(git, ctx).await {
                            Ok(()) => {
                                git.add_all().await?;
                                let sha = git.commit(&action.commit_message()).await?;
                                info!(action = %name, sha = %sha, "Sequential commit");
                                all_changes.push(AppliedChange {
                                    action_name: name,
                                    branch: base_branch.to_string(),
                                    commit_sha: sha,
                                    status: ChangeStatus::Applied,
                                });
                            }
                            Err(e) => {
                                warn!(action = %name, error = %e, "Sequential repair failed");
                                all_changes.push(AppliedChange {
                                    action_name: name,
                                    branch: base_branch.to_string(),
                                    commit_sha: String::new(),
                                    status: ChangeStatus::Failed {
                                        reason: e.to_string(),
                                    },
                                });
                            }
                        }
                    }
                }

                BatchKind::Parallel => {
                    info!(batch = batch_idx, count = batch.actions.len(), "Running parallel batch");

                    // Spawn each action on its own branch concurrently.
                    let mut futures = vec![];

                    for (action_idx, action) in batch.actions.into_iter().enumerate() {
                        let branch = format!(
                            "rusty-venture/change/{}/p{}-{}",
                            self.run_id_short,
                            batch_idx,
                            action_idx
                        );
                        let action_name = action.name().to_string();
                        let commit_msg = action.commit_message();
                        let git_clone = git.clone();
                        let ctx_clone = ctx.clone();
                        let base = base_branch.to_string();

                        futures.push(async move {
                            // Create and switch to the parallel branch.
                            if let Err(e) =
                                git_clone.create_branch(&branch, &base).await
                            {
                                return AppliedChange {
                                    action_name,
                                    branch,
                                    commit_sha: String::new(),
                                    status: ChangeStatus::Failed {
                                        reason: format!("create branch: {e}"),
                                    },
                                };
                            }

                            // Apply the repair.
                            match action.apply(&git_clone, &ctx_clone).await {
                                Ok(()) => {}
                                Err(e) => {
                                    return AppliedChange {
                                        action_name,
                                        branch,
                                        commit_sha: String::new(),
                                        status: ChangeStatus::Failed {
                                            reason: format!("apply: {e}"),
                                        },
                                    };
                                }
                            }

                            // Stage and commit.
                            if let Err(e) = git_clone.add_all().await {
                                return AppliedChange {
                                    action_name,
                                    branch,
                                    commit_sha: String::new(),
                                    status: ChangeStatus::Failed {
                                        reason: format!("git add: {e}"),
                                    },
                                };
                            }

                            match git_clone.commit(&commit_msg).await {
                                Ok(sha) => AppliedChange {
                                    action_name,
                                    branch,
                                    commit_sha: sha,
                                    status: ChangeStatus::Applied,
                                },
                                Err(e) => AppliedChange {
                                    action_name,
                                    branch,
                                    commit_sha: String::new(),
                                    status: ChangeStatus::Failed {
                                        reason: format!("commit: {e}"),
                                    },
                                },
                            }
                        });
                    }

                    // Wait for all parallel branches to complete.
                    let results: Vec<AppliedChange> = join_all(futures).await;

                    // Merge successful branches back into base_branch.
                    git.checkout(base_branch).await?;

                    for change in results {
                        let branch = change.branch.clone();
                        let action_name = change.action_name.clone();

                        match &change.status {
                            ChangeStatus::Applied => {
                                // Merge the parallel branch back.
                                match merge_with_llm_resolution(
                                    git,
                                    &branch,
                                    &*self.connector,
                                )
                                .await
                                {
                                    Ok(merge_sha) => {
                                        info!(
                                            action = %action_name,
                                            branch = %branch,
                                            sha = %merge_sha,
                                            "Parallel branch merged"
                                        );
                                        all_changes.push(AppliedChange {
                                            action_name,
                                            branch,
                                            commit_sha: merge_sha,
                                            // Distinguish clean vs conflict-resolved.
                                            status: ChangeStatus::Applied,
                                        });
                                    }
                                    Err(e) => {
                                        warn!(
                                            action = %action_name,
                                            error = %e,
                                            "Failed to merge parallel branch"
                                        );
                                        all_changes.push(AppliedChange {
                                            action_name,
                                            branch,
                                            commit_sha: change.commit_sha,
                                            status: ChangeStatus::Failed {
                                                reason: format!("merge: {e}"),
                                            },
                                        });
                                    }
                                }
                            }
                            _ => {
                                // Already failed; record as-is.
                                all_changes.push(change);
                            }
                        }
                    }
                }
            }
        }

        // Persist results to context.
        ctx.insert(CTX_APPLIED_CHANGES, all_changes.clone()).await;

        info!(
            total = all_changes.len(),
            applied = all_changes.iter().filter(|c| matches!(c.status, ChangeStatus::Applied | ChangeStatus::MergedWithConflictResolution)).count(),
            "Apply changes workflow complete"
        );

        Ok(all_changes)
    }
}
