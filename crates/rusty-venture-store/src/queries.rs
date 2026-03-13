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
