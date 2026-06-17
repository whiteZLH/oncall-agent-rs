use crate::{
    domain::chat::{ChatSessionRecord, ChatSessionSummary, SessionInfoResponse},
    http::dto::{ApiResponse, ChatRequest, ChatResponse, ClearRequest},
    services::chat_service::ClearResult,
    state::AppState,
};
use axum::{
    extract::{Path, State},
    routing::{get, post},
    Json, Router,
};
use std::sync::Arc;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/chat", post(chat))
        .route("/api/chat/clear", post(clear_chat_history))
        .route("/api/chat/sessions", get(list_chat_sessions))
        .route(
            "/api/chat/session/{session_id}",
            get(get_session_info).delete(delete_chat_session),
        )
        .route(
            "/api/chat/session/{session_id}/messages",
            get(get_session_messages),
        )
}

async fn chat(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<ChatRequest>,
) -> Json<ApiResponse<ChatResponse>> {
    let response = state.chat_service.reply(payload);
    Json(ApiResponse::success(response))
}

async fn clear_chat_history(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<ClearRequest>,
) -> Json<ApiResponse<String>> {
    let Some(session_id) = payload.id.as_deref() else {
        return Json(ApiResponse::error(500, "会话ID不能为空"));
    };

    match state.chat_service.clear(session_id) {
        ClearResult::Cleared => Json(ApiResponse::success_message("会话历史已清空")),
        ClearResult::MissingSessionId => Json(ApiResponse::error(500, "会话ID不能为空")),
        ClearResult::NotFound => Json(ApiResponse::error(500, "会话不存在")),
    }
}

async fn get_session_info(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
) -> Json<ApiResponse<SessionInfoResponse>> {
    match state.chat_service.session_info(&session_id) {
        Some(info) => Json(ApiResponse::success(info)),
        None => Json(ApiResponse::error(500, "会话不存在")),
    }
}

async fn list_chat_sessions(
    State(state): State<Arc<AppState>>,
) -> Json<ApiResponse<Vec<ChatSessionSummary>>> {
    Json(ApiResponse::success(state.chat_service.list_sessions()))
}

async fn get_session_messages(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
) -> Json<ApiResponse<ChatSessionRecord>> {
    match state.chat_service.session_messages(&session_id) {
        Some(record) => Json(ApiResponse::success(record)),
        None => Json(ApiResponse::error(500, "会话不存在")),
    }
}

async fn delete_chat_session(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
) -> Json<ApiResponse<String>> {
    if state.chat_service.delete_session(&session_id) {
        Json(ApiResponse::success_message("会话已删除"))
    } else {
        Json(ApiResponse::error(500, "会话不存在"))
    }
}
