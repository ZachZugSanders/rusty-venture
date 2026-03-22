use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::sse::{Event, KeepAlive},
    response::Sse,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use dashmap::DashMap;
use futures::StreamExt;
use rusty_venture_actions::repo::{
    maturity::MaturityScore, run_repo_analysis, AuditReport, RepoAnalysisRequest,
};
use rusty_venture_actions::{DecisionGraph, NodeSizeConfig};
use rusty_venture_store::{
    advance_repo_tier, get_decision_graph_for_scan, get_repo_overview, get_repo_tier_by_url,
    get_repo_trends, get_repo_url, get_scan, insert_scan, list_repos, list_scans,
    list_scans_for_repo, open_pool, scan_exists_for_commit, Pool,
};
use serde::{Deserialize, Serialize};
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;
use tower_http::trace::TraceLayer;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

// ── Run event: sent over SSE to the frontend ─────────────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum RunEvent {
    Log {
        level: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        step: Option<String>,
        message: String,
    },
    Done {
        #[serde(skip_serializing_if = "Option::is_none")]
        scan_id: Option<String>,
        repo_url: String,
    },
    Failed {
        error: String,
    },
}

/// Registry mapping run_id → broadcast sender for that run's event stream.
type RunRegistry = Arc<DashMap<String, broadcast::Sender<RunEvent>>>;

/// Shared application state injected into all route handlers.
#[derive(Clone)]
struct AppState {
    anthropic_api_key: Arc<String>,
    db: Arc<Pool>,
    /// Active run streams. Entries are removed ~30s after the run completes.
    runs: RunRegistry,
}

/// Request body for POST /analyze.
#[derive(Debug, Deserialize)]
struct AnalyzeRequest {
    repo_url: String,
    branch: Option<String>,
    /// When `true`, skip Docker and analyse using local `git` + LLM only.
    #[serde(default)]
    no_container: bool,
    /// When `true`, commit the clone container as a local Docker image
    /// (`rv-cache-{owner}-{repo}:latest`) right after cloning so future
    /// actions can reuse the pre-cloned state. Ignored when `no_container` is true.
    #[serde(default)]
    cache_repo_image: bool,
    /// Which maturity tier to scan (1, 2, or 3). Defaults to 1.
    /// Must be ≤ the repo's `max_unlocked_tier`; the server rejects higher values.
    /// If the repo has never been seen before, only tier 1 is permitted.
    tier: Option<u8>,
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
        .route("/repos/:id/rescan", post(rescan_handler))
        .route("/runs/:run_id/stream", get(run_stream_handler))
        .route("/scans", get(scans_handler))
        .route("/scans/:id", get(scan_detail_handler))
        .route("/repos", get(repos_handler))
        .route("/repos/:id/overview", get(repo_overview_handler))
        .route("/repos/:id/trends", get(repo_trends_handler))
        .route(
            "/scans/:id/decision-graph",
            get(scan_decision_graph_handler),
        )
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
        .with_target(true)
        .compact()
        .init();

    let api_key = std::env::var("ANTHROPIC_API_KEY")
        .expect("ANTHROPIC_API_KEY environment variable must be set");

    let database_url =
        std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite://rusty-venture.db".to_string());

    let pool = open_pool(Some(&database_url))
        .await
        .expect("Failed to open database");

    let state = AppState {
        anthropic_api_key: Arc::new(api_key),
        db: Arc::new(pool),
        runs: Arc::new(DashMap::new()),
    };

    let app = build_app(state);

    let addr = std::env::var("LISTEN_ADDR").unwrap_or_else(|_| "0.0.0.0:3002".to_string());
    info!(addr = %addr, "rusty-venture-server starting");

    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect("Failed to bind to address");

    axum::serve(listener, app).await.expect("Server error");
}

// ── SSE stream handler ────────────────────────────────────────────────────────

async fn run_stream_handler(State(state): State<AppState>, Path(run_id): Path<String>) -> Response {
    let rx = match state.runs.get(&run_id) {
        Some(entry) => entry.value().subscribe(),
        None => {
            return (
                StatusCode::NOT_FOUND,
                ApiError::new(format!("Run {run_id} not found")),
            )
                .into_response();
        }
    };

    let stream = BroadcastStream::new(rx).filter_map(|res| {
        futures::future::ready(match res {
            Ok(event) => serde_json::to_string(&event)
                .ok()
                .map(|data| Ok::<Event, Infallible>(Event::default().data(data))),
            // Lagged receiver: skip missed events
            Err(_) => None,
        })
    });

    Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}

