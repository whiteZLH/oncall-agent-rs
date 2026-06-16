use crate::{domain::incident::IncidentSummary, state::AppState};
use axum::{extract::State, routing::get, Json, Router};
use std::sync::Arc;

pub fn router() -> Router<Arc<AppState>> {
    Router::new().route("/api/incidents", get(list_incidents))
}

async fn list_incidents(State(state): State<Arc<AppState>>) -> Json<Vec<IncidentSummary>> {
    Json(state.incident_service.list())
}
