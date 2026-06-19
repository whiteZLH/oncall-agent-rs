use crate::{
    domain::chat::{ChatSessionRecord, ChatSessionSummary, SessionInfoResponse},
    http::dto::{ApiResponse, ChatRequest, ChatResponse, ClearRequest, SseMessage},
    services::{chat_service::ChatStreamEvent, session_manager::SessionInfo},
    state::AppState,
};
use axum::{
    extract::{Path, State},
    http::{header, HeaderValue},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
    routing::{get, post},
    Json, Router,
};
use futures_util::{Stream, StreamExt};
use std::{convert::Infallible, sync::Arc, time::Duration};
use tracing::{info, warn};

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/chat", post(chat))
        .route("/api/chat_stream", post(chat_stream))
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

async fn chat_stream(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<ChatRequest>,
) -> Response {
    let Some(question) = payload.question.as_deref().map(str::trim) else {
        return sse_response(async_stream::stream! {
            yield sse_event(SseMessage::error("问题内容不能为空"));
        });
    };

    if question.is_empty() {
        return sse_response(async_stream::stream! {
            yield sse_event(SseMessage::error("问题内容不能为空"));
        });
    }

    let question = question.to_string();
    let requested_session_id = payload.id.clone();

    let stream = async_stream::stream! {
        info!(
            "收到 ReactAgent 对话请求 - SessionId: {:?}, Question: {}",
            requested_session_id,
            question
        );

        let mut session = state
            .session_manager
            .get_or_create_session(requested_session_id.as_deref());
        let history = session.get_history();
        info!("ReactAgent 会话历史消息对数: {}", history.len() / 2);

        state.chat_service.log_available_tools();
        info!("开始 ReactAgent 流式对话（支持自动工具调用）");

        let session_id = session.session_id().to_string();
        let private_memories = search_private_memories(&state, &question, &session).await;

        let system_prompt = state
            .chat_service
            .build_system_prompt(&history, &private_memories);

        let agent = state
            .chat_service
            .create_react_agent(state.chat_service.chat_model(), &system_prompt);

        let mut chat_stream = match state.chat_service.stream_chat(&agent, &question).await {
            Ok(stream) => stream,
            Err(error) => {
                warn!("ReactAgent 流式对话初始化失败: {}", error);
                yield sse_event(SseMessage::error(error.to_string()));
                return;
            }
        };

        let mut full_answer = String::new();
        let mut final_answer = None;

        while let Some(item) = chat_stream.next().await {
            match item {
                Ok(ChatStreamEvent::Content(chunk)) => {
                    if !chunk.is_empty() {
                        full_answer.push_str(&chunk);
                        yield sse_event(SseMessage::content(chunk));
                    }
                }
                Ok(ChatStreamEvent::Final(answer)) => {
                    final_answer = Some(answer);
                }
                Err(error) => {
                    warn!("ReactAgent 流式对话失败: {}", error);
                    yield sse_event(SseMessage::error(error.to_string()));
                    return;
                }
            }
        }

        let answer = final_answer.unwrap_or_else(|| full_answer.clone());
        if full_answer.is_empty() && !answer.is_empty() {
            yield sse_event(SseMessage::content(answer.clone()));
        }

        session.add_message(&question, &answer, &state.session_manager);
        info!(
            "已更新会话历史 - SessionId: {}, 当前消息对数: {}",
            session_id,
            session.get_message_pair_count()
        );

        yield sse_event(SseMessage::done());
    };

    sse_response(stream)
}

async fn clear_chat_history(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<ClearRequest>,
) -> Json<ApiResponse<String>> {
    let Some(session_id) = payload.id.as_deref() else {
        return Json(ApiResponse::error(500, "会话ID不能为空"));
    };

    if session_id.trim().is_empty() {
        return Json(ApiResponse::error(500, "会话ID不能为空"));
    }

    let Some(mut session) = state.session_manager.get_session(session_id) else {
        return Json(ApiResponse::error(500, "会话不存在"));
    };

    session.clear_history(&state.session_manager);
    Json(ApiResponse::success_message("会话历史已清空"))
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

fn sse_response<S>(stream: S) -> Response
where
    S: Stream<Item = Result<Event, Infallible>> + Send + 'static,
{
    let mut response = Sse::new(stream)
        .keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(15))
                .text("keep-alive"),
        )
        .into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/event-stream;charset=UTF-8"),
    );
    response
}

fn sse_event(message: SseMessage) -> Result<Event, Infallible> {
    let data = serde_json::to_string(&message).expect("SSE message serialization should not fail");
    Ok(Event::default().event("message").data(data))
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
