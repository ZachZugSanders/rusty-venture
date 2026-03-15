use anyhow::{Context, Result};
use chrono::Utc;
use sqlx::Row;
use uuid::Uuid;

use rusty_venture_actions::repo::{audit_files::AuditReport, maturity::MaturityScore, RepoAnalysisResult};

use crate::db::Pool;

/// Persist a completed analysis run to the database in a single transaction.
pub async fn insert_scan(
    pool: &Pool,
    result: &RepoAnalysisResult,
    maturity: &MaturityScore,
    audit: &AuditReport,
) -> Result<()> {
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
                composite_maturity, maturity_grade, raw_report, raw_maturity)
           VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)"#,
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
                 (dimension_score_id, name, description, passed, points, detail) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )
            .bind(dim_id)
            .bind(&signal.name)
            .bind(&signal.description)
            .bind(signal.passed as i64)
            .bind(signal.points as i64)
            .bind(&signal.detail)
            .execute(&mut *tx)
            .await
            .context("insert scan_signal in insert_scan")?;
        }
    }

    // Store the full MaturityScore JSON as the decision-graph artifact.
    let graph_id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO decision_graphs (id, scan_id, graph_json, created_at) \
         VALUES (?1, ?2, ?3, ?4)",
    )
    .bind(&graph_id)
    .bind(scan_id)
    .bind(&raw_maturity)
    .bind(&now)
    .execute(&mut *tx)
    .await
    .context("insert decision_graph in insert_scan")?;

    tx.commit().await.context("commit transaction")?;

    tracing::info!(
        repo = %result.repo_url,
        scan_id = %scan_id,
        composite_maturity = maturity.composite,
        grade = %grade,
        "Scan persisted to database"
    );

    Ok(())
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
pub async fn insert_scan_signal(
    pool: &Pool,
    dimension_score_id: i64,
    name: &str,
    description: &str,
    passed: bool,
    points: i64,
    detail: Option<&str>,
) -> Result<i64> {
    let result = sqlx::query(
        "INSERT INTO scan_signals \
         (dimension_score_id, name, description, passed, points, detail) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )
    .bind(dimension_score_id)
    .bind(name)
    .bind(description)
    .bind(passed as i64)
    .bind(points)
    .bind(detail)
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
    sqlx::query(
        "INSERT INTO signal_evidence (signal_id, kind, value) VALUES (?1, ?2, ?3)",
    )
    .bind(signal_id)
    .bind(kind)
    .bind(value)
    .execute(pool)
    .await
    .context("insert signal_evidence")?;
    Ok(())
}

/// Store the decision-graph artifact for a scan.
pub async fn insert_decision_graph(
    pool: &Pool,
    id: &str,
    scan_id: &str,
    graph_json: &str,
    created_at: &str,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO decision_graphs (id, scan_id, graph_json, created_at) \
         VALUES (?1, ?2, ?3, ?4)",
    )
    .bind(id)
    .bind(scan_id)
    .bind(graph_json)
    .bind(created_at)
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
        "SELECT id, scan_id, graph_json, created_at \
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
                     (dimension_score_id, name, description, passed, points, detail) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                )
                .bind(dim_id)
                .bind(&signal.name)
                .bind(&signal.description)
                .bind(signal.passed as i64)
                .bind(signal.points as i64)
                .bind(&signal.detail)
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


// ── v1 list helpers ───────────────────────────────────────────────────────────

/// Return the N most recent scans for one specific repo URL.
pub async fn list_scans_for_repo(pool: &Pool, repo_url: &str, limit: i64) -> Result<Vec<ScanSummary>> {
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
