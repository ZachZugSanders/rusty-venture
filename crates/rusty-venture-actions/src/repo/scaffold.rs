use std::sync::Arc;

use async_trait::async_trait;
use rusty_venture_core::{action::Action, context::ExecutionContext, CoreError};
use rusty_venture_llm::LlmConnector;
use serde::{Deserialize, Serialize};
use tracing::info;

use super::{
    audit_files::AuditReport, detect_language::Language, find_dockerfiles::DockerfileReport,
    governance::GovernanceReport, maturity::MaturityScore, report::FinalReport, CTX_AUDIT_REPORT,
    CTX_DETECTED_LANGUAGES, CTX_DOCKERFILE_REPORT, CTX_FINAL_REPORT, CTX_GOVERNANCE_REPORT,
};
use crate::repo::analyze_deps::{DependencyReport, CTX_DEPENDENCY_REPORT};
use crate::repo::detect_language::DetectedLanguages;
use crate::repo::maturity::CTX_MATURITY_SCORE;

pub const CTX_SCAFFOLD_SPEC: &str = "scaffold.spec";
pub const CTX_IMPROVEMENT_INTENT: &str = "scaffold.intent";

// ── ScaffoldSpec ──────────────────────────────────────────────────────────────

/// The complete blueprint generated before running improvement actions.
///
/// Contains all the names, types, file paths, and directory structure that
/// parallel actions (containerize, generate DAL, generate API endpoints, etc.)
/// need to produce self-consistent output without stepping on each other.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScaffoldSpec {
    /// Human-readable summary of what this spec intends to accomplish.
    pub intent_summary: String,

    /// The directory tree that will exist after all actions complete.
    pub directory_tree: DirectoryNode,

    /// All types/structs/classes/interfaces that will be created.
    pub types: Vec<TypeSpec>,

    /// Named variables or constants shared across generated files.
    pub variables: Vec<VariableSpec>,

    /// Individual files that will be created or modified.
    pub files: Vec<FileSpec>,

    /// Ordered list of improvement actions the DAG should execute.
    /// Each entry names a well-known action ID (e.g. `"containerize"`,
    /// `"generate-dal"`, `"generate-api"`).
    pub actions: Vec<PlannedAction>,
}

/// A node in the proposed directory tree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectoryNode {
    pub name: String,
    pub kind: NodeKind,
    pub children: Vec<DirectoryNode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NodeKind {
    Dir,
    File,
}

impl DirectoryNode {
    pub fn dir(name: impl Into<String>, children: Vec<DirectoryNode>) -> Self {
        Self {
            name: name.into(),
            kind: NodeKind::Dir,
            children,
        }
    }

    pub fn file(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            kind: NodeKind::File,
            children: vec![],
        }
    }
}

/// A type, struct, class, or interface to be generated.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypeSpec {
    /// Name of the type as it will appear in source code.
    pub name: String,
    /// Kind of type construct.
    pub kind: TypeKind,
    /// File path where this type will be defined (repo-relative).
    pub file_path: String,
    /// Fields or members.
    pub fields: Vec<FieldSpec>,
    /// Optional doc comment.
    pub doc: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TypeKind {
    Struct,
    Enum,
    Interface,
    Class,
    Trait,
    Type, // type alias
}

/// A field/member within a `TypeSpec`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldSpec {
    pub name: String,
    pub type_name: String,
    pub optional: bool,
    pub doc: Option<String>,
}

/// A named variable, constant, or configuration value shared across files.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VariableSpec {
    pub name: String,
    pub type_name: String,
    pub description: String,
    pub scope: VariableScope,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VariableScope {
    /// Accessible from any module (e.g. env var name, global const).
    Global,
    /// Scoped to a specific module or file.
    Module { file_path: String },
}

