// ── Migration tests ──────────────────────────────────────────────────────────
//
// Tests in this module verify that `002_grade_v2.sql` creates every expected
// v2 table and leaves the existing v1 tables intact.  They run against an
// in-memory SQLite database so they have no side-effects on real data.

#[cfg(test)]
mod migration {
    use sqlx::sqlite::SqlitePoolOptions;

    async fn open_test_pool() -> sqlx::SqlitePool {
        let pool = SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .expect("open in-memory DB");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("run migrations");
        pool
    }

    #[tokio::test]
    async fn v2_tables_exist_after_migration() {
        let pool = open_test_pool().await;
        for table in [
            "grade_models",
            "scan_grades",
            "scan_dimension_scores_v2",
            "scan_signals",
            "signal_evidence",
            "decision_graphs",
        ] {
            let count: (i64,) = sqlx::query_as(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?",
            )
            .bind(table)
            .fetch_one(&pool)
            .await
            .unwrap_or_else(|e| panic!("query sqlite_master for `{table}`: {e}"));
            assert_eq!(count.0, 1, "table `{table}` is missing after migration");
        }
    }

    #[tokio::test]
    async fn v1_tables_still_exist_after_migration_002() {
        let pool = open_test_pool().await;
        for table in ["repos", "scans", "maturity_dimensions", "violations"] {
            let count: (i64,) = sqlx::query_as(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?",
            )
            .bind(table)
            .fetch_one(&pool)
            .await
            .unwrap_or_else(|e| panic!("query sqlite_master for `{table}`: {e}"));
            assert_eq!(count.0, 1, "v1 table `{table}` disappeared after migration 002");
        }
    }

    #[tokio::test]
    async fn scan_grades_has_expected_columns() {
        let pool = open_test_pool().await;
        let rows = sqlx::query("PRAGMA table_info(scan_grades)")
            .fetch_all(&pool)
            .await
            .expect("PRAGMA table_info(scan_grades)");
        let names: Vec<String> = rows
            .iter()
            .map(|r| {
                use sqlx::Row;
                r.get::<String, _>("name")
            })
            .collect();
        for col in ["id", "scan_id", "model_id", "composite", "grade", "created_at"] {
            assert!(names.contains(&col.to_string()), "scan_grades missing column `{col}`");
        }
    }

    #[tokio::test]
    async fn scan_signals_has_expected_columns() {
        let pool = open_test_pool().await;
        let rows = sqlx::query("PRAGMA table_info(scan_signals)")
            .fetch_all(&pool)
            .await
            .expect("PRAGMA table_info(scan_signals)");
        let names: Vec<String> = rows
            .iter()
            .map(|r| {
                use sqlx::Row;
                r.get::<String, _>("name")
            })
            .collect();
        for col in [
            "id",
            "dimension_score_id",
            "name",
            "description",
            "passed",
            "points",
            "detail",
        ] {
            assert!(names.contains(&col.to_string()), "scan_signals missing column `{col}`");
        }
    }

    #[tokio::test]
    async fn v2_indices_exist_after_migration() {
        let pool = open_test_pool().await;
        for index in [
            "idx_scan_grades_scan",
            "idx_dim_scores_v2_grade",
            "idx_scan_signals_dim",
            "idx_signal_evidence_signal",
            "idx_decision_graphs_scan",
        ] {
            let count: (i64,) = sqlx::query_as(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name=?",
            )
            .bind(index)
            .fetch_one(&pool)
            .await
            .unwrap_or_else(|e| panic!("query index `{index}`: {e}"));
            assert_eq!(count.0, 1, "index `{index}` is missing after migration");
        }
    }
}

// ── Store integration tests ──────────────────────────────────────────────────
//
// These tests exercise the v2 query API end-to-end against an in-memory DB.
// Each test is independent; connections are not shared between tests.

#[cfg(test)]
mod store {
    use chrono::Utc;
    use sqlx::sqlite::SqlitePoolOptions;
    use uuid::Uuid;

    use crate::queries::{
        get_decision_graph_for_scan, get_scan_grade_v2, insert_decision_graph,
        insert_dimension_score_v2, insert_scan_grade, insert_scan_signal,
        insert_signal_evidence, upsert_grade_model,
    };

