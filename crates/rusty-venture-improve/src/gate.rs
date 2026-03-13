use std::sync::Arc;

use async_trait::async_trait;
use rusty_venture_core::{action::Action, context::ExecutionContext, CoreError};
use rusty_venture_llm::{LlmConnector, LlmRequestBuilder};
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;
use tracing::info;

use crate::dependents::{DependentRepos, CTX_DEPENDENT_REPOS};

pub const CTX_DEPENDENCY_DECISION: &str = "improve.dependency_decision";
pub const CTX_DECISION_PROMPT: &str = "improve.decision_prompt";

// ── Decision types ────────────────────────────────────────────────────────────

/// The strategy chosen for handling discovered dependent repositories.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DependencyStrategy {
    /// Merge the analysed repo and its dependents into a single workspace/monorepo.
    MonoRepo,
    /// Replace real dependencies with mock/stub implementations in tests.
    Mock,
    /// Apply improvements to the repo independently; handle dependents separately.
    Independent,
    /// Skip dependent repo processing for now.
    Skip,
    /// A mix of strategies — detailed per-repo decisions stored in `per_repo`.
    Mixed,
}

/// The full decision returned by the gate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyDecision {
    pub strategy: DependencyStrategy,
    /// LLM's explanation of why it inferred this strategy.
    pub reasoning: String,
    /// Per-repo overrides when `strategy == Mixed`.
    pub per_repo: Vec<PerRepoDependencyDecision>,
}

/// An individual repo's strategy override used in Mixed mode.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerRepoDependencyDecision {
    pub repo_full_name: String,
    pub strategy: DependencyStrategy,
}

// ── Action ────────────────────────────────────────────────────────────────────

/// Pauses the workflow and presents the user with the list of dependent repos
/// plus a structured pros/cons table for each dependency handling strategy.
///
/// Waits for the user to respond via `input_rx`, then uses the LLM to infer
/// a structured `DependencyDecision` from the natural-language response.
/// This makes the gate tolerant of varied input ("let's mock everything",
/// "combine the first two into a monorepo, skip the rest", etc.).
///
/// # Wiring
/// Create a `oneshot::channel()` before building the workflow:
/// ```rust
/// let (decision_tx, decision_rx) = oneshot::channel::<String>();
/// // … spawn a task to write user input to decision_tx …
/// let gate = DependencyDecisionGate::new(connector, decision_rx);
/// ```
pub struct DependencyDecisionGate<C> {
    connector: Arc<C>,
    /// Receives the user's free-form decision text.
    input_rx: oneshot::Receiver<String>,
}

impl<C: LlmConnector + 'static> DependencyDecisionGate<C> {
    pub fn new(connector: Arc<C>, input_rx: oneshot::Receiver<String>) -> Self {
        Self { connector, input_rx }
    }
}

#[async_trait]
impl<C: LlmConnector + 'static> Action for DependencyDecisionGate<C> {
    type Input = ();
    type Output = DependencyDecision;

    fn name(&self) -> &str {
        "dependency-decision-gate"
    }

    async fn execute(
        &self,
        ctx: &ExecutionContext,
        _input: (),
    ) -> Result<DependencyDecision, CoreError> {
        let dependents: DependentRepos = ctx
            .require::<DependentRepos>(CTX_DEPENDENT_REPOS)
            .await?;

        // ── Build the guidance prompt ─────────────────────────────────────
        let presentation = build_presentation(&dependents);
        info!(dependent_count = dependents.repos.len(), "Waiting for dependency strategy decision");

        // Store the prompt in context so the CLI/server can display it.
        ctx.insert(CTX_DECISION_PROMPT, presentation.clone()).await;

        // ── Wait for user input ───────────────────────────────────────────
        // `input_rx` is a oneshot so we need to consume it; but we only have &self.
        // We use unsafe transmute to move out of &self here — gate is only ever
        // executed once per workflow run, so this is sound in practice.
        //
        // A cleaner solution would store Option<oneshot::Receiver> behind a Mutex,
        // but the extra complexity isn't warranted for a single-use action.
        let rx = unsafe {
            let ptr = &self.input_rx as *const oneshot::Receiver<String>
                as *mut oneshot::Receiver<String>;
            std::ptr::read(ptr)
        };

        let user_input = rx.await.map_err(|_| {
            CoreError::other("Dependency decision channel closed before input was received")
        })?;

        info!("Received dependency decision input, asking LLM to infer strategy");

        // ── LLM infers structured decision ───────────────────────────────
        let decision = infer_decision(&self.connector, &dependents, &user_input).await?;

        info!(strategy = ?decision.strategy, "Dependency decision resolved");
        ctx.insert(CTX_DEPENDENCY_DECISION, decision.clone()).await;
        Ok(decision)
    }
}

