use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use rusty_venture_actions::repo::{run_repo_analysis, maturity::MaturityScore, AuditReport, RepoAnalysisRequest};
use rusty_venture_actions::DecisionGraph;
use rusty_venture_store::{get_decision_graph_for_scan, get_scan, insert_scan, list_repos, list_scans, list_scans_for_repo, open_pool, get_repo_overview, get_repo_trends, Pool};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tower_http::trace::TraceLayer;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

/// Shared application state injected into all route handlers.
#[derive(Clone)]
struct AppState {
    anthropic_api_key: Arc<String>,
    db: Arc<Pool>,
}

/// Request body for POST /analyze.
#[derive(Debug, Deserialize)]
struct AnalyzeRequest {
    repo_url: String,
    branch: Option<String>,
}

/// Query params for GET /scans.
#[derive(Debug, Deserialize)]
struct ScansQuery {
    repo: Option<String>,
    limit: Option<i64>,
}

/// Query params for GET /repos/:id/trends.
#[derive(Debug, Deserialize)]
struct TrendsQuery {
    window: Option<i64>,
}

/// Success response wrapper.
#[derive(Serialize)]
struct ApiResponse<T> {
    success: bool,
    data: T,
}

/// Error response.
#[derive(Serialize)]
struct ApiError {
    success: bool,
    error: String,
}

impl ApiError {
    fn new(msg: impl Into<String>) -> Json<ApiError> {
        Json(ApiError {
            success: false,
            error: msg.into(),
        })
    }
}

/// Build the application router from the given state.
///
/// Extracted from `main` so integration tests can construct a `Router`
/// without binding to a real TCP port.
fn build_app(state: AppState) -> Router {
    Router::new()
        .route("/analyze", post(analyze_handler))
        .route("/scans", get(scans_handler))
        .route("/scans/:id", get(scan_detail_handler))
        .route("/repos", get(repos_handler))
        .route("/repos/:id/overview", get(repo_overview_handler))
        .route("/repos/:id/trends", get(repo_trends_handler))
        .route("/scans/:id/decision-graph", get(scan_decision_graph_handler))
        .route("/health", get(health_handler))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok(); // load .env if present — no-op if file is missing

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .json()
        .init();

    let api_key = std::env::var("ANTHROPIC_API_KEY")
        .expect("ANTHROPIC_API_KEY environment variable must be set");

    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "sqlite://rusty-venture.db".to_string());

    let pool = open_pool(Some(&database_url))
        .await
        .expect("Failed to open database");

    let state = AppState {
        anthropic_api_key: Arc::new(api_key),
        db: Arc::new(pool),
    };

    let app = build_app(state);

    let addr = std::env::var("LISTEN_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".to_string());
    info!(addr = %addr, "rusty-venture-server starting");

    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect("Failed to bind to address");

    axum::serve(listener, app)
        .await
        .expect("Server error");
}

async fn scan_decision_graph_handler(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    // 1. Look up the stored graph blob (raw MaturityScore JSON).
    let row = match get_decision_graph_for_scan(&state.db, &id).await {
        Ok(Some(row)) => row,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                ApiError::new(format!("No decision graph for scan {id}")),
            )
                .into_response();
        }
        Err(e) => {
            warn!(scan_id = %id, error = %e, "Failed to fetch decision graph");
            return (StatusCode::INTERNAL_SERVER_ERROR, ApiError::new(e.to_string()))
                .into_response();
        }
    };

    // 2. Deserialise the blob as a MaturityScore.
    let maturity: MaturityScore = match serde_json::from_str(&row.graph_json) {
        Ok(m) => m,
        Err(e) => {
            warn!(scan_id = %id, error = %e, "Failed to parse decision graph blob");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                ApiError::new(format!("Corrupted decision-graph blob: {e}")),
            )
                .into_response();
        }
    };

    // 3. Transform to the typed graph payload.
    let graph = DecisionGraph::from_maturity(&maturity);

    (StatusCode::OK, Json(ApiResponse { success: true, data: graph })).into_response()
}

async fn health_handler() -> impl IntoResponse {
    Json(serde_json::json!({ "status": "ok", "version": env!("CARGO_PKG_VERSION") }))
}

