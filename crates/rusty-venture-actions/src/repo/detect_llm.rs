use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use bollard::Docker;
use serde::{Deserialize, Serialize};
use rusty_venture_core::{action::Action, context::ExecutionContext, error::CoreError};

use crate::container::{exec_in_container, ExecCommand, CTX_CONTAINER_ID, CTX_DOCKER_CLIENT};

pub const CTX_LLM_CONFIG: &str = "repo.llm_config";

/// Enumeration of known AI/LLM tools and providers that can be configured
/// at the project level.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum LlmProvider {
    /// Anthropic Claude (detected via .claude/ directory, CLAUDE.md, or claude.json)
    Claude,
    /// OpenAI GPT-4/o (detected via .openai, openai.json, or direct API key references)
    OpenAi,
    /// Google Gemini (detected via .gemini/, gemini.json, or GEMINI.md)
    Gemini,
    /// GitHub Copilot (detected via .github/copilot-instructions.md or AGENTS.md)
    Copilot,
    /// Cursor IDE (detected via .cursor/ directory or .cursorrules)
    Cursor,
    /// xAI Grok (detected via .grok/ or grok.toml)
    Grok,
    /// Meta Llama / local models (detected via ollama config, llama.cpp, or llamafile)
    Llama,
    /// Mistral AI (detected via .mistral/ or mistral.toml)
    Mistral,
    /// DeepSeek (detected via .deepseek/ or deepseek.toml)
    DeepSeek,
    /// AWS Bedrock (detected via bedrock config or AWS SDK usage patterns)
    AwsBedrock,
    /// Azure OpenAI (detected via azure_openai config)
    AzureOpenAi,
    /// Custom or unknown LLM configuration
    Custom,
}

impl LlmProvider {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Claude => "Anthropic Claude",
            Self::OpenAi => "OpenAI GPT",
            Self::Gemini => "Google Gemini",
            Self::Copilot => "GitHub Copilot",
            Self::Cursor => "Cursor",
            Self::Grok => "xAI Grok",
            Self::Llama => "Meta Llama / Local",
            Self::Mistral => "Mistral AI",
            Self::DeepSeek => "DeepSeek",
            Self::AwsBedrock => "AWS Bedrock",
            Self::AzureOpenAi => "Azure OpenAI",
            Self::Custom => "Custom / Unknown",
        }
    }
}

/// Result of scanning a repository for AI/LLM configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmConfig {
    /// Detected LLM providers (a repo may have multiple).
    pub providers: Vec<LlmProvider>,
    /// Whether a structured project-level LLM config file was found
    /// (e.g. .llm-config, llm.toml, or AGENTS.md with LLM= field).
    pub has_project_config: bool,
    /// Whether AI context files (CLAUDE.md, AGENTS.md, .cursorrules, etc.)
    /// are present to help the LLM understand the codebase.
    pub has_context_files: bool,
    /// Number of CLAUDE.md / AGENTS.md files found across all directories.
    pub context_file_count: usize,
    /// Whether the detected LLM config specifies a model name.
    pub has_model_specified: bool,
    /// The detected model name/ID if present.
    pub detected_model: Option<String>,
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            providers: vec![],
            has_project_config: false,
            has_context_files: false,
            context_file_count: 0,
            has_model_specified: false,
            detected_model: None,
        }
    }
}

/// Detects AI/LLM tooling configuration from the repository filesystem.
/// Runs in the analysis container against /workspace.
pub struct DetectLlmConfigAction;

#[async_trait]
impl Action for DetectLlmConfigAction {
    type Input = ();
    type Output = ();

    fn name(&self) -> &str {
        "detect-llm-config"
    }

    async fn execute(&self, ctx: &ExecutionContext, _: ()) -> Result<(), CoreError> {
        let docker: Arc<Docker> = ctx.require::<Arc<Docker>>(CTX_DOCKER_CLIENT).await?;
        let container_id: String = ctx.require::<String>(CTX_CONTAINER_ID).await?;

        let result = detect_llm_config_in_container(&docker, &container_id).await;
        ctx.insert(CTX_LLM_CONFIG, result).await;
        Ok(())
    }
}

