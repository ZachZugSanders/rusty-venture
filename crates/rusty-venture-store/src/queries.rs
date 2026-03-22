use anyhow::{Context, Result};
use chrono::Utc;
use sqlx::Row;
use uuid::Uuid;

use rusty_venture_actions::{
    graph::{DecisionGraph, NodeSizeConfig},
    repo::{audit_files::AuditReport, maturity::MaturityScore, RepoAnalysisResult},
};

use crate::db::Pool;

/// Persist a completed analysis run to the database in a single transaction.
///
/// Returns the authoritative `repo_id` (UUID) for the upserted repo row so
/// the caller can perform post-scan operations such as tier advancement.
pub async fn insert_scan(
    pool: &Pool,
    result: &RepoAnalysisResult,
    maturity: &MaturityScore,
    audit: &AuditReport,
    scan_tier: u8,
) -> Result<String> {
    let repo_id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    let raw_report = serde_json::to_string(&result.report).context("serialise FinalReport")?;
    let raw_maturity = serde_json::to_string(maturity).context("serialise MaturityScore")?;

    let mut tx = pool.begin().await.context("begin transaction")?;

    // Upsert repo
    sqlx::query(
        r#"INSERT INTO repos (id, url, first_seen, last_scanned)
           VALUES (?1, ?2, ?3, ?3)
           ON CONFLICT(url) DO UPDATE SET last_scanned = excluded.last_scanned"#,
    )
    .bind(&repo_id)
    .bind(&result.repo_url)
    .bind(&now)
    .execute(&mut *tx)
    .await
    .context("upsert repo")?;

    // Fetch the authoritative repo id (may differ if row already existed)
    let actual_repo_id: String = sqlx::query("SELECT id FROM repos WHERE url = ?1")
        .bind(&result.repo_url)
        .fetch_one(&mut *tx)
        .await
        .context("fetch repo id")?
        .get(0);

    // Insert scan
    let scan_id = &result.run_id;
    let duration = result.duration_ms as i64;
    let risk = result.report.risk_score as i64;
    let composite = maturity.composite as i64;
    let grade = maturity.grade.label().to_string();

    sqlx::query(
        r#"INSERT INTO scans
               (id, repo_id, scanned_at, duration_ms, risk_score,
                composite_maturity, maturity_grade, raw_report, raw_maturity, commit_hash, scan_tier)
           VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)"#,
    )
    .bind(scan_id)
    .bind(&actual_repo_id)
    .bind(&now)
    .bind(duration)
    .bind(risk)
    .bind(composite)
    .bind(&grade)
    .bind(&raw_report)
    .bind(&raw_maturity)
    .bind(&result.commit_hash)
    .bind(scan_tier as i64)
    .execute(&mut *tx)
    .await
    .context("insert scan")?;

    // Insert per-dimension scores
    for dim in &maturity.dimensions {
        sqlx::query(
            "INSERT INTO maturity_dimensions (scan_id, dimension, score) VALUES (?1, ?2, ?3)",
        )
        .bind(scan_id)
        .bind(dim.dimension.label())
        .bind(dim.score as i64)
        .execute(&mut *tx)
        .await
        .context("insert dimension")?;
    }

    // Insert violations
    for v in &audit.violations {
        sqlx::query(
            "INSERT INTO violations (scan_id, severity, file_path, recommendation) VALUES (?1, ?2, ?3, ?4)",
        )
        .bind(scan_id)
        .bind(v.severity.to_string())
        .bind(&v.file)
        .bind(&v.recommendation)
        .execute(&mut *tx)
        .await
        .context("insert violation")?;
    }

    // ── v2 grade persistence ─────────────────────────────────────────────────
    // Uses a well-known fixed ID for the "v2.0.0" grade model so the seed is
    // effectively a no-op on repeated runs (INSERT OR IGNORE).
    const V2_MODEL_ID: &str = "model-v2.0.0";
    sqlx::query(
        "INSERT OR IGNORE INTO grade_models (id, version, description, created_at) \
         VALUES (?1, ?2, ?3, ?4)",
    )
    .bind(V2_MODEL_ID)
    .bind("2.0.0")
    .bind("Default v2 maturity grade model")
    .bind(&now)
    .execute(&mut *tx)
    .await
    .context("upsert grade_model in insert_scan")?;

    let grade_id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO scan_grades \
         (id, scan_id, model_id, composite, grade, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )
    .bind(&grade_id)
    .bind(scan_id)
    .bind(V2_MODEL_ID)
    .bind(composite)
    .bind(&grade)
    .bind(&now)
    .execute(&mut *tx)
    .await
    .context("insert scan_grade in insert_scan")?;

    for dim in &maturity.dimensions {
        let dim_result = sqlx::query(
            "INSERT INTO scan_dimension_scores_v2 \
             (scan_grade_id, dimension, score, weight) VALUES (?1, ?2, ?3, ?4)",
        )
        .bind(&grade_id)
        .bind(dim.dimension.label())
        .bind(dim.score as i64)
        .bind(dim.dimension.weight() as f64)
        .execute(&mut *tx)
        .await
        .context("insert dimension_score_v2 in insert_scan")?;

        let dim_id = dim_result.last_insert_rowid();

        for signal in &dim.signals {
            sqlx::query(
                "INSERT INTO scan_signals \
                 (dimension_score_id, name, description, passed, points, detail, tier) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )
            .bind(dim_id)
            .bind(&signal.name)
            .bind(&signal.description)
            .bind(signal.passed as i64)
            .bind(signal.points as i64)
            .bind(&signal.detail)
            .bind(signal.tier as i64)
            .execute(&mut *tx)
            .await
            .context("insert scan_signal in insert_scan")?;
        }
    }

    // Store the full MaturityScore JSON AND the pre-computed DecisionGraph.
    let graph_id = Uuid::new_v4().to_string();
    let graph_payload = serde_json::to_string(&DecisionGraph::from_maturity_full(
        maturity,
        Some(result.report.risk_score),
        NodeSizeConfig::default(),
    ))
    .context("serialise DecisionGraph payload")?;
    sqlx::query(
        "INSERT INTO decision_graphs (id, scan_id, graph_json, created_at, graph_payload) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
    )
    .bind(&graph_id)
    .bind(scan_id)
    .bind(&raw_maturity)
    .bind(&now)
    .bind(&graph_payload)
    .execute(&mut *tx)
    .await
    .context("insert decision_graph in insert_scan")?;

    tx.commit().await.context("commit transaction")?;

    tracing::info!(
        repo = %result.repo_url,
        scan_id = %scan_id,
        composite_maturity = maturity.composite,
        grade = %grade,
        scan_tier = scan_tier,
        "Scan persisted to database"
    );

    Ok(actual_repo_id)
}

