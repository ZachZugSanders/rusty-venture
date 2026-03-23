pub mod db;
pub mod models;
pub mod queries;

#[cfg(test)]
mod tests;

pub use db::{open_pool, Pool};
pub use models::{
    BlockerItem, DecisionGraphRow, DimensionOverview, DimensionRow, DimensionScoreV2Row,
    DimensionSignals, GradeModelRow, LlmReport, RepoBranchRow, RepoOverview, RepoRow, RepoSummary,
    ScanGradeRow, ScanRow, ScanSignalRow, SignalEvidenceRow, SignalItem, TrendPoint, ViolationRow,
};
pub use queries::{
    advance_repo_tier, backfill_v2_grades, ensure_repo, get_decision_graph_for_scan,
    get_repo_overview, get_repo_tier, get_repo_tier_by_url, get_repo_trends, get_repo_url,
    get_scan, get_scan_grade_v2, insert_decision_graph, insert_dimension_score_v2, insert_scan,
    insert_scan_grade, insert_scan_signal, insert_signal_evidence, list_branches_for_repo,
    list_repos, list_scans, list_scans_for_repo, scan_exists_for_commit, upsert_branches,
    upsert_grade_model, ScanDetail,
};