/// Run a single shell command in the container and return trimmed stdout.
/// Returns an empty string on error so callers can treat it as a no-match.
async fn exec_str(docker: &Docker, container_id: &str, cmd: &[&str]) -> String {
    let owned: Vec<String> = cmd.iter().map(|s| s.to_string()).collect();
    match exec_in_container(docker, container_id, ExecCommand::new(owned)).await {
        Ok(result) => result.stdout.trim().to_string(),
        Err(_) => String::new(),
    }
}

async fn detect_llm_config_in_container(
    docker: &Docker,
    container_id: &str,
) -> LlmConfig {
    let mut providers: Vec<LlmProvider> = vec![];
    let mut has_project_config = false;
    let mut has_model_specified = false;
    let mut detected_model: Option<String> = None;

    // ── Check for provider-specific directories and files ────────────────────
    let probe_files: &[(&str, LlmProvider)] = &[
        (".claude", LlmProvider::Claude),
        ("CLAUDE.md", LlmProvider::Claude),
        (".cursor", LlmProvider::Cursor),
        (".cursorrules", LlmProvider::Cursor),
        (".gemini", LlmProvider::Gemini),
        (".grok", LlmProvider::Grok),
        (".mistral", LlmProvider::Mistral),
        (".deepseek", LlmProvider::DeepSeek),
        ("ollama.json", LlmProvider::Llama),
        ("llamafile", LlmProvider::Llama),
    ];

    for (path, provider) in probe_files {
        let cmd = format!("test -e /workspace/repo/{path} && echo yes || echo no");
        let out = exec_str(docker, container_id, &["sh", "-c", &cmd]).await;
        if out == "yes" && !providers.contains(provider) {
            providers.push(provider.clone());
        }
    }

    // Check for AGENTS.md (GitHub Copilot / generic AI instructions)
    {
        let out = exec_str(docker, container_id, &[
            "sh", "-c",
            "test -f /workspace/repo/AGENTS.md && echo yes || echo no",
        ]).await;
        if out == "yes" {
            if !providers.contains(&LlmProvider::Copilot) {
                providers.push(LlmProvider::Copilot);
            }
            has_project_config = true;
        }
    }

    // Check for .github/copilot-instructions.md
    {
        let out = exec_str(docker, container_id, &[
            "sh", "-c",
            "test -f /workspace/repo/.github/copilot-instructions.md && echo yes || echo no",
        ]).await;
        if out == "yes" && !providers.contains(&LlmProvider::Copilot) {
            providers.push(LlmProvider::Copilot);
        }
    }

    // ── Count context/instruction files (CLAUDE.md, AGENTS.md, etc.) ─────────
    let (context_file_count, has_context_files) = {
        let out = exec_str(docker, container_id, &[
            "sh", "-c",
            "find /workspace/repo -name 'CLAUDE.md' -o -name 'AGENTS.md' -o -name '.cursorrules' 2>/dev/null | wc -l",
        ]).await;
        let count: usize = out.parse().unwrap_or(0);
        (count, count > 0)
    };

    // ── Check for a .llm-config or llm.toml project config file ─────────────
    {
        let out = exec_str(docker, container_id, &[
            "sh", "-c",
            "test -f /workspace/repo/.llm-config && echo yes || test -f /workspace/repo/llm.toml && echo yes || echo no",
        ]).await;
        if out == "yes" {
            has_project_config = true;
            // Try to read model name from the config
            let content = exec_str(docker, container_id, &[
                "sh", "-c",
                "cat /workspace/repo/.llm-config 2>/dev/null || cat /workspace/repo/llm.toml 2>/dev/null",
            ]).await;
            for line in content.lines() {
                let line = line.trim();
                let model_val = line
                    .strip_prefix("MODEL=")
                    .or_else(|| line.strip_prefix("model="))
                    .or_else(|| {
                        line.strip_prefix("model = \"").map(|s| s.trim_end_matches('"'))
                    });
                if let Some(model) = model_val {
                    let m = model.trim_matches('"').trim_matches('\'').to_string();
                    if !m.is_empty() {
                        has_model_specified = true;
                        detected_model = Some(m);
                    }
                }
            }
        }
    }

    // If .claude/settings.json or .claude/settings.local.json exists, try to extract model
    if providers.contains(&LlmProvider::Claude) && detected_model.is_none() {
        let content = exec_str(docker, container_id, &[
            "sh", "-c",
            "cat /workspace/repo/.claude/settings.json 2>/dev/null || cat /workspace/repo/.claude/settings.local.json 2>/dev/null",
        ]).await;
        // Look for "model": "..." in the JSON
        for line in content.lines() {
            let line = line.trim();
            if line.contains("\"model\"") {
                if let Some(start) = line.find(": \"") {
                    let rest = &line[start + 3..];
                    if let Some(end) = rest.find('"') {
                        let m = rest[..end].to_string();
                        if !m.is_empty() {
                            has_model_specified = true;
                            detected_model = Some(m);
                            has_project_config = true;
                        }
                    }
                }
            }
        }
    }

    LlmConfig {
        providers,
        has_project_config,
        has_context_files,
        context_file_count,
        has_model_specified,
        detected_model,
    }
}

