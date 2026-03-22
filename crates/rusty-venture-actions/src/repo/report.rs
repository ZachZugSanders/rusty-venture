use std::sync::Arc;

use async_trait::async_trait;
use rusty_venture_core::{action::Action, context::ExecutionContext, CoreError};
use rusty_venture_llm::{LlmConnector, LlmRequestBuilder};
use serde::{Deserialize, Serialize};
use tracing::info;

use super::analyze_deps::CTX_DEPENDENCY_REPORT;
use super::audit_files::CTX_AUDIT_REPORT;
use super::clone::CTX_REPO_URL;
use super::detect_language::CTX_DETECTED_LANGUAGES;
use super::find_dockerfiles::CTX_DOCKERFILE_REPORT;

pub const CTX_FINAL_REPORT: &str = "repo.final_report";

/// The structured final report returned by the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FinalReport {
    pub summary: String,
    /// Risk score from 0 (no issues) to 100 (critical problems).
    pub risk_score: u8,
    pub language_insights: Vec<String>,
    pub dependency_recommendations: Vec<String>,
    pub dockerfile_findings: Vec<String>,
    pub security_violations: Vec<String>,
    pub general_recommendations: Vec<String>,
}

const REPORT_SYSTEM_PROMPT: &str = r#"
You are a software security and quality analyst. You will receive a JSON object
containing the automated analysis results for a git repository. Produce a
structured report as valid JSON with exactly this schema:

{
  "summary": "2-3 sentence executive summary of the repository's health",
  "risk_score": <integer 0-100, where 100 is most critical>,
  "language_insights": ["string observations about the detected language/runtime"],
  "dependency_recommendations": ["string recommendations about dependencies"],
  "dockerfile_findings": ["string findings about Docker configuration"],
  "security_violations": ["string descriptions of security violations found"],
  "general_recommendations": ["string actionable improvement recommendations"]
}

Rules:
- Respond ONLY with valid JSON. No markdown fences, no extra text.
- Be specific and actionable in recommendations.
- Prioritize security issues in the risk score.
- If a section has no findings, return an empty array [].
"#;

/// Aggregates all analysis results and sends them to the LLM to generate
/// a human-readable report with actionable recommendations.
pub struct GenerateReportAction<C: LlmConnector> {
    connector: Arc<C>,
}

impl<C: LlmConnector> GenerateReportAction<C> {
    pub fn new(connector: Arc<C>) -> Self {
        Self { connector }
    }
}

#[async_trait]
impl<C: LlmConnector + 'static> Action for GenerateReportAction<C> {
    type Input = ();
    type Output = FinalReport;

    fn name(&self) -> &str {
        "generate-report"
    }

    async fn execute(&self, ctx: &ExecutionContext, _input: ()) -> Result<FinalReport, CoreError> {
        // Gather all analysis data from context
        let repo_url = ctx.get::<String>(CTX_REPO_URL).await.unwrap_or_default();

        let detected = ctx
            .get::<super::detect_language::DetectedLanguages>(CTX_DETECTED_LANGUAGES)
            .await;
        let dep_report = ctx
            .get::<super::analyze_deps::DependencyReport>(CTX_DEPENDENCY_REPORT)
            .await;
        let dockerfile_report = ctx
            .get::<super::find_dockerfiles::DockerfileReport>(CTX_DOCKERFILE_REPORT)
            .await;
        let audit_report = ctx
            .get::<super::audit_files::AuditReport>(CTX_AUDIT_REPORT)
            .await;

        // Build the analysis payload for the LLM
        let payload = serde_json::json!({
            "repo_url": repo_url,
            "detected_languages": detected,
            "dependency_report": dep_report,
            "dockerfile_report": dockerfile_report,
            "audit_report": audit_report,
        });

        info!("Sending analysis to LLM for report generation");

        let request = LlmRequestBuilder::new()
            .system(REPORT_SYSTEM_PROMPT)
            .user(serde_json::to_string_pretty(&payload).map_err(CoreError::Serde)?)
            .max_tokens(2048)
            .temperature(0.1)
            .build();

        let response = self
            .connector
            .complete(request)
            .await
            .map_err(|e| CoreError::Llm(e.to_string()))?;

        let text = response.text().unwrap_or("{}");

        let report: FinalReport = serde_json::from_str(text).map_err(|e| {
            CoreError::Llm(format!(
                "Failed to parse LLM response as FinalReport: {e}\nRaw response: {text}"
            ))
        })?;

        info!(
            risk_score = report.risk_score,
            "Report generated successfully"
        );

        ctx.insert(CTX_FINAL_REPORT, report.clone()).await;
        Ok(report)
    }
}