/// Lightweight scan summary for display in the history table.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ScanSummary {
    pub id: String,
    pub repo_url: String,
    pub scanned_at: String,
    pub duration_ms: i64,
    pub risk_score: u8,
    pub composite_maturity: u8,
    pub maturity_grade: String,
}

/// Return the URL of a repo by its ID, or `None` if not found.
pub async fn get_repo_url(pool: &Pool, repo_id: &str) -> Result<Option<String>> {
    let row = sqlx::query("SELECT url FROM repos WHERE id = ?1")
        .bind(repo_id)
        .fetch_optional(pool)
        .await
        .context("get_repo_url")?;
    Ok(row.map(|r| r.get("url")))
}

/// Check whether any scan for the given repo already used `commit_hash`.
/// Returns the scan id if found, `None` if this commit has not been scanned.
pub async fn scan_exists_for_commit(
    pool: &Pool,
    repo_id: &str,
    commit_hash: &str,
) -> Result<Option<String>> {
    let row = sqlx::query("SELECT id FROM scans WHERE repo_id = ?1 AND commit_hash = ?2 LIMIT 1")
        .bind(repo_id)
        .bind(commit_hash)
        .fetch_optional(pool)
        .await
        .context("scan_exists_for_commit")?;
    Ok(row.map(|r| r.get("id")))
}

/// Return the N most recent scans across all repos.
pub async fn list_scans(pool: &Pool, limit: i64) -> Result<Vec<ScanSummary>> {
    let rows = sqlx::query(
        r#"SELECT s.id, r.url, s.scanned_at, s.duration_ms,
                  s.risk_score, s.composite_maturity, s.maturity_grade
           FROM scans s
           JOIN repos r ON r.id = s.repo_id
           ORDER BY s.scanned_at DESC
           LIMIT ?1"#,
    )
    .bind(limit)
    .fetch_all(pool)
    .await
    .context("list scans")?;

    Ok(rows
        .into_iter()
        .map(|r| ScanSummary {
            id: r.get::<String, _>("id"),
            repo_url: r.get::<String, _>("url"),
            scanned_at: r.get::<String, _>("scanned_at"),
            duration_ms: r.get::<i64, _>("duration_ms"),
            risk_score: r.get::<i64, _>("risk_score") as u8,
            composite_maturity: r.get::<i64, _>("composite_maturity") as u8,
            maturity_grade: r.get::<String, _>("maturity_grade"),
        })
        .collect())
}

