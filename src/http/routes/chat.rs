use crate::{
    error::AppError,
    http::dto::{ChatResponse, MessageEnvelope},
    state::AppState,
};
use axum::{extract::State, routing::post, Json, Router};
use std::sync::Arc;

pub fn router() -> Router<Arc<AppState>> {
    Router::new().route("/api/chat", post(chat))
}

async fn chat(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<MessageEnvelope>,
) -> Result<Json<ChatResponse>, AppError> {
    let response = state.chat_service.reply(payload)?;
    Ok(Json(response))
}
