use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use oncall_agent_rs::{app, config::AppConfig};
use tower::util::ServiceExt;

fn test_config() -> AppConfig {
    AppConfig {
        host: "127.0.0.1".parse().expect("valid loopback address"),
        port: 3000,
        allowed_origin: "*".to_string(),
        request_timeout: std::time::Duration::from_secs(30),
        log_filter: "info".to_string(),
    }
}

#[tokio::test]
async fn health_endpoint_returns_ok() {
    let app = app::build_router(test_config());

    let response = app
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    assert!(response.headers().contains_key("x-request-id"));
}

#[tokio::test]
async fn chat_endpoint_rejects_blank_message() {
    let app = app::build_router(test_config());

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/chat")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"message":"   "}"#))
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let body = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let body_text = String::from_utf8(body.to_vec()).expect("utf8 body");
    assert!(body_text.contains("\"code\":\"bad_request\""));
    assert!(body_text.contains("message must not be empty"));
}

#[tokio::test]
async fn incidents_endpoint_returns_seed_data() {
    let app = app::build_router(test_config());

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/incidents")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);

    let body = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let body_text = String::from_utf8(body.to_vec()).expect("utf8 body");
    assert!(body_text.contains("INC-1001"));
    assert!(body_text.contains("API error rate is elevated"));
}