// ── v2 query API ─────────────────────────────────────────────────────────────

/// Insert or ignore a grade model row (idempotent by primary key).
pub async fn upsert_grade_model(
    pool: &Pool,
    id: &str,
    version: &str,
    description: &str,
    created_at: &str,
) -> Result<()> {
    sqlx::query(
        "INSERT OR IGNORE INTO grade_models (id, version, description, created_at) \
         VALUES (?1, ?2, ?3, ?4)",
    )
    .bind(id)
    .bind(version)
    .bind(description)
    .bind(created_at)
    .execute(pool)
    .await
    .context("upsert grade_model")?;
    Ok(())
}

/// Insert a v2 scan grade record.
pub async fn insert_scan_grade(
    pool: &Pool,
    id: &str,
    scan_id: &str,
    model_id: &str,
    composite: i64,
    grade: &str,
    created_at: &str,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO scan_grades \
         (id, scan_id, model_id, composite, grade, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )
    .bind(id)
    .bind(scan_id)
    .bind(model_id)
    .bind(composite)
    .bind(grade)
    .bind(created_at)
    .execute(pool)
    .await
    .context("insert scan_grade")?;
    Ok(())
}

/// Retrieve the v2 grade for a scan (returns `None` if not yet recorded).
pub async fn get_scan_grade_v2(
    pool: &Pool,
    scan_id: &str,
) -> Result<Option<crate::models::ScanGradeRow>> {
    let row = sqlx::query_as::<_, crate::models::ScanGradeRow>(
        "SELECT id, scan_id, model_id, composite, grade, created_at \
         FROM scan_grades WHERE scan_id = ?",
    )
    .bind(scan_id)
    .fetch_optional(pool)
    .await
    .context("get_scan_grade_v2")?;
    Ok(row)
}

/// Insert a v2 dimension score row and return its auto-increment id.
pub async fn insert_dimension_score_v2(
    pool: &Pool,
    scan_grade_id: &str,
    dimension: &str,
    score: i64,
    weight: f64,
) -> Result<i64> {
    let result = sqlx::query(
        "INSERT INTO scan_dimension_scores_v2 \
         (scan_grade_id, dimension, score, weight) VALUES (?1, ?2, ?3, ?4)",
    )
    .bind(scan_grade_id)
    .bind(dimension)
    .bind(score)
    .bind(weight)
    .execute(pool)
    .await
    .context("insert dimension_score_v2")?;
    Ok(result.last_insert_rowid())
}

/// Insert a signal row and return its auto-increment id.
#[allow(clippy::too_many_arguments)]
pub async fn insert_scan_signal(
    pool: &Pool,
    dimension_score_id: i64,
    name: &str,
    description: &str,
    passed: bool,
    points: i64,
    detail: Option<&str>,
    tier: i64,
) -> Result<i64> {
    let result = sqlx::query(
        "INSERT INTO scan_signals \
         (dimension_score_id, name, description, passed, points, detail, tier) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
    )
    .bind(dimension_score_id)
    .bind(name)
    .bind(description)
    .bind(passed as i64)
    .bind(points)
    .bind(detail)
    .bind(tier)
    .execute(pool)
    .await
    .context("insert scan_signal")?;
    Ok(result.last_insert_rowid())
}

/// Attach an evidence record to a signal.
pub async fn insert_signal_evidence(
    pool: &Pool,
    signal_id: i64,
    kind: &str,
    value: &str,
) -> Result<()> {
    sqlx::query("INSERT INTO signal_evidence (signal_id, kind, value) VALUES (?1, ?2, ?3)")
        .bind(signal_id)
        .bind(kind)
        .bind(value)
        .execute(pool)
        .await
        .context("insert signal_evidence")?;
    Ok(())
}