// ── Analyze handler (fire-and-forget, returns run_id immediately) ─────────────

async fn analyze_handler(
    State(state): State<AppState>,
    Json(body): Json<AnalyzeRequest>,
) -> impl IntoResponse {
    info!(repo_url = %body.repo_url, "Received analyze request");

    // Resolve and validate the requested scan tier.
    let requested_tier = body.tier.unwrap_or(1).max(1);
    if requested_tier > 3 {
        return (
            StatusCode::BAD_REQUEST,
            ApiError::new("tier must be 1, 2, or 3"),
        )
            .into_response();
    }
    // If the repo already exists, enforce tier ≤ max_unlocked_tier.
    if requested_tier > 1 {
        match get_repo_tier_by_url(&state.db, &body.repo_url).await {
            Ok(Some((_id, max_tier))) if requested_tier as i64 > max_tier => {
                return (
                    StatusCode::BAD_REQUEST,
                    ApiError::new(format!(
                        "Tier {requested_tier} is not yet unlocked for this repo (max: {max_tier})"
                    )),
                )
                    .into_response();
            }
            Ok(None) => {
                // Repo not seen before — only tier 1 is valid.
                return (
                    StatusCode::BAD_REQUEST,
                    ApiError::new("Tier 2+ requires a completed Tier 1 scan first"),
                )
                    .into_response();
            }
            Ok(Some(_)) => {} // tier is unlocked — proceed
            Err(e) => {
                warn!(error = %e, "Failed to check repo tier");
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    ApiError::new(e.to_string()),
                )
                    .into_response();
            }
        }
    }

    // Create a broadcast channel for this run.
    let (broadcast_tx, _) = broadcast::channel::<RunEvent>(512);
    let run_id = uuid::Uuid::new_v4().to_string();

    // Register so the SSE handler can subscribe.
    state.runs.insert(run_id.clone(), broadcast_tx.clone());

    // mpsc channel: workflow → bridge task
    let (log_tx, mut log_rx) =
        tokio::sync::mpsc::unbounded_channel::<rusty_venture_core::LogLine>();

    // Bridge: convert LogLine → RunEvent::Log and broadcast.
    let bridge_tx = broadcast_tx.clone();
    let bridge_handle = tokio::spawn(async move {
        while let Some(line) = log_rx.recv().await {
            let _ = bridge_tx.send(RunEvent::Log {
                level: line.level.to_string(),
                step: line.step,
                message: line.message,
            });
        }
    });

    // Background analysis task.
    let runs = Arc::clone(&state.runs);
    let db = Arc::clone(&state.db);
    let api_key = Arc::clone(&state.anthropic_api_key);
    let run_id2 = run_id.clone();
    let repo_url = body.repo_url.clone();

    tokio::spawn(async move {
        let cache_repo_image = body.cache_repo_image && !body.no_container;
        let result = run_repo_analysis(RepoAnalysisRequest {
            repo_url: repo_url.clone(),
            branch: body.branch,
            claude_api_key: (*api_key).clone(),
            docker_socket: None,
            skip_container: body.no_container,
            log_tx: Some(log_tx),
            commit_hash: None,
            cache_repo_image,
            scan_tier: requested_tier,
        })
        .await;

        // Wait for bridge to drain any remaining log lines before sending terminal event.
        let _ = bridge_handle.await;

        match result {
            Ok(analysis) => {
                let scan_id = analysis.run_id.clone();
                let empty_audit = AuditReport::default();
                match insert_scan(
                    &db,
                    &analysis,
                    &analysis.maturity,
                    &empty_audit,
                    analysis.scan_tier,
                )
                .await
                {
                    Ok(repo_id) => {
                        // Advance tier if all signals in the scanned tier passed.
                        let all_passed = analysis
                            .maturity
                            .dimensions
                            .iter()
                            .flat_map(|d| d.signals.iter())
                            .filter(|s| s.tier == analysis.scan_tier)
                            .all(|s| s.passed);
                        if all_passed && analysis.scan_tier < 3 {
                            let next_tier = analysis.scan_tier as i64 + 1;
                            match advance_repo_tier(&db, &repo_id, next_tier).await {
                                Ok(true) => info!(repo_id = %repo_id, next_tier, "Tier advanced"),
                                Ok(false) => {}
                                Err(e) => warn!(error = %e, "Failed to advance repo tier"),
                            }
                        }
                    }
                    Err(e) => warn!(error = %e, "Failed to persist scan to database"),
                }
                let _ = broadcast_tx.send(RunEvent::Done {
                    scan_id: Some(scan_id),
                    repo_url: repo_url.clone(),
                });
            }
            Err(e) => {
                warn!(repo_url = %repo_url, error = %e, "Analysis failed");
                let _ = broadcast_tx.send(RunEvent::Failed {
                    error: e.to_string(),
                });
            }
        }

        // Keep registry entry alive briefly so late SSE subscribers can receive Done/Failed.
        tokio::time::sleep(Duration::from_secs(30)).await;
        runs.remove(&run_id2);
    });

    (
        StatusCode::ACCEPTED,
        Json(ApiResponse {
            success: true,
            data: serde_json::json!({ "run_id": run_id, "repo_url": body.repo_url }),
        }),
    )
        .into_response()
}

