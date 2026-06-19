use crate::{domain::rag::SearchTrace, http::dto::ApiResponse, state::AppState};
use axum::{
    extract::{Query, State},
    http::StatusCode,
    routing::get,
    Json, Router,
};
use serde::Deserialize;
use std::sync::Arc;

const MIN_TOP_K: usize = 1;
const MAX_TOP_K: usize = 20;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/knowledge/search", get(search_knowledge))
        .route("/api/knowledge/index-tasks", get(list_index_tasks))
}

async fn search_knowledge(
    State(state): State<Arc<AppState>>,
    Query(params): Query<SearchQuery>,
) -> (StatusCode, Json<ApiResponse<SearchTrace>>) {
    let query = params.query.trim();
    if query.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse::error(400, "query 不能为空")),
        );
    }
    let safe_top_k = params.top_k.unwrap_or(5).clamp(MIN_TOP_K, MAX_TOP_K);
    match state
        .vector_search_service
        .explain_similar_documents(query, safe_top_k)
        .await
    {
        Ok(trace) => (StatusCode::OK, Json(ApiResponse::success(trace))),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse::error(500, error.to_string())),
        ),
    }
}

async fn list_index_tasks(
    State(state): State<Arc<AppState>>,
) -> Json<ApiResponse<Vec<crate::domain::rag::IndexTaskStatus>>> {
    Json(ApiResponse::success(
        state.index_task_status_service.list_statuses(),
    ))
}

#[derive(Deserialize)]
struct SearchQuery {
    query: String,
    #[serde(rename = "topK", alias = "top_k")]
    top_k: Option<usize>,
}