/// Store the decision-graph artifact for a scan.
///
/// `graph_json` is the legacy raw `MaturityScore` JSON for backwards-compat.
/// `graph_payload` is the pre-computed `DecisionGraph` JSON introduced in
/// migration 003; pass `None` when inserting from old code paths.
pub async fn insert_decision_graph(
    pool: &Pool,
    id: &str,
    scan_id: &str,
    graph_json: &str,
    created_at: &str,
    graph_payload: Option<&str>,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO decision_graphs (id, scan_id, graph_json, created_at, graph_payload) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
    )
    .bind(id)
    .bind(scan_id)
    .bind(graph_json)
    .bind(created_at)
    .bind(graph_payload)
    .execute(pool)
    .await
    .context("insert decision_graph")?;
    Ok(())
}

/// Retrieve the decision graph for a scan (returns `None` if absent).
pub async fn get_decision_graph_for_scan(
    pool: &Pool,
    scan_id: &str,
) -> Result<Option<crate::models::DecisionGraphRow>> {
    let row = sqlx::query_as::<_, crate::models::DecisionGraphRow>(
        "SELECT id, scan_id, graph_json, created_at, graph_payload \
         FROM decision_graphs WHERE scan_id = ?",
    )
    .bind(scan_id)
    .fetch_optional(pool)
    .await
    .context("get_decision_graph_for_scan")?;
    Ok(row)
}

// ── Backfill ─────────────────────────────────────────────────────────────────

/// Backfill v2 grade rows for every scan that pre-dates the v2 schema.
///
/// For each `scans` row that has no corresponding `scan_grades` row, this
/// function deserialises the stored `raw_maturity` JSON blob and inserts the
/// full v2 chain: `grade_models` seed → `scan_grades` → per-dimension
/// `scan_dimension_scores_v2` → per-signal `scan_signals`.
///
/// The function is **idempotent**: it only processes scans that have no v2
/// grade yet, so running it multiple times is safe.
///
/// Returns the number of scans that were backfilled.
pub async fn backfill_v2_grades(pool: &Pool) -> Result<usize> {
    use rusty_venture_actions::repo::maturity::MaturityScore;

    // Find all scan IDs that have no scan_grades row yet.
    let pending: Vec<(String, String)> = sqlx::query_as(
        "SELECT s.id, s.raw_maturity FROM scans s \
         WHERE NOT EXISTS (SELECT 1 FROM scan_grades g WHERE g.scan_id = s.id)",
    )
    .fetch_all(pool)
    .await
    .context("backfill: fetch pending scans")?;

    if pending.is_empty() {
        return Ok(0);
    }

    // Ensure the v2 model seed row exists.
    const V2_MODEL_ID: &str = "model-v2.0.0";
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT OR IGNORE INTO grade_models (id, version, description, created_at) \
         VALUES (?1, '2.0.0', 'Default v2 maturity grade model', ?2)",
    )
    .bind(V2_MODEL_ID)
    .bind(&now)
    .execute(pool)
    .await
    .context("backfill: upsert grade_model")?;

    let mut count = 0usize;

    for (scan_id, raw_maturity) in &pending {
        let maturity: MaturityScore = match serde_json::from_str(raw_maturity) {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!(scan_id = %scan_id, err = %e, "backfill: skip — cannot parse raw_maturity");
                continue;
            }
        };

        let mut tx = pool.begin().await.context("backfill: begin tx")?;

        let grade_id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO scan_grades \
             (id, scan_id, model_id, composite, grade, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )
        .bind(&grade_id)
        .bind(scan_id)
        .bind(V2_MODEL_ID)
        .bind(maturity.composite as i64)
        .bind(maturity.grade.label())
        .bind(&now)
        .execute(&mut *tx)
        .await
        .context("backfill: insert scan_grade")?;

        for dim in &maturity.dimensions {
            let res = sqlx::query(
                "INSERT INTO scan_dimension_scores_v2 \
                 (scan_grade_id, dimension, score, weight) VALUES (?1, ?2, ?3, ?4)",
            )
            .bind(&grade_id)
            .bind(dim.dimension.label())
            .bind(dim.score as i64)
            .bind(dim.dimension.weight() as f64)
            .execute(&mut *tx)
            .await
            .context("backfill: insert dimension_score_v2")?;

            let dim_id = res.last_insert_rowid();

            for signal in &dim.signals {
                sqlx::query(
                    "INSERT INTO scan_signals \
                     (dimension_score_id, name, description, passed, points, detail, tier) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                )
                .bind(dim_id)
                .bind(&signal.name)
                .bind(&signal.description)
                .bind(signal.passed as i64)
                .bind(signal.points as i64)
                .bind(&signal.detail)
                .bind(signal.tier as i64)
                .execute(&mut *tx)
                .await
                .context("backfill: insert scan_signal")?;
            }
        }

        tx.commit().await.context("backfill: commit")?;
        count += 1;
    }

    tracing::info!(backfilled = count, "v2 grade backfill complete");
    Ok(count)
}