// ── GitHub commit-hash fetch ──────────────────────────────────────────────────

/// Fetch the latest commit SHA for the default branch of a GitHub repository
/// using only the public REST API (no auth, works for public repos).
///
/// `repo_url` can be either `https://github.com/owner/repo` or
/// `https://github.com/owner/repo.git`.
async fn fetch_github_commit_sha(repo_url: &str) -> anyhow::Result<String> {
    let path = repo_url
        .trim_start_matches("https://github.com/")
        .trim_end_matches(".git");
    let api_url = format!("https://api.github.com/repos/{path}/commits/HEAD");

    let client = reqwest::Client::new();
    let resp = client
        .get(&api_url)
        .header("User-Agent", "rusty-venture/1.0")
        .header("Accept", "application/vnd.github.v3+json")
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("GitHub API request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("GitHub API returned {status}: {body}");
    }

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to parse GitHub response: {e}"))?;

    json["sha"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow::anyhow!("GitHub response missing 'sha' field"))
}

// ── Rescan handler ────────────────────────────────────────────────────────────

/// `POST /repos/:id/rescan`
///
/// 1. Fetches the latest commit SHA from GitHub (no container needed).
/// 2. If any existing scan for this repo used that SHA → returns `{skipped: true}`.
/// 3. Otherwise starts a new background analysis and returns `{run_id, commit_sha}`.
async fn rescan_handler(
    State(state): State<AppState>,
    Path(repo_id): Path<String>,
) -> impl IntoResponse {
    // Look up the repo URL from the database.
    let repo_url = match get_repo_url(&state.db, &repo_id).await {
        Ok(Some(url)) => url,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                ApiError::new(format!("Repo {repo_id} not found")),
            )
                .into_response();
        }
        Err(e) => {
            warn!(repo_id = %repo_id, error = %e, "Failed to look up repo");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                ApiError::new(e.to_string()),
            )
                .into_response();
        }
    };

    // Fetch the current HEAD commit SHA from GitHub API.
    let commit_sha = match fetch_github_commit_sha(&repo_url).await {
        Ok(sha) => sha,
        Err(e) => {
            warn!(repo_url = %repo_url, error = %e, "Failed to fetch commit SHA from GitHub");
            return (
                StatusCode::BAD_GATEWAY,
                ApiError::new(format!("GitHub API error: {e}")),
            )
                .into_response();
        }
    };

    // Check if this commit was already scanned.
    match scan_exists_for_commit(&state.db, &repo_id, &commit_sha).await {
        Ok(Some(scan_id)) => {
            return (
                StatusCode::OK,
                Json(ApiResponse {
                    success: true,
                    data: serde_json::json!({
                        "skipped": true,
                        "commit_sha": commit_sha,
                        "existing_scan_id": scan_id,
                        "message": "Repository HEAD has not changed since last scan",
                    }),
                }),
            )
                .into_response();
        }
        Ok(None) => {} // new commit — proceed with analysis
        Err(e) => {
            warn!(error = %e, "Failed to check existing scans");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                ApiError::new(e.to_string()),
            )
                .into_response();
        }
    }

    // New commit: start a background analysis (same pattern as analyze_handler).
    info!(repo_url = %repo_url, commit_sha = %commit_sha, "Starting rescan for new commit");

    let (broadcast_tx, _) = broadcast::channel::<RunEvent>(512);
    let run_id = uuid::Uuid::new_v4().to_string();
    state.runs.insert(run_id.clone(), broadcast_tx.clone());

    let (log_tx, mut log_rx) =
        tokio::sync::mpsc::unbounded_channel::<rusty_venture_core::LogLine>();

    let bridge_tx = broadcast_tx.clone();
    let bridge_handle = tokio::spawn(async move {
        while let Some(line) = log_rx.recv().await {
            let _ = bridge_tx.send(RunEvent::Log {
                level: line.level.to_string(),
                step: line.step,
                message: line.message,
            });
        }
    });

    let runs = Arc::clone(&state.runs);
    let db = Arc::clone(&state.db);
    let api_key = Arc::clone(&state.anthropic_api_key);
    let run_id2 = run_id.clone();
    let repo_url2 = repo_url.clone();
    let sha = commit_sha.clone();

    // Rescan always runs the repo's current max unlocked tier.
    let rescan_tier = match get_repo_tier_by_url(&state.db, &repo_url).await {
        Ok(Some((_id, t))) => t as u8,
        _ => 1,
    };

    tokio::spawn(async move {
        let result = run_repo_analysis(RepoAnalysisRequest {
            repo_url: repo_url2.clone(),
            branch: None,
            claude_api_key: (*api_key).clone(),
            docker_socket: None,
            skip_container: false,
            log_tx: Some(log_tx),
            commit_hash: Some(sha),
            cache_repo_image: false,
            scan_tier: rescan_tier,
        })
        .await;

        let _ = bridge_handle.await;

        match result {
            Ok(analysis) => {
                let scan_id = analysis.run_id.clone();
                let empty_audit = AuditReport::default();
                match insert_scan(
                    &db,
                    &analysis,
                    &analysis.maturity,
                    &empty_audit,
                    analysis.scan_tier,
                )
                .await
                {
                    Ok(repo_id) => {
                        let all_passed = analysis
                            .maturity
                            .dimensions
                            .iter()
                            .flat_map(|d| d.signals.iter())
                            .filter(|s| s.tier == analysis.scan_tier)
                            .all(|s| s.passed);
                        if all_passed && analysis.scan_tier < 3 {
                            let next_tier = analysis.scan_tier as i64 + 1;
                            if let Err(e) = advance_repo_tier(&db, &repo_id, next_tier).await {
                                warn!(error = %e, "Failed to advance repo tier on rescan");
                            }
                        }
                    }
                    Err(e) => warn!(error = %e, "Failed to persist rescan to database"),
                }
                let _ = broadcast_tx.send(RunEvent::Done {
                    scan_id: Some(scan_id),
                    repo_url: repo_url2.clone(),
                });
            }
            Err(e) => {
                warn!(repo_url = %repo_url2, error = %e, "Rescan failed");
                let _ = broadcast_tx.send(RunEvent::Failed {
                    error: e.to_string(),
                });
            }
        }

        tokio::time::sleep(Duration::from_secs(30)).await;
        runs.remove(&run_id2);
    });

    (
        StatusCode::ACCEPTED,
        Json(ApiResponse {
            success: true,
            data: serde_json::json!({
                "skipped": false,
                "run_id": run_id,
                "commit_sha": commit_sha,
                "repo_url": repo_url,
            }),
        }),
    )
        .into_response()
}