    async fn open_test_pool() -> sqlx::SqlitePool {
        let pool = SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .expect("open in-memory DB");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("run migrations");
        pool
    }

    /// Seed a minimal repo + scan row so FK constraints in v2 tables are satisfied.
    async fn seed_scan(pool: &sqlx::SqlitePool) -> String {
        let scan_id = Uuid::new_v4().to_string();
        let repo_id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO repos (id, url, first_seen) VALUES (?1, ?2, ?3)",
        )
        .bind(&repo_id)
        .bind(format!("https://example.com/{repo_id}"))
        .bind(&now)
        .execute(pool)
        .await
        .expect("seed repo");
        sqlx::query(
            r#"INSERT INTO scans
               (id, repo_id, scanned_at, duration_ms, risk_score,
                composite_maturity, maturity_grade, raw_report, raw_maturity)
               VALUES (?1, ?2, ?3, 100, 0, 75, 'GOLD', '{}', '{}')"#,
        )
        .bind(&scan_id)
        .bind(&repo_id)
        .bind(&now)
        .execute(pool)
        .await
        .expect("seed scan");
        scan_id
    }

    // ── grade_models ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn grade_model_upsert_and_query_round_trip() {
        let pool = open_test_pool().await;
        let model_id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        upsert_grade_model(&pool, &model_id, "2.0.0", "v2 grade model", &now)
            .await
            .expect("upsert_grade_model");

        let row: crate::models::GradeModelRow = sqlx::query_as(
            "SELECT id, version, description, created_at FROM grade_models WHERE id = ?",
        )
        .bind(&model_id)
        .fetch_one(&pool)
        .await
        .expect("fetch grade_model");

        assert_eq!(row.id, model_id);
        assert_eq!(row.version, "2.0.0");
        assert_eq!(row.description, "v2 grade model");
    }

    #[tokio::test]
    async fn grade_model_upsert_is_idempotent() {
        let pool = open_test_pool().await;
        let model_id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();

        upsert_grade_model(&pool, &model_id, "2.0.0", "first insert", &now)
            .await
            .expect("first upsert");
        // Same id, same version — should not error and should not duplicate.
        upsert_grade_model(&pool, &model_id, "2.0.0", "second insert", &now)
            .await
            .expect("second upsert (idempotent)");

        let count: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM grade_models WHERE id = ?")
                .bind(&model_id)
                .fetch_one(&pool)
                .await
                .expect("count");
        assert_eq!(count.0, 1, "upsert must not create duplicate rows");
    }

    // ── scan_grades ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn scan_grade_insert_and_query_round_trip() {
        let pool = open_test_pool().await;
        let scan_id = seed_scan(&pool).await;
        let model_id = Uuid::new_v4().to_string();
        let grade_id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();

        upsert_grade_model(&pool, &model_id, "2.0.0", "test", &now)
            .await
            .unwrap();
        insert_scan_grade(&pool, &grade_id, &scan_id, &model_id, 75, "GOLD", &now)
            .await
            .expect("insert_scan_grade");

        let row = get_scan_grade_v2(&pool, &scan_id)
            .await
            .expect("get_scan_grade_v2")
            .expect("row must be present");

        assert_eq!(row.id, grade_id);
        assert_eq!(row.scan_id, scan_id);
        assert_eq!(row.model_id, model_id);
        assert_eq!(row.composite, 75);
        assert_eq!(row.grade, "GOLD");
    }

    #[tokio::test]
    async fn get_scan_grade_v2_returns_none_when_absent() {
        let pool = open_test_pool().await;
        let result = get_scan_grade_v2(&pool, "does-not-exist")
            .await
            .expect("get_scan_grade_v2");
        assert!(result.is_none());
    }

    // ── scan_dimension_scores_v2 ─────────────────────────────────────────────

    #[tokio::test]
    async fn dimension_score_v2_insert_and_query_round_trip() {
        let pool = open_test_pool().await;
        let scan_id = seed_scan(&pool).await;
        let model_id = Uuid::new_v4().to_string();
        let grade_id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();

        upsert_grade_model(&pool, &model_id, "2.0.0", "test", &now)
            .await
            .unwrap();
        insert_scan_grade(&pool, &grade_id, &scan_id, &model_id, 75, "GOLD", &now)
            .await
            .unwrap();

        let dim_id =
            insert_dimension_score_v2(&pool, &grade_id, "Security", 80, 0.25)
                .await
                .expect("insert_dimension_score_v2");

        let row: crate::models::DimensionScoreV2Row = sqlx::query_as(
            "SELECT id, scan_grade_id, dimension, score, weight \
             FROM scan_dimension_scores_v2 WHERE id = ?",
        )
        .bind(dim_id)
        .fetch_one(&pool)
        .await
        .expect("fetch dimension_score_v2");

        assert_eq!(row.scan_grade_id, grade_id);
        assert_eq!(row.dimension, "Security");
        assert_eq!(row.score, 80);
        assert!((row.weight - 0.25).abs() < 1e-9, "weight should be 0.25");
    }

    // ── scan_signals ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn scan_signal_insert_and_query_round_trip() {
        let pool = open_test_pool().await;
        let scan_id = seed_scan(&pool).await;
        let model_id = Uuid::new_v4().to_string();
        let grade_id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();

        upsert_grade_model(&pool, &model_id, "2.0.0", "test", &now)
            .await
            .unwrap();
        insert_scan_grade(&pool, &grade_id, &scan_id, &model_id, 75, "GOLD", &now)
            .await
            .unwrap();
        let dim_id =
            insert_dimension_score_v2(&pool, &grade_id, "Security", 80, 0.25)
                .await
                .unwrap();

        let signal_id = insert_scan_signal(
            &pool,
            dim_id,
            "no_critical_violations",
            "No critical committed files",
            true,
            40,
            None,
        )
        .await
        .expect("insert_scan_signal");

        let row: crate::models::ScanSignalRow = sqlx::query_as(
            "SELECT id, dimension_score_id, name, description, passed, points, detail \
             FROM scan_signals WHERE id = ?",
        )
        .bind(signal_id)
        .fetch_one(&pool)
        .await
        .expect("fetch scan_signal");

        assert_eq!(row.dimension_score_id, dim_id);
        assert_eq!(row.name, "no_critical_violations");
        assert!(row.passed);
        assert_eq!(row.points, 40);
        assert!(row.detail.is_none());
    }

    #[tokio::test]
    async fn scan_signal_with_detail_round_trip() {
        let pool = open_test_pool().await;
        let scan_id = seed_scan(&pool).await;
        let model_id = Uuid::new_v4().to_string();
        let grade_id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        upsert_grade_model(&pool, &model_id, "2.0.0", "t", &now)
            .await
            .unwrap();
        insert_scan_grade(&pool, &grade_id, &scan_id, &model_id, 50, "GOLD", &now)
            .await
            .unwrap();
        let dim_id = insert_dimension_score_v2(&pool, &grade_id, "Security", 50, 0.25)
            .await
            .unwrap();

        let signal_id = insert_scan_signal(
            &pool,
            dim_id,
            "security_policy",
            "SECURITY.md present",
            false,
            25,
            Some("Add a SECURITY.md file"),
        )
        .await
        .unwrap();

        let row: crate::models::ScanSignalRow = sqlx::query_as(
            "SELECT id, dimension_score_id, name, description, passed, points, detail \
             FROM scan_signals WHERE id = ?",
        )
        .bind(signal_id)
        .fetch_one(&pool)
        .await
        .unwrap();

        assert!(!row.passed);
        assert_eq!(row.detail.as_deref(), Some("Add a SECURITY.md file"));
    }

    // ── signal_evidence ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn signal_evidence_insert_and_query_round_trip() {
        let pool = open_test_pool().await;
        let scan_id = seed_scan(&pool).await;
        let model_id = Uuid::new_v4().to_string();
        let grade_id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();

        upsert_grade_model(&pool, &model_id, "2.0.0", "test", &now)
            .await
            .unwrap();
        insert_scan_grade(&pool, &grade_id, &scan_id, &model_id, 75, "GOLD", &now)
            .await
            .unwrap();
        let dim_id = insert_dimension_score_v2(&pool, &grade_id, "Security", 80, 0.25)
            .await
            .unwrap();
        let signal_id = insert_scan_signal(
            &pool,
            dim_id,
            "test_signal",
            "A test signal",
            false,
            20,
            Some("fix needed"),
        )
        .await
        .unwrap();

        insert_signal_evidence(&pool, signal_id, "count", "3")
            .await
            .expect("insert_signal_evidence");

        let row: crate::models::SignalEvidenceRow = sqlx::query_as(
            "SELECT id, signal_id, kind, value FROM signal_evidence WHERE signal_id = ?",
        )
        .bind(signal_id)
        .fetch_one(&pool)
        .await
        .expect("fetch signal_evidence");

        assert_eq!(row.signal_id, signal_id);
        assert_eq!(row.kind, "count");
        assert_eq!(row.value, "3");
    }

    // ── decision_graphs ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn decision_graph_insert_and_query_round_trip() {
        let pool = open_test_pool().await;
        let scan_id = seed_scan(&pool).await;
        let graph_id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        let graph_json = r#"{"nodes":[],"edges":[]}"#;

        insert_decision_graph(&pool, &graph_id, &scan_id, graph_json, &now)
            .await
            .expect("insert_decision_graph");

        let row = get_decision_graph_for_scan(&pool, &scan_id)
            .await
            .expect("get_decision_graph_for_scan")
            .expect("row must be present");

        assert_eq!(row.id, graph_id);
        assert_eq!(row.scan_id, scan_id);
        assert_eq!(row.graph_json, graph_json);
    }

    #[tokio::test]
    async fn get_decision_graph_returns_none_when_absent() {
        let pool = open_test_pool().await;
        let result = get_decision_graph_for_scan(&pool, "no-such-scan")
            .await
            .expect("query should not error");
        assert!(result.is_none());
    }

    // ── Full chain ───────────────────────────────────────────────────────────

    #[tokio::test]
    async fn full_v2_chain_insert_and_query() {
        let pool = open_test_pool().await;
        let scan_id = seed_scan(&pool).await;
        let model_id = Uuid::new_v4().to_string();
        let grade_id = Uuid::new_v4().to_string();
        let graph_id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();

        // 1. Grade model
        upsert_grade_model(&pool, &model_id, "2.0.0", "full-chain model", &now)
            .await
            .unwrap();

        // 2. Scan grade
        insert_scan_grade(&pool, &grade_id, &scan_id, &model_id, 82, "PLATINUM", &now)
            .await
            .unwrap();

        // 3. Two dimensions with signals
        let security_id =
            insert_dimension_score_v2(&pool, &grade_id, "Security", 90, 0.25)
                .await
                .unwrap();
        let gov_id =
            insert_dimension_score_v2(&pool, &grade_id, "Project Governance", 70, 0.15)
                .await
                .unwrap();

        let sig1 = insert_scan_signal(
            &pool,
            security_id,
            "no_critical_violations",
            "No criticals",
            true,
            40,
            None,
        )
        .await
        .unwrap();

        let sig2 = insert_scan_signal(
            &pool,
            gov_id,
            "license_present",
            "License file present",
            false,
            30,
            Some("Add a LICENSE file"),
        )
        .await
        .unwrap();

        // 4. Evidence
        insert_signal_evidence(&pool, sig1, "file_found", "SECURITY.md")
            .await
            .unwrap();
        insert_signal_evidence(&pool, sig2, "file_missing", "LICENSE")
            .await
            .unwrap();

        // 5. Decision graph
        insert_decision_graph(
            &pool,
            &graph_id,
            &scan_id,
            r#"{"nodes":["root"],"edges":[]}"#,
            &now,
        )
        .await
        .unwrap();

        // ── Query and assert ─────────────────────────────────────────────────

        let grade = get_scan_grade_v2(&pool, &scan_id)
            .await
            .unwrap()
            .expect("scan grade must exist");
        assert_eq!(grade.composite, 82);
        assert_eq!(grade.grade, "PLATINUM");

        let dim_count: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM scan_dimension_scores_v2 WHERE scan_grade_id = ?",
        )
        .bind(&grade_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(dim_count.0, 2, "expected 2 dimension scores");

        let signal_count: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM scan_signals \
             WHERE dimension_score_id IN \
               (SELECT id FROM scan_dimension_scores_v2 WHERE scan_grade_id = ?)",
        )
        .bind(&grade_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(signal_count.0, 2, "expected 2 signals");

        let evidence_count: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM signal_evidence \
             WHERE signal_id IN \
               (SELECT id FROM scan_signals \
                WHERE dimension_score_id IN \
                  (SELECT id FROM scan_dimension_scores_v2 WHERE scan_grade_id = ?))",
        )
        .bind(&grade_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(evidence_count.0, 2, "expected 2 evidence records");

        let graph = get_decision_graph_for_scan(&pool, &scan_id)
            .await
            .unwrap()
            .expect("decision graph must exist");
        assert!(graph.graph_json.contains("root"));
    }
}