// ── Scan detail query ─────────────────────────────────────────────────────────

/// Full scan detail including raw JSON fields for a single scan id.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ScanDetail {
    pub id: String,
    pub repo_url: String,
    pub scanned_at: String,
    pub duration_ms: i64,
    pub risk_score: u8,
    pub composite_maturity: u8,
    pub maturity_grade: String,
    /// Parsed `FinalReport` JSON value.
    pub report: serde_json::Value,
    /// Parsed `MaturityScore` JSON value.
    pub maturity: serde_json::Value,
}

/// Fetch a single scan by id, with full report payload.
pub async fn get_scan(pool: &Pool, scan_id: &str) -> Result<Option<ScanDetail>> {
    let row = sqlx::query(
        r#"SELECT s.id, r.url, s.scanned_at, s.duration_ms,
                  s.risk_score, s.composite_maturity, s.maturity_grade,
                  s.raw_report, s.raw_maturity
           FROM scans s
           JOIN repos r ON r.id = s.repo_id
           WHERE s.id = ?1"#,
    )
    .bind(scan_id)
    .fetch_optional(pool)
    .await
    .context("get scan")?;

    let Some(r) = row else { return Ok(None) };

    let report: serde_json::Value =
        serde_json::from_str(&r.get::<String, _>("raw_report")).unwrap_or(serde_json::Value::Null);
    let maturity: serde_json::Value = serde_json::from_str(&r.get::<String, _>("raw_maturity"))
        .unwrap_or(serde_json::Value::Null);

    Ok(Some(ScanDetail {
        id: r.get::<String, _>("id"),
        repo_url: r.get::<String, _>("url"),
        scanned_at: r.get::<String, _>("scanned_at"),
        duration_ms: r.get::<i64, _>("duration_ms"),
        risk_score: r.get::<i64, _>("risk_score") as u8,
        composite_maturity: r.get::<i64, _>("composite_maturity") as u8,
        maturity_grade: r.get::<String, _>("maturity_grade"),
        report,
        maturity,
    }))
}

// ── v1 list helpers ───────────────────────────────────────────────────────────

/// Return the N most recent scans for one specific repo URL.
pub async fn list_scans_for_repo(
    pool: &Pool,
    repo_url: &str,
    limit: i64,
) -> Result<Vec<ScanSummary>> {
    let rows = sqlx::query(
        r#"SELECT s.id, r.url, s.scanned_at, s.duration_ms,
                  s.risk_score, s.composite_maturity, s.maturity_grade
           FROM scans s
           JOIN repos r ON r.id = s.repo_id
           WHERE r.url = ?1
           ORDER BY s.scanned_at DESC
           LIMIT ?2"#,
    )
    .bind(repo_url)
    .bind(limit)
    .fetch_all(pool)
    .await
    .context("list scans for repo")?;

    Ok(rows
        .into_iter()
        .map(|r| ScanSummary {
            id: r.get::<String, _>("id"),
            repo_url: r.get::<String, _>("url"),
            scanned_at: r.get::<String, _>("scanned_at"),
            duration_ms: r.get::<i64, _>("duration_ms"),
            risk_score: r.get::<i64, _>("risk_score") as u8,
            composite_maturity: r.get::<i64, _>("composite_maturity") as u8,
            maturity_grade: r.get::<String, _>("maturity_grade"),
        })
        .collect())
}

// ── Overview / trend queries ──────────────────────────────────────────────────