async fn analyze_handler(
    State(state): State<AppState>,
    Json(body): Json<AnalyzeRequest>,
) -> impl IntoResponse {
    info!(repo_url = %body.repo_url, "Received analyze request");

    match run_repo_analysis(RepoAnalysisRequest {
        repo_url: body.repo_url.clone(),
        branch: body.branch,
        claude_api_key: (*state.anthropic_api_key).clone(),
        docker_socket: None,
    })
    .await
    {
        Ok(result) => {
            // Persist the scan (non-fatal if it fails)
            let empty_audit = AuditReport::default();
            if let Err(e) = insert_scan(&state.db, &result, &result.maturity, &empty_audit).await {
                warn!(error = %e, "Failed to persist scan to database");
            }

            (
                StatusCode::OK,
                Json(ApiResponse {
                    success: true,
                    data: result,
                }),
            )
                .into_response()
        }
        Err(e) => {
            warn!(repo_url = %body.repo_url, error = %e, "Analysis failed");
            (StatusCode::INTERNAL_SERVER_ERROR, ApiError::new(e.to_string())).into_response()
        }
    }
}

async fn scan_detail_handler(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match get_scan(&state.db, &id).await {
        Ok(Some(detail)) => (
            StatusCode::OK,
            Json(ApiResponse { success: true, data: detail }),
        )
            .into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            ApiError::new(format!("Scan {id} not found")),
        )
            .into_response(),
        Err(e) => {
            warn!(scan_id = %id, error = %e, "Failed to fetch scan detail");
            (StatusCode::INTERNAL_SERVER_ERROR, ApiError::new(e.to_string())).into_response()
        }
    }
}

async fn scans_handler(
    State(state): State<AppState>,
    Query(params): Query<ScansQuery>,
) -> impl IntoResponse {
    let limit = params.limit.unwrap_or(20);

    let result = match &params.repo {
        Some(url) => list_scans_for_repo(&state.db, url, limit).await,
        None => list_scans(&state.db, limit).await,
    };

    match result {
        Ok(scans) => (
            StatusCode::OK,
            Json(ApiResponse {
                success: true,
                data: scans,
            }),
        )
            .into_response(),
        Err(e) => {
            warn!(error = %e, "Failed to list scans");
            (StatusCode::INTERNAL_SERVER_ERROR, ApiError::new(e.to_string())).into_response()
        }
    }
}

async fn repos_handler(State(state): State<AppState>) -> impl IntoResponse {
    match list_repos(&state.db, 100).await {
        Ok(repos) => (
            StatusCode::OK,
            Json(ApiResponse {
                success: true,
                data: repos,
            }),
        )
            .into_response(),
        Err(e) => {
            warn!(error = %e, "Failed to list repos");
            (StatusCode::INTERNAL_SERVER_ERROR, ApiError::new(e.to_string())).into_response()
        }
    }
}

async fn repo_overview_handler(
    State(state): State<AppState>,
    Path(repo_id): Path<String>,
) -> impl IntoResponse {
    match get_repo_overview(&state.db, &repo_id).await {
        Ok(Some(overview)) => (
            StatusCode::OK,
            Json(ApiResponse {
                success: true,
                data: overview,
            }),
        )
            .into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            ApiError::new(format!("no overview for repo {repo_id}")),
        )
            .into_response(),
        Err(e) => {
            warn!(repo_id = %repo_id, error = %e, "Failed to get repo overview");
            (StatusCode::INTERNAL_SERVER_ERROR, ApiError::new(e.to_string())).into_response()
        }
    }
}

async fn repo_trends_handler(
    State(state): State<AppState>,
    Path(repo_id): Path<String>,
    Query(params): Query<TrendsQuery>,
) -> impl IntoResponse {
    let window = params.window.unwrap_or(10);
    match get_repo_trends(&state.db, &repo_id, window).await {
        Ok(trends) => (
            StatusCode::OK,
            Json(ApiResponse {
                success: true,
                data: trends,
            }),
        )
            .into_response(),
        Err(e) => {
            warn!(repo_id = %repo_id, error = %e, "Failed to get repo trends");
            (StatusCode::INTERNAL_SERVER_ERROR, ApiError::new(e.to_string())).into_response()
        }
    }
}