// ── Backfill tests ───────────────────────────────────────────────────────────
//
// Verify that `backfill_v2_grades` correctly projects historical v1 scan rows
// (which carry `raw_maturity` JSON but no v2 rows) into the v2 schema, and
// that running it a second time is a no-op (idempotent).

#[cfg(test)]
mod backfill {
    use chrono::Utc;
    use sqlx::sqlite::SqlitePoolOptions;
    use uuid::Uuid;

    use crate::queries::backfill_v2_grades;

    async fn open_test_pool() -> sqlx::SqlitePool {
        let pool = SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .expect("open in-memory DB");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("run migrations");
        pool
    }

    /// Build a minimal but valid `MaturityScore` JSON blob that matches the
    /// shape produced by `rusty_venture_actions::repo::maturity::MaturityScore`.
    fn minimal_raw_maturity(composite: u8, grade: &str) -> String {
        serde_json::json!({
            "composite": composite,
            "grade": grade,
            "dimensions": [
                {
                    "dimension": "Security",
                    "score": composite,
                    "signals": [
                        {
                            "name": "no_critical_violations",
                            "description": "No critical files committed",
                            "passed": true,
                            "points": 40,
                            "detail": null
                        }
                    ]
                }
            ]
        })
        .to_string()
    }

    async fn seed_legacy_scan(pool: &sqlx::SqlitePool, composite: u8, grade: &str) -> String {
        let scan_id = Uuid::new_v4().to_string();
        let repo_id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        let raw_maturity = minimal_raw_maturity(composite, grade);

        sqlx::query("INSERT INTO repos (id, url, first_seen) VALUES (?1, ?2, ?3)")
            .bind(&repo_id)
            .bind(format!("https://example.com/{repo_id}"))
            .bind(&now)
            .execute(pool)
            .await
            .unwrap();
        sqlx::query(
            r#"INSERT INTO scans
               (id, repo_id, scanned_at, duration_ms, risk_score,
                composite_maturity, maturity_grade, raw_report, raw_maturity)
               VALUES (?1, ?2, ?3, 200, 0, ?4, ?5, '{}', ?6)"#,
        )
        .bind(&scan_id)
        .bind(&repo_id)
        .bind(&now)
        .bind(composite as i64)
        .bind(grade)
        .bind(&raw_maturity)
        .execute(pool)
        .await
        .unwrap();
        scan_id
    }