/// Return a summary of every tracked repository, ordered by `last_scanned DESC`.
///
/// Each row includes the total scan count and, if at least one v2 grade exists,
/// the grade/composite/risk from the latest scan.
pub async fn list_repos(pool: &Pool, limit: i64) -> Result<Vec<crate::models::RepoSummary>> {
    let rows = sqlx::query(
        r#"SELECT
               r.id,
               r.url,
               r.first_seen,
               r.last_scanned,
               r.max_unlocked_tier,
               COUNT(s.id)                                                       AS scan_count,
               (SELECT sg.grade        FROM scan_grades sg
                JOIN scans ss ON ss.id = sg.scan_id
                WHERE ss.repo_id = r.id ORDER BY ss.scanned_at DESC LIMIT 1)    AS latest_maturity_grade,
               (SELECT sg.composite    FROM scan_grades sg
                JOIN scans ss ON ss.id = sg.scan_id
                WHERE ss.repo_id = r.id ORDER BY ss.scanned_at DESC LIMIT 1)    AS latest_composite_maturity,
               (SELECT ss2.risk_score  FROM scans ss2
                WHERE ss2.repo_id = r.id ORDER BY ss2.scanned_at DESC LIMIT 1)  AS latest_risk_score
           FROM repos r
           LEFT JOIN scans s ON s.repo_id = r.id
           GROUP BY r.id
           ORDER BY r.last_scanned DESC
           LIMIT ?1"#,
    )
    .bind(limit)
    .fetch_all(pool)
    .await
    .context("list_repos")?;

    Ok(rows
        .into_iter()
        .map(|r| crate::models::RepoSummary {
            id: r.get::<String, _>("id"),
            url: r.get::<String, _>("url"),
            first_seen: r.get::<String, _>("first_seen"),
            last_scanned: r
                .try_get::<Option<String>, _>("last_scanned")
                .unwrap_or(None),
            scan_count: r.get::<i64, _>("scan_count"),
            latest_maturity_grade: r
                .try_get::<Option<String>, _>("latest_maturity_grade")
                .unwrap_or(None),
            latest_composite_maturity: r
                .try_get::<Option<i64>, _>("latest_composite_maturity")
                .unwrap_or(None),
            latest_risk_score: r
                .try_get::<Option<i64>, _>("latest_risk_score")
                .unwrap_or(None),
            max_unlocked_tier: r.get::<i64, _>("max_unlocked_tier"),
        })
        .collect())
}

