use crate::{
    http::dto::{HealthResponse, ReadinessResponse},
    state::AppState,
};
use axum::{routing::get, Json, Router};
use std::sync::Arc;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/health", get(health))
        .route("/ready", get(ready))
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

async fn ready() -> Json<ReadinessResponse> {
    Json(ReadinessResponse { status: "ready" })
}
