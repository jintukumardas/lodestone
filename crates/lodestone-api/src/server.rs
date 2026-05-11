use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use clickhouse::Client;
use serde::Deserialize;
use tower_http::trace::TraceLayer;

use crate::{auth, queries};

#[derive(Clone)]
pub struct AppState {
    pub ch: Arc<Client>,
}

pub fn router(client: Client, api_token: String) -> Router {
    let state = AppState {
        ch: Arc::new(client),
    };
    let token = auth::ApiToken(api_token);
    Router::new()
        .route("/healthz", get(healthz))
        .route("/callers/{function_id}", get(callers))
        .route("/impacted/{mr_id}", get(impacted))
        .route("/subgraph/{node_id}", get(subgraph))
        .route("/find", get(find))
        .layer(axum::middleware::from_fn_with_state(
            token,
            auth::require_bearer,
        ))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn healthz() -> &'static str {
    "ok"
}

async fn callers(
    State(s): State<AppState>,
    Path(function_id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let rows = queries::callers_of(&s.ch, &function_id).await?;
    Ok(Json(serde_json::json!({ "callers": rows })))
}

async fn impacted(
    State(s): State<AppState>,
    Path(mr_id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let rows = queries::impacted_by(&s.ch, &mr_id).await?;
    Ok(Json(serde_json::json!({ "impacted": rows })))
}

#[derive(Debug, Deserialize)]
struct SubgraphParams {
    #[serde(default = "default_depth")]
    depth: u32,
    #[serde(default = "default_max")]
    max: usize,
}
fn default_depth() -> u32 {
    2
}
fn default_max() -> usize {
    200
}

async fn subgraph(
    State(s): State<AppState>,
    Path(node_id): Path<String>,
    Query(params): Query<SubgraphParams>,
) -> Result<Json<serde_json::Value>, AppError> {
    let g = queries::subgraph_around(&s.ch, &node_id, params.depth, params.max).await?;
    Ok(Json(serde_json::to_value(g)?))
}

#[derive(Debug, Deserialize)]
struct FindParams {
    repo: String,
    qname: String,
}

async fn find(
    State(s): State<AppState>,
    Query(params): Query<FindParams>,
) -> Result<Json<serde_json::Value>, AppError> {
    let row = queries::find_by_qname(&s.ch, &params.repo, &params.qname).await?;
    Ok(Json(serde_json::json!({ "node": row })))
}

pub struct AppError(anyhow::Error);

impl<E: Into<anyhow::Error>> From<E> for AppError {
    fn from(e: E) -> Self {
        AppError(e.into())
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        tracing::error!(error = %self.0, "request failed");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": self.0.to_string() })),
        )
            .into_response()
    }
}