// ── Integration tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use sqlx::sqlite::SqlitePoolOptions;
    use tower::ServiceExt;

    /// Create a test in-memory database, run migrations, and return the pool.
    async fn test_pool() -> Pool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory DB");
        sqlx::migrate!("../rusty-venture-store/migrations")
            .run(&pool)
            .await
            .expect("run migrations on test DB");
        pool
    }

    fn test_state(pool: Pool) -> AppState {
        AppState {
            anthropic_api_key: Arc::new("test-key".to_string()),
            db: Arc::new(pool),
        }
    }

    async fn body_json(body: Body) -> serde_json::Value {
        let bytes = body.collect().await.expect("collect body").to_bytes();
        serde_json::from_slice(&bytes).expect("parse JSON")
    }

    // ── GET /health ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn health_returns_ok() {
        let pool = test_pool().await;
        let app = build_app(test_state(pool));
        let response = app
            .oneshot(Request::builder().uri("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    // ── GET /repos ───────────────────────────────────────────────────────────

    #[tokio::test]
    async fn repos_returns_200_with_empty_list() {
        let pool = test_pool().await;
        let app = build_app(test_state(pool));
        let response = app
            .oneshot(Request::builder().uri("/repos").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let json = body_json(response.into_body()).await;
        assert_eq!(json["success"], true);
        assert!(json["data"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn repos_returns_seeded_repo() {
        let pool = test_pool().await;
        // Seed a repo directly
        sqlx::query("INSERT INTO repos (id, url, first_seen) VALUES ('r1', 'https://github.com/test/repo', '2026-01-01T00:00:00Z')")
            .execute(&pool)
            .await
            .unwrap();
        let app = build_app(test_state(pool));
        let response = app
            .oneshot(Request::builder().uri("/repos").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let json = body_json(response.into_body()).await;
        let repos = json["data"].as_array().unwrap();
        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0]["url"], "https://github.com/test/repo");
    }

    // ── GET /repos/:id/overview ──────────────────────────────────────────────

    #[tokio::test]
    async fn repo_overview_returns_404_for_unknown_repo() {
        let pool = test_pool().await;
        let app = build_app(test_state(pool));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/repos/no-such-repo/overview")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn repo_overview_returns_200_with_correct_shape() {
        let pool = test_pool().await;
        let repo_id = "test-repo-1";
        let now = "2026-03-15T10:00:00Z";
        let grade_id = "grade-1";
        let model_id = "model-v2.0.0";

        sqlx::query("INSERT INTO repos (id, url, first_seen) VALUES (?1, 'https://github.com/test/r', ?2)")
            .bind(repo_id)
            .bind(now)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO scans (id, repo_id, scanned_at, duration_ms, risk_score, composite_maturity, maturity_grade, raw_report, raw_maturity) VALUES (?1, ?2, ?3, 100, 5, 75, 'GOLD', '{}', '{}')")
            .bind("scan-1")
            .bind(repo_id)
            .bind(now)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT OR IGNORE INTO grade_models (id, version, description, created_at) VALUES (?1, '2.0.0', 'test', ?2)")
            .bind(model_id)
            .bind(now)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO scan_grades (id, scan_id, model_id, composite, grade, created_at) VALUES (?1, 'scan-1', ?2, 75, 'GOLD', ?3)")
            .bind(grade_id)
            .bind(model_id)
            .bind(now)
            .execute(&pool)
            .await
            .unwrap();

        let app = build_app(test_state(pool));
        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/repos/{repo_id}/overview"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let json = body_json(response.into_body()).await;
        assert_eq!(json["success"], true);
        let data = &json["data"];
        assert_eq!(data["repo_id"], repo_id);
        assert_eq!(data["composite"], 75);
        assert_eq!(data["grade"], "GOLD");
        assert!(data["dimensions"].is_array());
        assert!(data["top_blockers"].is_array());
        assert!(data["confidence"].is_number());
    }

    // ── GET /repos/:id/trends ────────────────────────────────────────────────

    #[tokio::test]
    async fn repo_trends_returns_200_empty_for_unknown_repo() {
        let pool = test_pool().await;
        let app = build_app(test_state(pool));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/repos/no-such-repo/trends?window=5")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let json = body_json(response.into_body()).await;
        assert_eq!(json["success"], true);
        assert!(json["data"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn repo_trends_returns_correct_point_shape() {
        let pool = test_pool().await;
        let repo_id = "trends-repo";
        let now = "2026-03-15T10:00:00Z";
        let model_id = "model-v2.0.0";

        sqlx::query("INSERT INTO repos (id, url, first_seen) VALUES (?1, 'https://github.com/test/r2', ?2)")
            .bind(repo_id)
            .bind(now)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO scans (id, repo_id, scanned_at, duration_ms, risk_score, composite_maturity, maturity_grade, raw_report, raw_maturity) VALUES ('scan-t1', ?1, ?2, 100, 5, 80, 'PLATINUM', '{}', '{}')")
            .bind(repo_id)
            .bind(now)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT OR IGNORE INTO grade_models (id, version, description, created_at) VALUES (?1, '2.0.0', 'test', ?2)")
            .bind(model_id)
            .bind(now)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO scan_grades (id, scan_id, model_id, composite, grade, created_at) VALUES ('grade-t1', 'scan-t1', ?1, 80, 'PLATINUM', ?2)")
            .bind(model_id)
            .bind(now)
            .execute(&pool)
            .await
            .unwrap();

        let app = build_app(test_state(pool));
        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/repos/{repo_id}/trends?window=10"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let json = body_json(response.into_body()).await;
        assert_eq!(json["success"], true);
        let points = json["data"].as_array().unwrap();
        assert_eq!(points.len(), 1);
        assert_eq!(points[0]["composite"], 80);
        assert_eq!(points[0]["grade"], "PLATINUM");
        assert_eq!(points[0]["scanned_at"], now);
        assert!(points[0]["dimensions"].is_object());
    }
    // ── GET /scans/:id/decision-graph ────────────────────────────────────────

    #[tokio::test]
    async fn decision_graph_returns_404_for_unknown_scan() {
        let pool = test_pool().await;
        let app = build_app(test_state(pool));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/scans/no-such-scan/decision-graph")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn decision_graph_returns_200_with_nodes_and_edges() {
        use rusty_venture_actions::repo::maturity::{
            DimensionScore, MaturityDimension, MaturityGrade, MaturityScore, MaturitySignal,
        };
        let pool = test_pool().await;
        let now = "2026-04-01T00:00:00Z";

        // Seed a minimal repo + scan.
        sqlx::query("INSERT INTO repos (id, url, first_seen) VALUES ('r-dg', 'https://github.com/test/dg', ?1)")
            .bind(now)
            .execute(&pool)
            .await
            .unwrap();

        let maturity = MaturityScore {
            composite: 72,
            grade: MaturityGrade::Gold,
            dimensions: vec![DimensionScore {
                dimension: MaturityDimension::Security,
                score: 80,
                signals: vec![
                    MaturitySignal {
                        name: "no_critical_cves".to_string(),
                        description: "No critical CVEs".to_string(),
                        passed: true,
                        points: 40,
                        detail: None,
                    },
                    MaturitySignal {
                        name: "has_security_policy".to_string(),
                        description: "Has SECURITY.md".to_string(),
                        passed: false,
                        points: 20,
                        detail: Some("Missing SECURITY.md".to_string()),
                    },
                ],
            }],
        };
        let raw_maturity = serde_json::to_string(&maturity).unwrap();

        // Seed scans + decision_graphs rows.
        sqlx::query(
            "INSERT INTO scans (id, repo_id, scanned_at, duration_ms, risk_score, \
             composite_maturity, maturity_grade, raw_report, raw_maturity) \
             VALUES ('scan-dg-1', 'r-dg', ?1, 10, 0, 72, 'GOLD', '{}', ?2)",
        )
        .bind(now)
        .bind(&raw_maturity)
        .execute(&pool)
        .await
        .unwrap();

        let dg_id = uuid::Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO decision_graphs (id, scan_id, graph_json, created_at) VALUES (?1, 'scan-dg-1', ?2, ?3)",
        )
        .bind(&dg_id)
        .bind(&raw_maturity)
        .bind(now)
        .execute(&pool)
        .await
        .unwrap();

        let app = build_app(test_state(pool));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/scans/scan-dg-1/decision-graph")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let json = body_json(response.into_body()).await;
        assert_eq!(json["success"], true);
        let data = &json["data"];
        // Nodes array must contain root + 1 dim + 2 sigs
        let nodes = data["nodes"].as_array().unwrap();
        assert_eq!(nodes.len(), 4, "expected 4 nodes (root + 1 dim + 2 sigs)");
        assert!(nodes.iter().any(|n| n["id"] == "root"), "root node missing");
        assert!(
            nodes.iter().any(|n| n["id"] == "dim:Security"),
            "dim:Security node missing"
        );
        // Edges: 1 root→dim + 2 dim→sig = 3
        let edges = data["edges"].as_array().unwrap();
        assert_eq!(edges.len(), 3, "expected 3 edges");
    }

    // ── GET /scans/:id/decision-graph — regression: corrupt blob ─────────────

    #[tokio::test]
    async fn decision_graph_returns_500_for_corrupt_blob() {
        let pool = test_pool().await;
        let now = "2026-04-01T00:00:00Z";

        sqlx::query("INSERT INTO repos (id, url, first_seen) VALUES ('r-corrupt', 'https://github.com/test/corrupt', ?1)")
            .bind(now)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO scans (id, repo_id, scanned_at, duration_ms, risk_score, \
             composite_maturity, maturity_grade, raw_report, raw_maturity) \
             VALUES ('scan-corrupt', 'r-corrupt', ?1, 10, 0, 50, 'SILVER', '{}', '{}')",
        )
        .bind(now)
        .execute(&pool)
        .await
        .unwrap();

        // Store deliberately malformed JSON as the graph blob.
        sqlx::query(
            "INSERT INTO decision_graphs (id, scan_id, graph_json, created_at) VALUES ('dg-corrupt-1', 'scan-corrupt', 'NOT_VALID_JSON', ?1)",
        )
        .bind(now)
        .execute(&pool)
        .await
        .unwrap();

        let app = build_app(test_state(pool));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/scans/scan-corrupt/decision-graph")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let json = body_json(response.into_body()).await;
        assert_eq!(json["success"], false);
        assert!(
            json["error"].as_str().unwrap().contains("Corrupted"),
            "error message must mention corrupt blob"
        );
    }

    // ── GET /scans — filtered by repo ────────────────────────────────────────

    #[tokio::test]
    async fn scans_filtered_by_repo_returns_only_matching() {
        let pool = test_pool().await;
        let now = "2026-04-01T00:00:00Z";

        // Seed two repos, each with one scan.
        for (repo_id, url, scan_id) in [
            ("repo-a", "https://github.com/test/repo-a", "scan-a"),
            ("repo-b", "https://github.com/test/repo-b", "scan-b"),
        ] {
            sqlx::query("INSERT INTO repos (id, url, first_seen) VALUES (?1, ?2, ?3)")
                .bind(repo_id)
                .bind(url)
                .bind(now)
                .execute(&pool)
                .await
                .unwrap();
            sqlx::query(
                "INSERT INTO scans (id, repo_id, scanned_at, duration_ms, risk_score, \
                 composite_maturity, maturity_grade, raw_report, raw_maturity) \
                 VALUES (?1, ?2, ?3, 100, 10, 70, 'GOLD', '{}', '{}')",
            )
            .bind(scan_id)
            .bind(repo_id)
            .bind(now)
            .execute(&pool)
            .await
            .unwrap();
        }

        let app = build_app(test_state(pool));

        // Filter to repo-a only.
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/scans?repo=https%3A%2F%2Fgithub.com%2Ftest%2Frepo-a")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let json = body_json(response.into_body()).await;
        let scans = json["data"].as_array().unwrap();
        assert_eq!(scans.len(), 1, "filter must return only 1 scan");
        assert_eq!(scans[0]["repo_url"], "https://github.com/test/repo-a");
    }

    // ── GET /scans/:id — scan detail ─────────────────────────────────────────

    #[tokio::test]
    async fn scan_detail_returns_404_for_unknown() {
        let pool = test_pool().await;
        let app = build_app(test_state(pool));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/scans/no-such-scan-id")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn scan_detail_returns_correct_shape() {
        let pool = test_pool().await;
        let now = "2026-04-02T00:00:00Z";

        sqlx::query("INSERT INTO repos (id, url, first_seen) VALUES ('r-detail', 'https://github.com/test/detail', ?1)")
            .bind(now)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO scans (id, repo_id, scanned_at, duration_ms, risk_score, \
             composite_maturity, maturity_grade, raw_report, raw_maturity) \
             VALUES ('scan-detail-1', 'r-detail', ?1, 200, 3, 85, 'PLATINUM', '{}', '{}')",
        )
        .bind(now)
        .execute(&pool)
        .await
        .unwrap();

        let app = build_app(test_state(pool));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/scans/scan-detail-1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let json = body_json(response.into_body()).await;
        assert_eq!(json["success"], true);
        let data = &json["data"];
        assert_eq!(data["id"], "scan-detail-1");
        assert_eq!(data["maturity_grade"], "PLATINUM");
        assert_eq!(data["composite_maturity"], 85);
    }

    // ── GET /health — content regression ─────────────────────────────────────

    #[tokio::test]
    async fn health_response_contains_version_field() {
        let pool = test_pool().await;
        let app = build_app(test_state(pool));
        let response = app
            .oneshot(Request::builder().uri("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let json = body_json(response.into_body()).await;
        assert_eq!(json["status"], "ok");
        assert!(
            json["version"].as_str().is_some(),
            "health response must include a 'version' field"
        );
    }
}
