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
