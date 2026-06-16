use crate::{
    error::AppError,
    models::{ChatResponse, MessageEnvelope},
    state::AppState,
};
use axum::{extract::State, routing::post, Json, Router};
use std::sync::Arc;

pub fn router() -> Router<Arc<AppState>> {
    Router::new().route("/api/chat", post(chat))
}

// 这里显式写 Router<Arc<AppState>>，是因为这个 handler 需要从 axum State 里拿到共享 AppState。
async fn chat(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<MessageEnvelope>,
) -> Result<Json<ChatResponse>, AppError> {
    let response = state.chat_service.reply(payload)?;
    Ok(Json(response))
}