    #[tokio::test]
    async fn backfill_creates_v2_grade_for_legacy_scan() {
        let pool = open_test_pool().await;
        let scan_id = seed_legacy_scan(&pool, 75, "GOLD").await;

        let backfilled = backfill_v2_grades(&pool).await.expect("backfill_v2_grades");
        assert_eq!(backfilled, 1, "should have backfilled exactly 1 scan");

        let count: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM scan_grades WHERE scan_id = ?")
                .bind(&scan_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(count.0, 1, "scan_grades row must exist after backfill");
    }

    #[tokio::test]
    async fn backfill_populates_grade_fields_from_raw_maturity() {
        let pool = open_test_pool().await;
        let scan_id = seed_legacy_scan(&pool, 82, "PLATINUM").await;
        backfill_v2_grades(&pool).await.unwrap();

        let row: crate::models::ScanGradeRow = sqlx::query_as(
            "SELECT id, scan_id, model_id, composite, grade, created_at \
             FROM scan_grades WHERE scan_id = ?",
        )
        .bind(&scan_id)
        .fetch_one(&pool)
        .await
        .expect("scan_grade row must exist");

        assert_eq!(row.composite, 82);
        assert_eq!(row.grade, "PLATINUM");
    }

    #[tokio::test]
    async fn backfill_creates_dimension_and_signal_rows() {
        let pool = open_test_pool().await;
        let scan_id = seed_legacy_scan(&pool, 75, "GOLD").await;
        backfill_v2_grades(&pool).await.unwrap();

        let grade: crate::models::ScanGradeRow = sqlx::query_as(
            "SELECT id, scan_id, model_id, composite, grade, created_at \
             FROM scan_grades WHERE scan_id = ?",
        )
        .bind(&scan_id)
        .fetch_one(&pool)
        .await
        .unwrap();

        let dim_count: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM scan_dimension_scores_v2 WHERE scan_grade_id = ?",
        )
        .bind(&grade.id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(dim_count.0, 1, "expected 1 dimension row (Security)");

        let sig_count: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM scan_signals \
             WHERE dimension_score_id IN \
               (SELECT id FROM scan_dimension_scores_v2 WHERE scan_grade_id = ?)",
        )
        .bind(&grade.id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(sig_count.0, 1, "expected 1 signal row");
    }

    #[tokio::test]
    async fn backfill_is_idempotent() {
        let pool = open_test_pool().await;
        let scan_id = seed_legacy_scan(&pool, 55, "GOLD").await;

        backfill_v2_grades(&pool).await.expect("first backfill");
        let second = backfill_v2_grades(&pool).await.expect("second backfill");
        assert_eq!(second, 0, "second backfill must be a no-op");

        let count: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM scan_grades WHERE scan_id = ?")
                .bind(&scan_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(count.0, 1, "must not duplicate scan_grades row");
    }

    #[tokio::test]
    async fn backfill_handles_multiple_legacy_scans() {
        let pool = open_test_pool().await;
        seed_legacy_scan(&pool, 10, "BRONZE").await;
        seed_legacy_scan(&pool, 50, "GOLD").await;
        seed_legacy_scan(&pool, 90, "DIAMOND").await;

        let backfilled = backfill_v2_grades(&pool).await.expect("backfill_v2_grades");
        assert_eq!(backfilled, 3, "should backfill all 3 legacy scans");

        let total: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM scan_grades")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(total.0, 3);
    }

    #[tokio::test]
    async fn backfill_skips_scans_that_already_have_v2_grade() {
        let pool = open_test_pool().await;
        // One legacy scan, one scan that already has a v2 grade (inserted directly).
        let legacy_id = seed_legacy_scan(&pool, 30, "SILVER").await;
        let modern_id = seed_legacy_scan(&pool, 70, "GOLD").await;

        // Pre-insert a v2 grade for `modern_id` to simulate a scan that was
        // already processed by new code.
        let model_id = "model-v2.0.0";
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT OR IGNORE INTO grade_models (id, version, description, created_at) \
             VALUES (?1, '2.0.0', 'pre-existing', ?2)",
        )
        .bind(model_id)
        .bind(&now)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO scan_grades (id, scan_id, model_id, composite, grade, created_at) \
             VALUES (?1, ?2, ?3, 70, 'GOLD', ?4)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(&modern_id)
        .bind(model_id)
        .bind(&now)
        .execute(&pool)
        .await
        .unwrap();

        let backfilled = backfill_v2_grades(&pool).await.expect("backfill_v2_grades");
        assert_eq!(backfilled, 1, "only the legacy scan should be backfilled");

        let _ = legacy_id; // used via seed_legacy_scan
    }
}

// ── Serialization tests ──────────────────────────────────────────────────────
//
// These are pure unit tests — no async, no DB.  They verify that the new v2
// row types implement Serialize/Deserialize correctly.

#[cfg(test)]
mod serialization {
    use crate::models::{
        DecisionGraphRow, DimensionScoreV2Row, GradeModelRow, ScanGradeRow, ScanSignalRow,
        SignalEvidenceRow,
    };

