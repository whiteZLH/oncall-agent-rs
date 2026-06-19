use crate::{
    domain::chat::{ChatSessionRecord, ChatSessionSummary, SessionInfoResponse},
    http::dto::{ApiResponse, ChatRequest, ChatResponse, ClearRequest},
    services::session_manager::{ClearResult, SessionInfo},
    state::AppState,
};
use axum::{
    extract::{Path, State},
    routing::{get, post},
    Json, Router,
};
use std::sync::Arc;
use tracing::{info, warn};

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

    let mut session = state
        .session_manager
        .get_or_create_session(payload.id.as_deref());
    let history = session.get_history();
    info!("会话历史消息对数: {}", history.len() / 2);

    state.chat_service.log_available_tools();

    info!("开始 ReactAgent 对话（支持自动工具调用）");

    let session_id = session.session_id().to_string();
    let private_memories = search_private_memories(&state, question, &session).await;

    let system_prompt = state
        .chat_service
        .build_system_prompt(&history, &private_memories);

    let agent = state
        .chat_service
        .create_react_agent(state.chat_service.chat_model(), &system_prompt);

    let answer = match state.chat_service.execute_chat(&agent, question).await {
        Ok(answer) => answer,
        Err(error) => {
            return Json(ApiResponse::success(ChatResponse::error(error.to_string())));
        }
    };

    session.add_message(question, &answer, &state.session_manager);
    info!(
        "已更新会话历史 - SessionId: {}, 当前消息对数: {}",
        session_id,
        session.get_message_pair_count()
    );

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

async fn search_private_memories(
    state: &Arc<AppState>,
    question: &str,
    session: &SessionInfo,
) -> Vec<crate::domain::memory::PrivateMemorySearchResult> {
    if session.session_id().trim().is_empty() {
        return Vec::new();
    }

    if !state.config.private_memory_recall_enabled {
        return Vec::new();
    }

    let memory_top_k = state.config.private_memory_recall_top_k.max(1);

    match state
        .vector_search_service
        .search_session_memories(question, session.session_id(), memory_top_k)
        .await
    {
        Ok(results) => results,
        Err(error) => {
            warn!("检索私人长期记忆失败，继续普通对话: {}", error);
            Vec::new()
        }
    }
}
