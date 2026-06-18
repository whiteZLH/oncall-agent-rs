use crate::{
    config::AppConfig,
    http::routes::{chat, health, incidents, metrics},
    services::{
        chat_service::ChatService, incident_service::IncidentService,
        session_manager::SessionManager,
    },
    state::AppState,
};
use axum::{
    extract::Request,
    http::{header::HeaderValue, HeaderName, StatusCode},
    middleware::{self, Next},
    response::Response,
    Router,
};
use std::sync::Arc;
use tower_http::{
    cors::{Any, CorsLayer},
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer},
    timeout::TimeoutLayer,
    trace::TraceLayer,
};

pub const REQUEST_ID_HEADER: &str = "x-request-id";

pub fn build_router(config: AppConfig) -> Router {
    let state = Arc::new(AppState::new(
        ChatService::new(&config),
        SessionManager::new(&config),
        IncidentService::new(),
    ));
    let request_id_header = HeaderName::from_static(REQUEST_ID_HEADER);
    let cors = if config.allowed_origin == "*" {
        CorsLayer::new().allow_origin(Any)
    } else {
        let origin = HeaderValue::from_str(&config.allowed_origin)
            .unwrap_or_else(|error| panic!("APP_ALLOWED_ORIGIN 配置不合法: {}", error));
        CorsLayer::new().allow_origin(origin)
    };

    // 子路由先声明它们需要 Arc<AppState>，这里统一注入真正的共享状态。
    Router::<Arc<AppState>>::new()
        .merge(health::router())
        .merge(metrics::router())
        .merge(chat::router())
        .merge(incidents::router())
        .layer(PropagateRequestIdLayer::new(request_id_header.clone()))
        .layer(SetRequestIdLayer::new(
            request_id_header.clone(),
            MakeRequestUuid,
        ))
        .layer(middleware::from_fn(attach_request_id))
        .layer(TraceLayer::new_for_http())
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            config.request_timeout,
        ))
        .layer(cors)
        .with_state(state)
}

async fn attach_request_id(mut request: Request, next: Next) -> Response {
    let request_id = request
        .headers()
        .get(REQUEST_ID_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned);
    request.extensions_mut().insert(request_id);
    next.run(request).await
}
