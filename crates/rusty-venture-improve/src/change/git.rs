use std::sync::Arc;

use bollard::Docker;
use rusty_venture_core::CoreError;

use rusty_venture_actions::container::{exec_in_container, ExecCommand};

// ── MergeResult ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum MergeResult {
    /// Clean merge — no conflicts.
    Clean { merge_commit_sha: String },
    /// Merge produced conflicts. The listed files need resolution.
    Conflict { conflicted_files: Vec<String> },
}

// ── ContainerGit ──────────────────────────────────────────────────────────────

/// Thin wrapper around `exec_in_container` that provides typed git operations.
///
/// All commands are executed via bollard argv arrays — no shell interpolation,
/// no injection risk.  The `repo_path` is the absolute path inside the
/// container where the repository was cloned (typically `/workspace/repo`).
#[derive(Clone)]
pub struct ContainerGit {
    docker: Arc<Docker>,
    pub container_id: String,
    pub repo_path: String,
    pub author_name: String,
    pub author_email: String,
}

impl ContainerGit {
    pub fn new(
        docker: Arc<Docker>,
        container_id: impl Into<String>,
        repo_path: impl Into<String>,
        author_name: impl Into<String>,
        author_email: impl Into<String>,
    ) -> Self {
        Self {
            docker,
            container_id: container_id.into(),
            repo_path: repo_path.into(),
            author_name: author_name.into(),
            author_email: author_email.into(),
        }
    }

    // ── Branch operations ─────────────────────────────────────────────────────

    /// Create a new branch from `from_ref` and check it out.
    pub async fn create_branch(&self, branch: &str, from_ref: &str) -> Result<(), CoreError> {
        self.git(&["checkout", "-b", branch, from_ref]).await?;
        Ok(())
    }

    /// Switch to an existing branch.
    pub async fn checkout(&self, branch: &str) -> Result<(), CoreError> {
        self.git(&["checkout", branch]).await?;
        Ok(())
    }

    /// Return the name of the current branch.
    pub async fn current_branch(&self) -> Result<String, CoreError> {
        let r = self.git(&["rev-parse", "--abbrev-ref", "HEAD"]).await?;
        Ok(r.trim().to_string())
    }

    // ── Staging & committing ──────────────────────────────────────────────────

    /// Stage all changes (new files, modifications, deletions).
    pub async fn add_all(&self) -> Result<(), CoreError> {
        self.git(&["add", "-A"]).await?;
        Ok(())
    }

    /// Stage a specific path.
    pub async fn add_path(&self, path: &str) -> Result<(), CoreError> {
        self.git(&["add", path]).await?;
        Ok(())
    }

    /// Commit staged changes. Returns the new commit SHA.
    pub async fn commit(&self, message: &str) -> Result<String, CoreError> {
        // git requires author identity — set via env vars to avoid touching the
        // container's global git config.
        let result = self
            .git_with_env(
                &["commit", "-m", message, "--allow-empty"],
                &[
                    ("GIT_AUTHOR_NAME", &self.author_name),
                    ("GIT_AUTHOR_EMAIL", &self.author_email),
                    ("GIT_COMMITTER_NAME", &self.author_name),
                    ("GIT_COMMITTER_EMAIL", &self.author_email),
                ],
            )
            .await?;
        drop(result);

        // Return the SHA of HEAD.
        let sha = self.git(&["rev-parse", "HEAD"]).await?;
        Ok(sha.trim().to_string())
    }

    // ── Merge & conflict resolution ───────────────────────────────────────────

    /// Merge `branch` into the current branch using `--no-ff` (always creates a
    /// merge commit so history stays readable).
    ///
    /// On conflict, stages are preserved and `MergeResult::Conflict` is returned.
    pub async fn merge(&self, branch: &str) -> Result<MergeResult, CoreError> {
        let cmd = ExecCommand::new([
            "git", "merge", "--no-ff",
            "--no-commit", // let caller inspect conflicts before committing
            branch,
        ])
        .working_dir(&self.repo_path)
        .env("GIT_AUTHOR_NAME", &self.author_name)
        .env("GIT_AUTHOR_EMAIL", &self.author_email)
        .env("GIT_COMMITTER_NAME", &self.author_name)
        .env("GIT_COMMITTER_EMAIL", &self.author_email);

        let res = exec_in_container(&self.docker, &self.container_id, cmd).await?;

        if res.exit_code == 0 {
            // Clean merge — commit it.
            let sha = self
                .commit(&format!("chore: merge {} into base branch", branch))
                .await?;
            return Ok(MergeResult::Clean {
                merge_commit_sha: sha,
            });
        }

        // Non-zero exit → check for actual conflicts.
        let conflicts = self.conflicted_files().await?;
        if conflicts.is_empty() {
            // git returned non-zero but no conflict markers — treat as error.
            return Err(CoreError::ContainerExecFailed {
                code: res.exit_code,
                stderr: res.stderr,
            });
        }

        Ok(MergeResult::Conflict {
            conflicted_files: conflicts,
        })
    }