/// Return a full overview for the given repo, or `None` if the repo has no v2
/// grade yet (or the repo ID does not exist).
pub async fn get_repo_overview(
    pool: &Pool,
    repo_id: &str,
) -> Result<Option<crate::models::RepoOverview>> {
    // 1. Find the latest scan that has a v2 grade (also pull raw_report for LLM narrative,
    //    scan_tier for which tier was run, and max_unlocked_tier from the repos row).
    let grade_row = sqlx::query(
        r#"SELECT sg.id AS grade_id, sg.composite, sg.grade, r.url AS repo_url,
                  s.raw_report, s.scan_tier, r.max_unlocked_tier
           FROM scan_grades sg
           JOIN scans s  ON s.id  = sg.scan_id
           JOIN repos  r ON r.id  = s.repo_id
           WHERE r.id = ?1
           ORDER BY s.scanned_at DESC
           LIMIT 1"#,
    )
    .bind(repo_id)
    .fetch_optional(pool)
    .await
    .context("get_repo_overview: fetch latest grade")?;

    let Some(grade_row) = grade_row else {
        return Ok(None);
    };

    let grade_id: String = grade_row.get("grade_id");
    let composite: i64 = grade_row.get("composite");
    let grade: String = grade_row.get("grade");
    let repo_url: String = grade_row.get("repo_url");
    let raw_report: String = grade_row.get("raw_report");
    let scan_tier: i64 = grade_row.get("scan_tier");
    let max_unlocked_tier: i64 = grade_row.get("max_unlocked_tier");

    // 2. Fetch dimension scores + signals for that grade (include tier per signal).
    let dim_rows = sqlx::query(
        r#"SELECT d.id AS dim_id, d.dimension, d.score, d.weight,
                  s.name, s.passed, s.points, s.detail, s.tier AS signal_tier
           FROM scan_dimension_scores_v2 d
           LEFT JOIN scan_signals s ON s.dimension_score_id = d.id
           WHERE d.scan_grade_id = ?1"#,
    )
    .bind(&grade_id)
    .fetch_all(pool)
    .await
    .context("get_repo_overview: fetch dimensions+signals")?;

    // 3. Aggregate into overview structures.
    use std::collections::HashMap;

    struct DimAccum {
        dimension: String,
        score: i64,
        weight: f64,
        passed: i64,
        total: i64,
        signals: Vec<crate::models::SignalItem>,
    }

    let mut dims: HashMap<String, DimAccum> = HashMap::new();
    let mut failed_signals: Vec<crate::models::BlockerItem> = Vec::new();
    let mut total_signals: i64 = 0;
    let mut passed_signals: i64 = 0;

    for row in &dim_rows {
        let dimension: String = row.get("dimension");
        let score: i64 = row.get("score");
        let weight: f64 = row.get("weight");
        let dim_entry = dims.entry(dimension.clone()).or_insert(DimAccum {
            dimension: dimension.clone(),
            score,
            weight,
            passed: 0,
            total: 0,
            signals: Vec::new(),
        });

        // Signals are LEFT JOINed — a dimension with no signals yields one NULL row.
        if let Ok(name) = row.try_get::<String, _>("name") {
            let passed: bool = row.get::<i64, _>("passed") != 0;
            let points: i64 = row.get("points");
            let detail: Option<String> = row.try_get("detail").ok().flatten();
            let tier: i64 = row.try_get("signal_tier").unwrap_or(1);

            dim_entry.total += 1;
            total_signals += 1;
            dim_entry.signals.push(crate::models::SignalItem {
                name: name.clone(),
                passed,
                points,
                detail: detail.clone(),
                tier,
            });
            if passed {
                dim_entry.passed += 1;
                passed_signals += 1;
            } else {
                failed_signals.push(crate::models::BlockerItem {
                    signal_name: name,
                    dimension: dimension.clone(),
                    points,
                    detail,
                });
            }
        }
    }

    let confidence = if total_signals == 0 {
        0.0
    } else {
        passed_signals as f64 / total_signals as f64 * 100.0
    };

    let mut top_blockers = failed_signals;
    top_blockers.sort_by(|a, b| b.points.cmp(&a.points));
    top_blockers.truncate(5);

    let mut signals_by_dimension: Vec<crate::models::DimensionSignals> = dims
        .values()
        .map(|d| crate::models::DimensionSignals {
            dimension: d.dimension.clone(),
            score: d.score,
            weight: d.weight,
            signals: {
                let mut sigs = d.signals.clone();
                // Failed (highest points) first, then passed.
                sigs.sort_by(|a, b| {
                    b.passed
                        .cmp(&a.passed)
                        .reverse()
                        .then(b.points.cmp(&a.points))
                });
                sigs
            },
        })
        .collect();
    signals_by_dimension.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .reverse()
            .then(a.dimension.cmp(&b.dimension))
    });

    let dimensions: Vec<crate::models::DimensionOverview> = dims
        .into_values()
        .map(|d| crate::models::DimensionOverview {
            dimension: d.dimension,
            score: d.score,
            weight: d.weight,
            passed_count: d.passed,
            total_count: d.total,
        })
        .collect();

    // 4. Parse LLM report from the raw_report JSON blob.
    let llm_report: Option<crate::models::LlmReport> =
        serde_json::from_str::<serde_json::Value>(&raw_report)
            .ok()
            .and_then(|v| {
                Some(crate::models::LlmReport {
                    summary: v["summary"].as_str()?.to_string(),
                    language_insights: v["language_insights"]
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .filter_map(|s| s.as_str().map(str::to_string))
                                .collect()
                        })
                        .unwrap_or_default(),
                    dependency_recommendations: v["dependency_recommendations"]
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .filter_map(|s| s.as_str().map(str::to_string))
                                .collect()
                        })
                        .unwrap_or_default(),
                    dockerfile_findings: v["dockerfile_findings"]
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .filter_map(|s| s.as_str().map(str::to_string))
                                .collect()
                        })
                        .unwrap_or_default(),
                    security_violations: v["security_violations"]
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .filter_map(|s| s.as_str().map(str::to_string))
                                .collect()
                        })
                        .unwrap_or_default(),
                    general_recommendations: v["general_recommendations"]
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .filter_map(|s| s.as_str().map(str::to_string))
                                .collect()
                        })
                        .unwrap_or_default(),
                })
            });

    Ok(Some(crate::models::RepoOverview {
        repo_id: repo_id.to_string(),
        repo_url,
        composite,
        grade,
        confidence,
        dimensions,
        top_blockers,
        signals_by_dimension,
        llm_report,
        scan_tier,
        max_unlocked_tier,
    }))
}