// ── Existing handlers ─────────────────────────────────────────────────────────

async fn scan_decision_graph_handler(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    // 1. Look up the stored graph row.
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
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                ApiError::new(e.to_string()),
            )
                .into_response();
        }
    };

    // 2. Prefer the pre-computed DecisionGraph payload (stored since migration 003).
    //    For older rows fall back to rebuilding from the raw MaturityScore JSON.
    let graph: DecisionGraph = if let Some(ref payload) = row.graph_payload {
        match serde_json::from_str(payload) {
            Ok(g) => g,
            Err(e) => {
                warn!(scan_id = %id, error = %e, "graph_payload parse failed; rebuilding from legacy");
                // Fall through to legacy path below.
                let risk_score: Option<u8> = get_scan(&state.db, &id)
                    .await
                    .ok()
                    .flatten()
                    .map(|s| s.risk_score);
                match serde_json::from_str::<MaturityScore>(&row.graph_json) {
                    Ok(m) => {
                        DecisionGraph::from_maturity_full(&m, risk_score, NodeSizeConfig::default())
                    }
                    Err(e2) => {
                        warn!(scan_id = %id, error = %e2, "legacy graph_json parse failed");
                        return (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            ApiError::new(format!("Corrupted decision-graph data: {e2}")),
                        )
                            .into_response();
                    }
                }
            }
        }
    } else {
        // Legacy path: row predates migration 003.
        let risk_score: Option<u8> = get_scan(&state.db, &id)
            .await
            .ok()
            .flatten()
            .map(|s| s.risk_score);
        match serde_json::from_str::<MaturityScore>(&row.graph_json) {
            Ok(m) => DecisionGraph::from_maturity_full(&m, risk_score, NodeSizeConfig::default()),
            Err(e) => {
                warn!(scan_id = %id, error = %e, "Failed to parse legacy decision graph blob");
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    ApiError::new(format!("Corrupted decision-graph blob: {e}")),
                )
                    .into_response();
            }
        }
    };

    (
        StatusCode::OK,
        Json(ApiResponse {
            success: true,
            data: graph,
        }),
    )
        .into_response()
}