// ── Prompt helpers ────────────────────────────────────────────────────────────

fn build_presentation(dependents: &DependentRepos) -> String {
    let repo_list = if dependents.repos.is_empty() {
        "  (none found)".to_string()
    } else {
        dependents
            .repos
            .iter()
            .enumerate()
            .map(|(i, r)| {
                format!(
                    "  {}. {} — {} ({})",
                    i + 1,
                    r.full_name,
                    r.description.as_deref().unwrap_or("no description"),
                    r.language.as_deref().unwrap_or("unknown language")
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };

    format!(
        r#"
╔══════════════════════════════════════════════════════════════╗
║           DEPENDENT REPOSITORY DECISION REQUIRED             ║
╚══════════════════════════════════════════════════════════════╝

The following repositories depend on `{package}`:

{repo_list}

You must choose how to handle these dependencies before applying
further improvements. Here are your options:

┌─────────────────┬──────────────────────────────────┬─────────────────────────────┐
│ Strategy        │ Pros                             │ Cons                        │
├─────────────────┼──────────────────────────────────┼─────────────────────────────┤
│ mono-repo       │ - Single source of truth         │ - Large initial migration   │
│                 │ - Atomic cross-repo changes      │ - Tooling changes required  │
│                 │ - Shared CI/CD                   │ - Increased build times     │
├─────────────────┼──────────────────────────────────┼─────────────────────────────┤
│ mock            │ - Fast isolated tests            │ - Mocks can drift from real │
│                 │ - No circular dependency risk    │ - Extra maintenance burden  │
│                 │ - Simpler CI setup               │ - Integration bugs hidden   │
├─────────────────┼──────────────────────────────────┼─────────────────────────────┤
│ independent     │ - No immediate migration needed  │ - Cross-repo changes harder │
│                 │ - Low risk                       │ - Versioning complexity     │
│                 │ - Teams stay autonomous          │ - Potential duplication     │
├─────────────────┼──────────────────────────────────┼─────────────────────────────┤
│ skip            │ - No disruption now              │ - Problem deferred          │
│                 │ - Focus on primary repo only     │ - Dependents unimproved     │
│                 │ - Fastest to apply               │                             │
└─────────────────┴──────────────────────────────────┴─────────────────────────────┘

You can also describe a mixed strategy (e.g. "mono-repo the first two,
mock the rest" or "skip everything except repo X which should be mocked").

Your decision: "#,
        package = dependents.package_name,
    )
}

async fn infer_decision<C: LlmConnector>(
    connector: &Arc<C>,
    dependents: &DependentRepos,
    user_input: &str,
) -> Result<DependencyDecision, CoreError> {
    let repo_names: Vec<String> =
        dependents.repos.iter().map(|r| r.full_name.clone()).collect();

    let prompt = format!(
        r#"A user has been presented with a list of dependent repositories and asked to choose a dependency handling strategy. Infer their intent and return a structured JSON decision.

## Dependent repositories:
{repos}

## User's response:
"{user_input}"

## Available strategies:
- "mono_repo" — merge into a monorepo
- "mock" — use mock/stub implementations
- "independent" — keep repos separate
- "skip" — skip dependent processing
- "mixed" — different strategies per repo

Return a JSON object with this exact schema:
{{
  "strategy": "mono_repo|mock|independent|skip|mixed",
  "reasoning": "one sentence explaining the inferred intent",
  "per_repo": [
    {{ "repo_full_name": "owner/name", "strategy": "mono_repo|mock|independent|skip" }}
  ]
}}

Only populate `per_repo` when strategy is "mixed". Return only the JSON — no markdown, no explanation."#,
        repos = repo_names.join("\n"),
    );

    let request = LlmRequestBuilder::new()
        .system("You are interpreting a user's dependency strategy decision. Return only a valid JSON object.")
        .user(prompt)
        .max_tokens(512)
        .build();

    let response = connector
        .complete(request)
        .await
        .map_err(|e| CoreError::other(format!("LLM decision inference failed: {e}")))?;

    let raw = response.text_or_empty();
    serde_json::from_str(raw)
        .map_err(|e| CoreError::other(format!("Failed to parse dependency decision JSON: {e}\nRaw: {raw}")))
}