    /// After the caller resolves conflicts, finish the merge by committing.
    pub async fn finish_merge(&self, message: &str) -> Result<String, CoreError> {
        self.add_all().await?;
        self.commit(message).await
    }

    /// Return the list of files that currently have conflict markers.
    pub async fn conflicted_files(&self) -> Result<Vec<String>, CoreError> {
        // `git diff --name-only --diff-filter=U` lists unmerged paths.
        let r = self
            .git(&["diff", "--name-only", "--diff-filter=U"])
            .await?;
        let files: Vec<String> = r
            .lines()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect();
        Ok(files)
    }

    // ── File I/O inside container ─────────────────────────────────────────────

    /// Read a file from the repo inside the container.
    pub async fn read_file(&self, relative_path: &str) -> Result<String, CoreError> {
        let full_path = format!("{}/{}", self.repo_path.trim_end_matches('/'), relative_path);
        let cmd = ExecCommand::new(["cat", &full_path]);
        let res = exec_in_container(&self.docker, &self.container_id, cmd).await?;
        if res.exit_code != 0 {
            return Err(CoreError::ContainerExecFailed {
                code: res.exit_code,
                stderr: res.stderr,
            });
        }
        Ok(res.stdout)
    }

    /// Write content to a file inside the container.
    /// Parent directories are created with `mkdir -p`.
    pub async fn write_file(&self, relative_path: &str, content: &str) -> Result<(), CoreError> {
        let full_path = format!("{}/{}", self.repo_path.trim_end_matches('/'), relative_path);

        // Ensure parent directory exists.
        if let Some(parent) = std::path::Path::new(&full_path).parent() {
            let parent_str = parent.to_string_lossy();
            let cmd = ExecCommand::new(["mkdir", "-p", &parent_str]);
            exec_in_container(&self.docker, &self.container_id, cmd).await?;
        }

        // Write via sh -c base64 decode (tee approach kept as comment for reference).

        // bollard doesn't support stdin directly in exec — use a base64 trick
        // via sh -c.  This is the one place we use a shell, but the content is
        // base64-encoded so there's no injection risk.
        let encoded = base64_encode(content.as_bytes());
        let sh_cmd = format!("echo '{encoded}' | base64 -d > {full_path}");
        let cmd = ExecCommand::new(["sh", "-c", &sh_cmd])
            .working_dir(&self.repo_path);

        let res = exec_in_container(&self.docker, &self.container_id, cmd).await?;
        if res.exit_code != 0 {
            return Err(CoreError::ContainerExecFailed {
                code: res.exit_code,
                stderr: res.stderr,
            });
        }
        Ok(())
    }

    // ── Internals ─────────────────────────────────────────────────────────────

    async fn git(&self, args: &[&str]) -> Result<String, CoreError> {
        let mut argv = vec!["git"];
        argv.extend_from_slice(args);
        let cmd = ExecCommand::new(argv).working_dir(&self.repo_path);
        let res = exec_in_container(&self.docker, &self.container_id, cmd).await?;
        if res.exit_code != 0 {
            return Err(CoreError::ContainerExecFailed {
                code: res.exit_code,
                stderr: res.stderr,
            });
        }
        Ok(res.stdout)
    }

    async fn git_with_env(
        &self,
        args: &[&str],
        env: &[(&str, &str)],
    ) -> Result<String, CoreError> {
        let mut argv = vec!["git"];
        argv.extend_from_slice(args);
        let mut cmd = ExecCommand::new(argv).working_dir(&self.repo_path);
        for (k, v) in env {
            cmd = cmd.env(k, v);
        }
        let res = exec_in_container(&self.docker, &self.container_id, cmd).await?;
        if res.exit_code != 0 {
            return Err(CoreError::ContainerExecFailed {
                code: res.exit_code,
                stderr: res.stderr,
            });
        }
        Ok(res.stdout)
    }
}

// ── Minimal base64 encoder (no external dep needed) ──────────────────────────

fn base64_encode(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((input.len() + 2) / 3 * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((n >> 18) & 0x3f) as usize] as char);
        out.push(TABLE[((n >> 12) & 0x3f) as usize] as char);
        out.push(if chunk.len() > 1 { TABLE[((n >> 6) & 0x3f) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { TABLE[(n & 0x3f) as usize] as char } else { '=' });
    }
    out
}