/// Local (no-container) version for use in skip_container mode.
pub fn detect_llm_config_local(repo_path: &Path) -> LlmConfig {
    let mut providers: Vec<LlmProvider> = vec![];
    let mut has_project_config = false;
    let mut has_model_specified = false;
    let mut detected_model: Option<String> = None;

    let probe: &[(&str, LlmProvider)] = &[
        (".claude", LlmProvider::Claude),
        ("CLAUDE.md", LlmProvider::Claude),
        (".cursor", LlmProvider::Cursor),
        (".cursorrules", LlmProvider::Cursor),
        (".gemini", LlmProvider::Gemini),
        (".grok", LlmProvider::Grok),
        ("AGENTS.md", LlmProvider::Copilot),
        (".github/copilot-instructions.md", LlmProvider::Copilot),
        ("ollama.json", LlmProvider::Llama),
    ];

    for (rel, provider) in probe {
        if repo_path.join(rel).exists() && !providers.contains(provider) {
            providers.push(provider.clone());
            if *rel == "AGENTS.md" {
                has_project_config = true;
            }
        }
    }

    // Count context files
    fn count_context_files(dir: &Path) -> usize {
        let mut count = 0;
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    count += count_context_files(&path);
                } else if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if matches!(name, "CLAUDE.md" | "AGENTS.md" | ".cursorrules") {
                        count += 1;
                    }
                }
            }
        }
        count
    }
    let context_file_count = count_context_files(repo_path);

    // Check .llm-config
    let llm_config_path = repo_path.join(".llm-config");
    let llm_toml_path = repo_path.join("llm.toml");
    let config_path = if llm_config_path.exists() {
        Some(llm_config_path)
    } else if llm_toml_path.exists() {
        Some(llm_toml_path)
    } else {
        None
    };

    if let Some(p) = config_path {
        has_project_config = true;
        if let Ok(content) = std::fs::read_to_string(&p) {
            for line in content.lines() {
                let line = line.trim();
                if let Some(model) = line
                    .strip_prefix("MODEL=")
                    .or_else(|| line.strip_prefix("model="))
                {
                    let m = model.trim_matches('"').trim_matches('\'').to_string();
                    if !m.is_empty() {
                        has_model_specified = true;
                        detected_model = Some(m);
                    }
                }
            }
        }
    }

    LlmConfig {
        providers,
        has_project_config,
        has_context_files: context_file_count > 0,
        context_file_count,
        has_model_specified,
        detected_model,
    }
}