/// A file that will be created or modified by an improvement action.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileSpec {
    /// Repo-relative path (e.g. `"Dockerfile"`, `"src/db/mod.rs"`).
    pub path: String,
    /// What role this file plays.
    pub purpose: FilePurpose,
    /// Other files in this spec that must exist before this one is generated.
    pub depends_on: Vec<String>,
    /// Which action ID is responsible for generating this file.
    pub owner_action: String,
    /// Optional starter content or template hints for the LLM.
    pub template_hint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilePurpose {
    Dockerfile,
    DockerCompose,
    DataAccessLayer,
    ApiEndpoint,
    ApiRouter,
    Model,
    Migration,
    Test,
    Configuration,
    CiConfig,
    Governance, // LICENSE, README, SECURITY.md, etc.
    Other,
}

/// A single planned improvement action with its dependency graph edges.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannedAction {
    /// Stable identifier matching a registered action (e.g. `"containerize"`).
    pub id: String,
    /// Human-readable description shown to the user.
    pub description: String,
    /// IDs of other planned actions that must complete first.
    pub depends_on: Vec<String>,
    /// Whether this action can be safely applied on its own branch.
    pub is_independent: bool,
}

// ── GenerateScaffoldSpecAction ────────────────────────────────────────────────

pub struct GenerateScaffoldSpecAction<C> {
    connector: Arc<C>,
}

impl<C: LlmConnector + 'static> GenerateScaffoldSpecAction<C> {
    pub fn new(connector: Arc<C>) -> Self {
        Self { connector }
    }
}

#[async_trait]
impl<C: LlmConnector + 'static> Action for GenerateScaffoldSpecAction<C> {
    type Input = ();
    type Output = ScaffoldSpec;

    fn name(&self) -> &str {
        "generate-scaffold-spec"
    }

    async fn execute(&self, ctx: &ExecutionContext, _input: ()) -> Result<ScaffoldSpec, CoreError> {
        // Gather all analysis context.
        let report = ctx.require::<FinalReport>(CTX_FINAL_REPORT).await?;
        let maturity = ctx.require::<MaturityScore>(CTX_MATURITY_SCORE).await?;
        let governance: GovernanceReport = ctx
            .get::<GovernanceReport>(CTX_GOVERNANCE_REPORT)
            .await
            .unwrap_or_default();
        let dockerfiles: DockerfileReport = ctx
            .get::<DockerfileReport>(CTX_DOCKERFILE_REPORT)
            .await
            .unwrap_or_default();
        let audit: AuditReport = ctx
            .get::<AuditReport>(CTX_AUDIT_REPORT)
            .await
            .unwrap_or_default();
        let deps: DependencyReport = ctx
            .get::<DependencyReport>(CTX_DEPENDENCY_REPORT)
            .await
            .unwrap_or_default();
        let languages: DetectedLanguages = ctx
            .get::<DetectedLanguages>(CTX_DETECTED_LANGUAGES)
            .await
            .unwrap_or_default();
        let intent: String = ctx
            .get::<String>(CTX_IMPROVEMENT_INTENT)
            .await
            .unwrap_or_default();

        let prompt = build_prompt(
            &report,
            &maturity,
            &governance,
            &dockerfiles,
            &audit,
            &deps,
            &languages.primary,
            &intent,
        );

        info!("Generating scaffold spec via LLM");

        let request = rusty_venture_llm::LlmRequestBuilder::new()
            .system(SYSTEM_PROMPT)
            .user(prompt)
            .max_tokens(8192)
            .build();

        let response = self
            .connector
            .complete(request)
            .await
            .map_err(|e| CoreError::other(format!("LLM scaffold spec failed: {e}")))?;

        let raw = response.text_or_empty().to_string();

        let spec: ScaffoldSpec = serde_json::from_str(&raw).map_err(|e| {
            CoreError::other(format!(
                "Failed to parse ScaffoldSpec JSON from LLM: {e}\nRaw: {}",
                &raw[..raw.len().min(500)]
            ))
        })?;

        info!(
            actions = spec.actions.len(),
            files = spec.files.len(),
            types = spec.types.len(),
            "Scaffold spec generated"
        );

        ctx.insert(CTX_SCAFFOLD_SPEC, spec.clone()).await;
        Ok(spec)
    }
}

