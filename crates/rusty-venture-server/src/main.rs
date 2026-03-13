use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use rusty_venture_actions::repo::{run_repo_analysis, AuditReport, RepoAnalysisRequest};
use rusty_venture_store::{insert_scan, list_scans, list_scans_for_repo, open_pool, Pool};
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

#[tokio::main]
async fn main() {
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

    let app = Router::new()
        .route("/analyze", post(analyze_handler))
        .route("/scans", get(scans_handler))
        .route("/health", get(health_handler))
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let addr = std::env::var("LISTEN_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".to_string());
    info!(addr = %addr, "rusty-venture-server starting");

    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect("Failed to bind to address");

    axum::serve(listener, app)
        .await
        .expect("Server error");
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
