use rusty_venture_core::CoreError;
use rusty_venture_llm::{LlmConnector, LlmRequestBuilder};
use tracing::{info, warn};

use super::git::{ContainerGit, MergeResult};

// ── Public entry point ────────────────────────────────────────────────────────

/// Merge `branch` into the current branch.  If conflicts arise, the LLM
/// connector is used to resolve each conflicted file automatically.
///
/// Returns the merge commit SHA on success.
pub async fn merge_with_llm_resolution<C: LlmConnector>(
    git: &ContainerGit,
    branch: &str,
    connector: &C,
) -> Result<String, CoreError> {
    match git.merge(branch).await? {
        MergeResult::Clean { merge_commit_sha } => {
            info!(branch, sha = %merge_commit_sha, "Clean merge");
            Ok(merge_commit_sha)
        }
        MergeResult::Conflict { conflicted_files } => {
            warn!(
                branch,
                count = conflicted_files.len(),
                "Merge conflicts — invoking LLM resolver"
            );
            resolve_conflicts(git, &conflicted_files, connector).await?;
            let sha = git
                .finish_merge(&format!("chore: merge {} (conflicts resolved by AI)", branch))
                .await?;
            info!(branch, sha = %sha, "Conflict-resolved merge committed");
            Ok(sha)
        }
    }
}

// ── Conflict resolution ───────────────────────────────────────────────────────

/// For each conflicted file, read its content (including `<<<<<<<` / `=======`
/// / `>>>>>>>` markers), ask the LLM to produce the correct merged version,
/// then write the resolved content back.
async fn resolve_conflicts<C: LlmConnector>(
    git: &ContainerGit,
    files: &[String],
    connector: &C,
) -> Result<(), CoreError> {
    for file in files {
        let content_with_markers = git.read_file(file).await.unwrap_or_default();

        let resolved = ask_llm_to_resolve(connector, file, &content_with_markers)
            .await
            .map_err(|e| CoreError::Llm(e))?;

        git.write_file(file, &resolved).await?;
        info!(file, "Conflict resolved");
    }
    Ok(())
}

async fn ask_llm_to_resolve<C: LlmConnector>(
    connector: &C,
    file_path: &str,
    content: &str,
) -> Result<String, String> {
    let user_msg = format!(
        r#"The file `{file_path}` has unresolved git merge conflicts (marked with `<<<<<<<`, `=======`, `>>>>>>>`).

Produce the correctly merged version with all conflict markers removed.

Rules:
1. Keep ALL content that belongs in the final file.
2. When both sides have compatible additions, include both.
3. When sides conflict, prefer the incoming branch (`>>>>>>>` side) unless it would break the code.
4. Return ONLY the final file content — no explanation, no markdown fences, no extra text.

File content:
```
{content}
```"#
    );

    let request = LlmRequestBuilder::new()
        .system("You are a precise merge conflict resolver. Output only file content, nothing else.")
        .user(user_msg)
        .max_tokens(8192)
        .build();

    let response = connector
        .complete(request)
        .await
        .map_err(|e| e.to_string())?;

    response
        .text()
        .map(|t| t.to_string())
        .ok_or_else(|| "LLM returned empty response for conflict resolution".to_string())
}
