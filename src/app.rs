use crate::{
    config::AppConfig,
    routes::{chat, health, incidents, metrics},
    services::{chat_service::ChatService, incident_service::IncidentService},
    state::AppState,
};
use axum::{
    http::{header::HeaderValue, StatusCode},
    Router,
};
use std::sync::Arc;
use tower_http::{
    cors::{Any, CorsLayer},
    timeout::TimeoutLayer,
    trace::TraceLayer,
};

pub fn build_router(config: AppConfig) -> Router {
    let state = Arc::new(AppState::new(ChatService::new(), IncidentService::new()));
    let cors = if config.allowed_origin == "*" {
        CorsLayer::new().allow_origin(Any)
    } else {
        let origin = HeaderValue::from_str(&config.allowed_origin)
            .unwrap_or_else(|error| panic!("invalid APP_ALLOWED_ORIGIN: {}", error));
        CorsLayer::new().allow_origin(origin)
    };

    // 子路由先声明它们需要 Arc<AppState>，这里统一注入真正的共享状态。
    Router::<Arc<AppState>>::new()
        .merge(health::router())
        .merge(metrics::router())
        .merge(chat::router())
        .merge(incidents::router())
        .layer(TraceLayer::new_for_http())
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            config.request_timeout,
        ))
        .layer(cors)
        .with_state(state)
}