async fn health_handler() -> impl IntoResponse {
    Json(serde_json::json!({ "status": "ok", "version": env!("CARGO_PKG_VERSION") }))
}

async fn scan_detail_handler(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match get_scan(&state.db, &id).await {
        Ok(Some(detail)) => (
            StatusCode::OK,
            Json(ApiResponse {
                success: true,
                data: detail,
            }),
        )
            .into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            ApiError::new(format!("Scan {id} not found")),
        )
            .into_response(),
        Err(e) => {
            warn!(scan_id = %id, error = %e, "Failed to fetch scan detail");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                ApiError::new(e.to_string()),
            )
                .into_response()
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
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                ApiError::new(e.to_string()),
            )
                .into_response()
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
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                ApiError::new(e.to_string()),
            )
                .into_response()
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
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                ApiError::new(e.to_string()),
            )
                .into_response()
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
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                ApiError::new(e.to_string()),
            )
                .into_response()
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
            runs: Arc::new(DashMap::new()),
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
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
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
            .oneshot(
                Request::builder()
                    .uri("/repos")
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
    async fn repos_returns_seeded_repo() {
        let pool = test_pool().await;
        // Seed a repo directly
        sqlx::query("INSERT INTO repos (id, url, first_seen) VALUES ('r1', 'https://github.com/test/repo', '2026-01-01T00:00:00Z')")
            .execute(&pool)
            .await
            .unwrap();
        let app = build_app(test_state(pool));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/repos")
                    .body(Body::empty())
                    .unwrap(),
            )
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

        sqlx::query(
            "INSERT INTO repos (id, url, first_seen) VALUES (?1, 'https://github.com/test/r', ?2)",
        )
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

        sqlx::query(
            "INSERT INTO repos (id, url, first_seen) VALUES (?1, 'https://github.com/test/r2', ?2)",
        )
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
                        tier: 1,
                    },
                    MaturitySignal {
                        name: "has_security_policy".to_string(),
                        description: "Has SECURITY.md".to_string(),
                        passed: false,
                        points: 20,
                        detail: Some("Missing SECURITY.md".to_string()),
                        tier: 1,
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
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
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

    // ── GET /scans/:id/decision-graph — config.orbit_l1 validation ───────────

    #[tokio::test]
    async fn decision_graph_config_has_default_orbit_l1() {
        use rusty_venture_actions::repo::maturity::{
            DimensionScore, MaturityDimension, MaturityGrade, MaturityScore, MaturitySignal,
        };
        let pool = test_pool().await;
        let now = "2026-04-01T00:00:00Z";

        sqlx::query("INSERT INTO repos (id, url, first_seen) VALUES ('r-cfg', 'https://github.com/test/cfg', ?1)")
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
                signals: vec![MaturitySignal {
                    name: "no_critical_cves".to_string(),
                    description: "No critical CVEs".to_string(),
                    passed: true,
                    points: 40,
                    detail: None,
                }],
            }],
        };
        let raw_maturity = serde_json::to_string(&maturity).unwrap();

        sqlx::query(
            "INSERT INTO scans (id, repo_id, scanned_at, duration_ms, risk_score, \
             composite_maturity, maturity_grade, raw_report, raw_maturity) \
             VALUES ('scan-cfg-1', 'r-cfg', ?1, 10, 0, 72, 'GOLD', '{}', ?2)",
        )
        .bind(now)
        .bind(&raw_maturity)
        .execute(&pool)
        .await
        .unwrap();

        let dg_id = uuid::Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO decision_graphs (id, scan_id, graph_json, created_at) VALUES (?1, 'scan-cfg-1', ?2, ?3)",
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
                    .uri("/scans/scan-cfg-1/decision-graph")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let json = body_json(response.into_body()).await;
        assert_eq!(json["success"], true);

        let expected = NodeSizeConfig::default().orbit_l1 as f64;
        let actual = json["data"]["config"]["orbit_l1"]
            .as_f64()
            .expect("config.orbit_l1 must be a number");
        assert!(
            (actual - expected).abs() < 1e-4,
            "config.orbit_l1 must equal NodeSizeConfig::default().orbit_l1 ({expected}), got {actual}"
        );
    }
}