// ── Prompt construction ───────────────────────────────────────────────────────

const SYSTEM_PROMPT: &str = r#"You are a senior software architect. Given a repository analysis report, \
a maturity score, and optionally a user-provided improvement intent, you must produce a ScaffoldSpec \
JSON object describing EXACTLY what files, types, and actions are needed to improve the repository.

Return ONLY a valid JSON object matching this schema — no markdown, no explanation, just the JSON.

ScaffoldSpec schema:
{
  "intent_summary": "string — one sentence describing the plan",
  "directory_tree": { "name": "string", "kind": "dir|file", "children": [...] },
  "types": [
    {
      "name": "string",
      "kind": "struct|enum|interface|class|trait|type",
      "file_path": "string",
      "fields": [{ "name": "string", "type_name": "string", "optional": bool, "doc": "string|null" }],
      "doc": "string|null"
    }
  ],
  "variables": [
    {
      "name": "string",
      "type_name": "string",
      "description": "string",
      "scope": { "global": {} } | { "module": { "file_path": "string" } }
    }
  ],
  "files": [
    {
      "path": "string",
      "purpose": "dockerfile|docker_compose|data_access_layer|api_endpoint|api_router|model|migration|test|configuration|ci_config|governance|other",
      "depends_on": ["string"],
      "owner_action": "string",
      "template_hint": "string|null"
    }
  ],
  "actions": [
    {
      "id": "string",
      "description": "string",
      "depends_on": ["string"],
      "is_independent": bool
    }
  ]
}"#;

#[allow(clippy::too_many_arguments)]
fn build_prompt(
    report: &FinalReport,
    maturity: &MaturityScore,
    governance: &GovernanceReport,
    dockerfiles: &DockerfileReport,
    audit: &AuditReport,
    deps: &DependencyReport,
    primary_language: &Language,
    intent: &str,
) -> String {
    let maturity_gaps: Vec<String> = maturity
        .dimensions
        .iter()
        .flat_map(|d| {
            d.signals
                .iter()
                .filter(|s| !s.passed)
                .map(|s| format!("[{}] {}", d.dimension.label(), s.description.clone()))
        })
        .collect();

    let intent_section = if intent.is_empty() {
        "No specific intent provided — address the highest-priority maturity gaps.".to_string()
    } else {
        format!("USER INTENT: {intent}")
    };

    format!(
        r#"## Repository Analysis Summary

Primary language: {lang}
Composite maturity: {composite}/100 ({grade})
Has Dockerfile: {has_docker}
Has docker-compose: {has_compose}
Has CI config: {has_ci}
License: {license}
Violations: {violations} ({critical} critical)
Lock file present: {lock_file}

## Maturity Gaps (signals that failed)
{gaps}

## LLM Report Summary
{summary}

## Improvement Intent
{intent_section}

Produce a ScaffoldSpec that addresses the intent and the most impactful gaps above.
Focus on actions that are automatable from a static code analysis context.
Available action IDs: "containerize", "generate-dal", "generate-api", "add-license",
"add-security-policy", "add-contributing", "add-changelog", "add-ci", "add-dependabot",
"add-lint-config", "add-pre-commit", "add-safety-config".
"#,
        lang = primary_language,
        composite = maturity.composite,
        grade = maturity.grade.label(),
        has_docker = dockerfiles.has_root_dockerfile,
        has_compose = dockerfiles.has_root_compose,
        has_ci = governance.has_ci_config,
        license = if governance.has_license {
            "present"
        } else {
            "MISSING (All Rights Reserved)"
        },
        violations = audit.violations.len(),
        critical = audit.critical_count,
        lock_file = deps.lock_file_present,
        gaps = maturity_gaps.join("\n"),
        summary = report.summary,
        intent_section = intent_section,
    )
}
