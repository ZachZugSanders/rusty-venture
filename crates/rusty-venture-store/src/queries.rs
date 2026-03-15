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

/// A single repo's latest-scan summary for the Repos table.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RepoSummary {
    pub id: String,
    pub url: String,
    pub first_seen: String,
    pub last_scanned: Option<String>,
    /// Latest scan id (if any)
    pub latest_scan_id: Option<String>,
    pub latest_risk_score: Option<u8>,
    pub latest_composite_maturity: Option<u8>,
    pub latest_maturity_grade: Option<String>,
    pub scan_count: i64,
}

/// Return all tracked repos with their most-recent scan metrics.
pub async fn list_repos(pool: &Pool) -> Result<Vec<RepoSummary>> {
    let rows = sqlx::query(
        r#"SELECT r.id, r.url, r.first_seen, r.last_scanned,
                  s.id        AS latest_scan_id,
                  s.risk_score,
                  s.composite_maturity,
                  s.maturity_grade,
                  (SELECT COUNT(*) FROM scans WHERE repo_id = r.id) AS scan_count
           FROM repos r
           LEFT JOIN scans s ON s.id = (
               SELECT id FROM scans WHERE repo_id = r.id ORDER BY scanned_at DESC LIMIT 1
           )
           ORDER BY r.last_scanned DESC"#,
    )
    .fetch_all(pool)
    .await
    .context("list repos")?;

    Ok(rows
        .into_iter()
        .map(|r| RepoSummary {
            id: r.get::<String, _>("id"),
            url: r.get::<String, _>("url"),
            first_seen: r.get::<String, _>("first_seen"),
            last_scanned: r.get::<Option<String>, _>("last_scanned"),
            latest_scan_id: r.get::<Option<String>, _>("latest_scan_id"),
            latest_risk_score: r.get::<Option<i64>, _>("risk_score").map(|v| v as u8),
            latest_composite_maturity: r.get::<Option<i64>, _>("composite_maturity").map(|v| v as u8),
            latest_maturity_grade: r.get::<Option<String>, _>("maturity_grade"),
            scan_count: r.get::<i64, _>("scan_count"),
        })
        .collect())
}

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
    let maturity: serde_json::Value =
        serde_json::from_str(&r.get::<String, _>("raw_maturity")).unwrap_or(serde_json::Value::Null);

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
