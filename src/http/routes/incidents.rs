use crate::{
    domain::{
        diagnosis::DiagnosisRun,
        incident::{ArchiveResult, IncidentRecord, IncidentSummary, SearchResult},
    },
    http::dto::ApiResponse,
    state::AppState,
};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use std::sync::Arc;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/incidents", get(list_incidents))
        .route("/api/incidents/{incident_id}", get(get_incident))
        .route("/api/incidents/{incident_id}/runs", get(get_diagnosis_runs))
        .route(
            "/api/incidents/{incident_id}/diagnose",
            post(diagnose_incident),
        )
        .route(
            "/api/incidents/{incident_id}/archive-case",
            post(archive_case),
        )
        .route(
            "/api/incidents/{incident_id}/similar-cases",
            get(similar_cases),
        )
}

async fn list_incidents(
    State(state): State<Arc<AppState>>,
) -> Json<ApiResponse<Vec<IncidentSummary>>> {
    Json(ApiResponse::success(state.incident_service.list()))
}

async fn get_incident(
    State(state): State<Arc<AppState>>,
    Path(incident_id): Path<String>,
) -> (StatusCode, Json<ApiResponse<IncidentRecord>>) {
    match state.incident_service.get(&incident_id) {
        Some(record) => (StatusCode::OK, Json(ApiResponse::success(record))),
        None => (
            StatusCode::NOT_FOUND,
            Json(ApiResponse::error(404, "事故不存在")),
        ),
    }
}

async fn get_diagnosis_runs(
    State(state): State<Arc<AppState>>,
    Path(incident_id): Path<String>,
) -> (StatusCode, Json<ApiResponse<Vec<DiagnosisRun>>>) {
    match state.incident_service.runs(&incident_id) {
        Some(runs) => (StatusCode::OK, Json(ApiResponse::success(runs))),
        None => (
            StatusCode::NOT_FOUND,
            Json(ApiResponse::error(404, "事故不存在")),
        ),
    }
}

async fn diagnose_incident(
    State(state): State<Arc<AppState>>,
    Path(incident_id): Path<String>,
) -> (StatusCode, Json<ApiResponse<DiagnosisRun>>) {
    match state.incident_service.diagnose(&incident_id) {
        Some(run) => (StatusCode::OK, Json(ApiResponse::success(run))),
        None => (
            StatusCode::NOT_FOUND,
            Json(ApiResponse::error(404, "事故不存在")),
        ),
    }
}

async fn archive_case(
    State(state): State<Arc<AppState>>,
    Path(incident_id): Path<String>,
) -> Json<ApiResponse<ArchiveResult>> {
    Json(ApiResponse::success(
        state.incident_service.archive_case(&incident_id),
    ))
}

async fn similar_cases(
    State(state): State<Arc<AppState>>,
    Path(incident_id): Path<String>,
    Query(params): Query<SimilarCasesQuery>,
) -> Json<ApiResponse<Vec<SearchResult>>> {
    Json(ApiResponse::success(
        state
            .incident_service
            .similar_cases(&incident_id, params.top_k.unwrap_or(3)),
    ))
}

#[derive(Deserialize)]
struct SimilarCasesQuery {
    #[serde(rename = "topK", alias = "top_k")]
    top_k: Option<usize>,
}
