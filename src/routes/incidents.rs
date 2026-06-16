use crate::{models::IncidentSummary, state::AppState};
use axum::{extract::State, routing::get, Json, Router};
use std::sync::Arc;

pub fn router() -> Router<Arc<AppState>> {
    Router::new().route("/api/incidents", get(list_incidents))
}

// 和 chat 一样，这个路由依赖共享状态，所以 router 也必须带上 Arc<AppState>。
async fn list_incidents(State(state): State<Arc<AppState>>) -> Json<Vec<IncidentSummary>> {
    Json(state.incident_service.list())
}