    #[test]
    fn grade_model_row_round_trips_through_json() {
        let original = GradeModelRow {
            id: "model-123".to_string(),
            version: "2.1.0".to_string(),
            description: "A grade model".to_string(),
            created_at: "2024-06-01T12:00:00Z".to_string(),
        };
        let json = serde_json::to_string(&original).unwrap();
        let back: GradeModelRow = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, original.id);
        assert_eq!(back.version, original.version);
        assert_eq!(back.description, original.description);
        assert_eq!(back.created_at, original.created_at);
    }

    #[test]
    fn scan_grade_row_serializes_expected_fields() {
        let row = ScanGradeRow {
            id: "grade-1".to_string(),
            scan_id: "scan-1".to_string(),
            model_id: "model-1".to_string(),
            composite: 75,
            grade: "GOLD".to_string(),
            created_at: "2024-01-01T00:00:00Z".to_string(),
        };
        let json = serde_json::to_string(&row).expect("serialize ScanGradeRow");
        assert!(json.contains("\"grade\":\"GOLD\""));
        assert!(json.contains("\"composite\":75"));
        assert!(json.contains("\"scan_id\":\"scan-1\""));
    }

    #[test]
    fn scan_grade_row_composite_preserves_boundary_values() {
        for composite in [0i64, 20, 21, 40, 41, 60, 61, 80, 81, 100] {
            let row = ScanGradeRow {
                id: "x".into(),
                scan_id: "s".into(),
                model_id: "m".into(),
                composite,
                grade: "BRONZE".into(),
                created_at: "2024-01-01T00:00:00Z".into(),
            };
            let json = serde_json::to_string(&row).unwrap();
            let back: ScanGradeRow = serde_json::from_str(&json).unwrap();
            assert_eq!(back.composite, composite);
        }
    }

