use crate::{
    domain::chat::{ChatSessionRecord, ChatSessionSummary, SessionInfoResponse},
    http::dto::{ApiResponse, ChatRequest, ChatResponse, ClearRequest},
    services::session_manager::ClearResult,
    state::AppState,
};
use axum::{
    extract::{Path, State},
    routing::{get, post},
    Json, Router,
};
use std::sync::Arc;
use tracing::info;

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
    let Some(question) = payload.question.as_deref().map(str::trim) else {
        return Json(ApiResponse::success(ChatResponse::error(
            "问题内容不能为空",
        )));
    };

    if question.is_empty() {
        return Json(ApiResponse::success(ChatResponse::error(
            "问题内容不能为空",
        )));
    }

    let session = state
        .session_manager
        .get_or_create_session(payload.id.as_deref());
    let history = session.get_history();
    info!("会话历史消息对数: {}", history.len() / 2);

    state.chat_service.log_available_tools();

    let system_prompt = state.chat_service.build_system_prompt(&history);

    let session_id = session.session_id().to_string();
    let answer = match state
        .chat_service
        .execute_chat(&session_id, question, &system_prompt)
        .await
    {
        Ok(answer) => answer,
        Err(error) => {
            return Json(ApiResponse::success(ChatResponse::error(error.to_string())));
        }
    };

    state
        .session_manager
        .record_exchange(&session_id, question, &answer);

    Json(ApiResponse::success(ChatResponse::success(answer)))
}

async fn clear_chat_history(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<ClearRequest>,
) -> Json<ApiResponse<String>> {
    let Some(session_id) = payload.id.as_deref() else {
        return Json(ApiResponse::error(500, "会话ID不能为空"));
    };

    match state.session_manager.clear(session_id) {
        ClearResult::Cleared => Json(ApiResponse::success_message("会话历史已清空")),
        ClearResult::MissingSessionId => Json(ApiResponse::error(500, "会话ID不能为空")),
        ClearResult::NotFound => Json(ApiResponse::error(500, "会话不存在")),
    }
}

async fn get_session_info(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
) -> Json<ApiResponse<SessionInfoResponse>> {
    match state.session_manager.session_info(&session_id) {
        Some(info) => Json(ApiResponse::success(info)),
        None => Json(ApiResponse::error(500, "会话不存在")),
    }
}

async fn list_chat_sessions(
    State(state): State<Arc<AppState>>,
) -> Json<ApiResponse<Vec<ChatSessionSummary>>> {
    Json(ApiResponse::success(state.session_manager.list_sessions()))
}

async fn get_session_messages(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
) -> Json<ApiResponse<ChatSessionRecord>> {
    match state.session_manager.session_messages(&session_id) {
        Some(record) => Json(ApiResponse::success(record)),
        None => Json(ApiResponse::error(500, "会话不存在")),
    }
}

async fn delete_chat_session(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
) -> Json<ApiResponse<String>> {
    if state.session_manager.delete_session(&session_id) {
        Json(ApiResponse::success_message("会话已删除"))
    } else {
        Json(ApiResponse::error(500, "会话不存在"))
    }
}
