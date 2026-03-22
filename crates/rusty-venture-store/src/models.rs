use serde::{Deserialize, Serialize};

/// A tracked repository (one row per unique URL).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoRow {
    pub id: String,
    pub url: String,
    pub first_seen: String,
    pub last_scanned: Option<String>,
}

/// One completed analysis run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanRow {
    pub id: String,
    pub repo_id: String,
    pub scanned_at: String,
    pub duration_ms: i64,
    pub risk_score: i64,
    pub composite_maturity: i64,
    pub maturity_grade: String,
    /// Full `FinalReport` serialised as JSON.
    pub raw_report: String,
    /// Full `MaturityScore` serialised as JSON.
    pub raw_maturity: String,
}

/// Per-dimension maturity score for one scan (enables dimension-level trending).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DimensionRow {
    pub id: i64,
    pub scan_id: String,
    pub dimension: String,
    pub score: i64,
}

/// One committed-file violation from the audit step.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViolationRow {
    pub id: i64,
    pub scan_id: String,
    pub severity: String,
    pub file_path: String,
    pub recommendation: String,
}

// ── v2 grade model ────────────────────────────────────────────────────────────

/// A version of the scoring model (grade_models table).
#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize)]
pub struct GradeModelRow {
    /// UUID primary key.
    pub id: String,
    /// Semantic version string, e.g. `"2.0.0"`.
    pub version: String,
    /// Human-readable description of this model version.
    pub description: String,
    /// ISO 8601 timestamp of when this model was registered.
    pub created_at: String,
}

/// The computed v2 grade for one scan (scan_grades table).
#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize)]
pub struct ScanGradeRow {
    /// UUID primary key.
    pub id: String,
    /// FK → scans.id
    pub scan_id: String,
    /// FK → grade_models.id
    pub model_id: String,
    /// Weighted composite score, 0–100.
    pub composite: i64,
    /// League-tier label: BRONZE / SILVER / GOLD / PLATINUM / DIAMOND.
    pub grade: String,
    /// ISO 8601 timestamp.
    pub created_at: String,
}

/// Per-dimension score within a v2 scan grade (scan_dimension_scores_v2 table).
#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize)]
pub struct DimensionScoreV2Row {
    /// Auto-increment primary key.
    pub id: i64,
    /// FK → scan_grades.id
    pub scan_grade_id: String,
    /// Dimension label, e.g. `"Security"`.
    pub dimension: String,
    /// Normalised score 0–100.
    pub score: i64,
    /// Dimension weight used in the composite calculation.
    pub weight: f64,
}

/// One evaluated signal within a dimension score (scan_signals table).
#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize)]
pub struct ScanSignalRow {
    /// Auto-increment primary key.
    pub id: i64,
    /// FK → scan_dimension_scores_v2.id
    pub dimension_score_id: i64,
    /// Short identifier, e.g. `"no_critical_violations"`.
    pub name: String,
    /// Human-readable description of what was measured.
    pub description: String,
    /// Whether the signal passed for this scan.
    pub passed: bool,
    /// Maximum point value for this signal.
    pub points: i64,
    /// Optional recommendation when the signal fails.
    pub detail: Option<String>,
    /// Maturity tier this signal belongs to (1 = Static Discovery,
    /// 2 = Content Quality, 3 = Active Functional Validation).
    pub tier: i64,
}

/// One piece of evidence attached to a signal (signal_evidence table).
#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize)]
pub struct SignalEvidenceRow {
    /// Auto-increment primary key.
    pub id: i64,
    /// FK → scan_signals.id
    pub signal_id: i64,
    /// Evidence kind, e.g. `"file_found"`, `"count"`.
    pub kind: String,
    /// The evidence value (may be a JSON blob).
    pub value: String,
}

/// Serialised evaluation DAG for one scan (decision_graphs table).
#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize)]
pub struct DecisionGraphRow {
    /// UUID primary key.
    pub id: String,
    /// FK → scans.id
    pub scan_id: String,
    /// Legacy raw MaturityScore JSON (kept for backwards-compat).
    pub graph_json: String,
    /// ISO 8601 timestamp.
    pub created_at: String,
    /// Full `DecisionGraph` JSON pre-computed at scan time (migration 003+).
    /// When present, the server uses this directly instead of rebuilding.
    pub graph_payload: Option<String>,
}

// ── Overview / trend view models ──────────────────────────────────────────────

/// Summary row returned by `list_repos`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoSummary {
    pub id: String,
    pub url: String,
    pub first_seen: String,
    pub last_scanned: Option<String>,
    pub scan_count: i64,
    pub latest_maturity_grade: Option<String>,
    pub latest_composite_maturity: Option<i64>,
    pub latest_risk_score: Option<i64>,
    /// Highest tier that has been fully unlocked for this repo (1, 2, or 3).
    pub max_unlocked_tier: i64,
}

/// Per-dimension breakdown inside a `RepoOverview`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DimensionOverview {
    pub dimension: String,
    pub score: i64,
    pub weight: f64,
    pub passed_count: i64,
    pub total_count: i64,
}

/// One failed signal that blocked the score — returned as part of `RepoOverview`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockerItem {
    pub signal_name: String,
    pub dimension: String,
    pub points: i64,
    pub detail: Option<String>,
}

/// One signal (passed or failed) inside a `DimensionSignals` breakdown.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalItem {
    pub name: String,
    pub passed: bool,
    pub points: i64,
    pub detail: Option<String>,
    /// Maturity tier this signal belongs to.
    pub tier: i64,
}

/// All signals for one dimension, included in `RepoOverview`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DimensionSignals {
    pub dimension: String,
    pub score: i64,
    pub weight: f64,
    pub signals: Vec<SignalItem>,
}

/// LLM-generated narrative report, extracted from the stored `raw_report` blob.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmReport {
    pub summary: String,
    pub language_insights: Vec<String>,
    pub dependency_recommendations: Vec<String>,
    pub dockerfile_findings: Vec<String>,
    pub security_violations: Vec<String>,
    pub general_recommendations: Vec<String>,
}

/// Full overview payload for `GET /repos/:id/overview`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoOverview {
    pub repo_id: String,
    pub repo_url: String,
    /// Weighted composite score, 0–100.
    pub composite: i64,
    /// League-tier label.
    pub grade: String,
    /// Percentage of signals that passed (0.0–100.0).
    pub confidence: f64,
    pub dimensions: Vec<DimensionOverview>,
    /// Up to 5 failed signals with highest point values.
    pub top_blockers: Vec<BlockerItem>,
    /// All signals grouped by dimension (passed + failed).
    pub signals_by_dimension: Vec<DimensionSignals>,
    /// LLM-generated narrative, if available.
    pub llm_report: Option<LlmReport>,
    /// Which tier was executed for the most recent scan (1, 2, or 3).
    pub scan_tier: i64,
    /// Highest tier that has been fully unlocked and is ready to advance from.
    pub max_unlocked_tier: i64,
}

/// One data point in the trend series returned by `GET /repos/:id/trends`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrendPoint {
    pub scanned_at: String,
    pub composite: i64,
    pub grade: String,
    /// Score per dimension at this point in time.
    pub dimensions: std::collections::HashMap<String, i64>,
}