    #[test]
    fn dimension_score_v2_row_round_trips_through_json() {
        let original = DimensionScoreV2Row {
            id: 7,
            scan_grade_id: "grade-abc".to_string(),
            dimension: "Security".to_string(),
            score: 88,
            weight: 0.25,
        };
        let json = serde_json::to_string(&original).unwrap();
        let back: DimensionScoreV2Row = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, original.id);
        assert_eq!(back.score, original.score);
        assert!((back.weight - original.weight).abs() < 1e-9);
    }

    #[test]
    fn scan_signal_row_with_no_detail_round_trips() {
        let original = ScanSignalRow {
            id: 1,
            dimension_score_id: 3,
            name: "no_critical_violations".to_string(),
            description: "No criticals".to_string(),
            passed: true,
            points: 40,
            detail: None,
        };
        let json = serde_json::to_string(&original).unwrap();
        let back: ScanSignalRow = serde_json::from_str(&json).unwrap();
        assert!(back.passed);
        assert!(back.detail.is_none());
    }

    #[test]
    fn scan_signal_row_with_detail_round_trips() {
        let original = ScanSignalRow {
            id: 2,
            dimension_score_id: 3,
            name: "security_policy".to_string(),
            description: "SECURITY.md present".to_string(),
            passed: false,
            points: 25,
            detail: Some("Add a SECURITY.md".to_string()),
        };
        let json = serde_json::to_string(&original).unwrap();
        let back: ScanSignalRow = serde_json::from_str(&json).unwrap();
        assert!(!back.passed);
        assert_eq!(back.detail.as_deref(), Some("Add a SECURITY.md"));
    }

    #[test]
    fn signal_evidence_row_round_trips_through_json() {
        let original = SignalEvidenceRow {
            id: 10,
            signal_id: 5,
            kind: "file_missing".to_string(),
            value: "LICENSE".to_string(),
        };
        let json = serde_json::to_string(&original).unwrap();
        let back: SignalEvidenceRow = serde_json::from_str(&json).unwrap();
        assert_eq!(back.kind, original.kind);
        assert_eq!(back.value, original.value);
    }

    #[test]
    fn decision_graph_row_round_trips_through_json() {
        let original = DecisionGraphRow {
            id: "graph-1".to_string(),
            scan_id: "scan-1".to_string(),
            graph_json: r#"{"nodes":["root"],"edges":[]}"#.to_string(),
            created_at: "2024-01-01T00:00:00Z".to_string(),
        };
        let json = serde_json::to_string(&original).expect("serialize DecisionGraphRow");
        let back: DecisionGraphRow = serde_json::from_str(&json).unwrap();
        assert_eq!(back.scan_id, original.scan_id);
        assert!(back.graph_json.contains("root"));
    }
}