// ── Tier management ───────────────────────────────────────────────────────────

/// Return `(repo_id, max_unlocked_tier)` for a repo looked up by URL, or
/// `None` if the URL has never been seen before.
pub async fn get_repo_tier_by_url(pool: &Pool, url: &str) -> Result<Option<(String, i64)>> {
    let row = sqlx::query("SELECT id, max_unlocked_tier FROM repos WHERE url = ?1")
        .bind(url)
        .fetch_optional(pool)
        .await
        .context("get_repo_tier_by_url")?;
    Ok(row.map(|r| {
        (
            r.get::<String, _>("id"),
            r.get::<i64, _>("max_unlocked_tier"),
        )
    }))
}

/// Return the current `max_unlocked_tier` for a repo, or `None` if not found.
pub async fn get_repo_tier(pool: &Pool, repo_id: &str) -> Result<Option<i64>> {
    let row = sqlx::query("SELECT max_unlocked_tier FROM repos WHERE id = ?1")
        .bind(repo_id)
        .fetch_optional(pool)
        .await
        .context("get_repo_tier")?;
    Ok(row.map(|r| r.get::<i64, _>("max_unlocked_tier")))
}

/// Advance `max_unlocked_tier` for a repo to `new_tier`, but only if
/// `new_tier` is strictly greater than the current value (never regress).
///
/// Call this after a scan completes with all signals in the current tier
/// passing. Returns `true` if the value was actually updated.
pub async fn advance_repo_tier(pool: &Pool, repo_id: &str, new_tier: i64) -> Result<bool> {
    let result = sqlx::query(
        "UPDATE repos SET max_unlocked_tier = ?1 \
         WHERE id = ?2 AND max_unlocked_tier < ?1",
    )
    .bind(new_tier)
    .bind(repo_id)
    .execute(pool)
    .await
    .context("advance_repo_tier")?;
    Ok(result.rows_affected() > 0)
}

/// Return the trend series for a repo — up to `window` most-recent scan points,
/// ordered oldest-first (so sparklines display left-to-right chronologically).
pub async fn get_repo_trends(
    pool: &Pool,
    repo_id: &str,
    window: i64,
) -> Result<Vec<crate::models::TrendPoint>> {
    // Fetch the N most-recent grades with their scan timestamps.
    let grade_rows = sqlx::query(
        r#"SELECT sg.id AS grade_id, sg.composite, sg.grade, s.scanned_at
           FROM scan_grades sg
           JOIN scans s ON s.id = sg.scan_id
           JOIN repos  r ON r.id = s.repo_id
           WHERE r.id = ?1
           ORDER BY s.scanned_at DESC
           LIMIT ?2"#,
    )
    .bind(repo_id)
    .bind(window)
    .fetch_all(pool)
    .await
    .context("get_repo_trends: fetch grades")?;

    if grade_rows.is_empty() {
        return Ok(vec![]);
    }

    // Collect grade ids to fetch dimension scores.
    let grade_ids: Vec<String> = grade_rows.iter().map(|r| r.get("grade_id")).collect();

    // Build a map grade_id → HashMap<dimension, score>.
    let mut dim_map: std::collections::HashMap<String, std::collections::HashMap<String, i64>> =
        std::collections::HashMap::new();

    for gid in &grade_ids {
        let dim_rows = sqlx::query(
            "SELECT dimension, score FROM scan_dimension_scores_v2 WHERE scan_grade_id = ?1",
        )
        .bind(gid)
        .fetch_all(pool)
        .await
        .context("get_repo_trends: fetch dimensions")?;

        let entry = dim_map.entry(gid.clone()).or_default();
        for d in dim_rows {
            entry.insert(d.get("dimension"), d.get("score"));
        }
    }

    // Assemble points, then reverse to oldest-first.
    let mut points: Vec<crate::models::TrendPoint> = grade_rows
        .into_iter()
        .map(|r| {
            let gid: String = r.get("grade_id");
            let dims = dim_map.remove(&gid).unwrap_or_default();
            crate::models::TrendPoint {
                scanned_at: r.get("scanned_at"),
                composite: r.get("composite"),
                grade: r.get("grade"),
                dimensions: dims,
            }
        })
        .collect();

    points.reverse(); // oldest first
    Ok(points)
}
