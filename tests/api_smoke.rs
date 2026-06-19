use axum::{
    body::Body,
    http::{Request, StatusCode},
    response::Response,
};
use http_body_util::BodyExt;
use oncall_agent_rs::{app, config::AppConfig};
use serde_json::Value;
use tower::util::ServiceExt;

fn test_config() -> AppConfig {
    AppConfig {
        host: "127.0.0.1".parse().expect("合法的本地回环地址"),
        port: 3000,
        allowed_origin: "*".to_string(),
        request_timeout: std::time::Duration::from_secs(30),
        log_filter: "info".to_string(),
        redis_url: None,
        chat_history_path: "./target/test-chat-history".to_string(),
        session_ttl_secs: 3600,
        dashscope_api_key: None,
        dashscope_base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1".to_string(),
        dashscope_api_base_url: "https://dashscope.aliyuncs.com/api/v1".to_string(),
        dashscope_chat_model: "qwen-plus".to_string(),
        chat_agent_max_turns: 6,
        dashscope_embedding_model: "text-embedding-v4".to_string(),
        dashscope_rerank_model: "gte-rerank".to_string(),
        dashscope_rerank_url:
            "https://dashscope.aliyuncs.com/api/v1/services/rerank/text-rerank/text-rerank"
                .to_string(),
        milvus_host: "localhost".to_string(),
        milvus_port: 19530,
        milvus_username: String::new(),
        milvus_password: String::new(),
        milvus_database: "default".to_string(),
        milvus_timeout_ms: 10_000,
        rag_candidate_k: 10,
        rag_search_ef: 64,
        upload_path: "./target/uploads".to_string(),
        upload_allowed_extensions: vec!["txt".to_string(), "md".to_string()],
        document_chunk_max_size: 800,
        document_chunk_overlap: 100,
        private_memory_recall_enabled: true,
        private_memory_recall_top_k: 3,
        private_memory_store_path: "./target/test-private-memories".to_string(),
    }
}

async fn body_json(response: Response) -> Value {
    let body = response
        .into_body()
        .collect()
        .await
        .expect("读取响应体失败")
        .to_bytes();
    serde_json::from_slice(&body).expect("响应体不是合法的 JSON")
}

#[tokio::test]
async fn health_endpoint_returns_ok() {
    let app = app::build_router(test_config());

    let response = app
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .expect("构造请求失败"),
        )
        .await
        .expect("执行请求失败");

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
                .body(Body::from(r#"{"Id":"session-1","Question":"   "}"#))
                .expect("构造请求失败"),
        )
        .await
        .expect("执行请求失败");

    assert_eq!(response.status(), StatusCode::OK);

    let body = body_json(response).await;
    assert_eq!(body["code"], 200);
    assert_eq!(body["data"]["success"], false);
    assert_eq!(body["data"]["errorMessage"], "问题内容不能为空");
}

#[tokio::test]
async fn chat_endpoint_accepts_java_style_payload() {
    let app = app::build_router(test_config());

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/chat")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"Id":"session-1","Question":"继续上次的话题"}"#,
                ))
                .expect("构造请求失败"),
        )
        .await
        .expect("执行请求失败");

    assert_eq!(response.status(), StatusCode::OK);

    let body = body_json(response).await;
    assert_eq!(body["code"], 200);
    assert_eq!(body["message"], "success");
    assert_eq!(body["data"]["success"], true);
    assert!(body["data"]["answer"]
        .as_str()
        .expect("响应缺少答案字段")
        .contains("继续上次的话题"));
}

#[tokio::test]
async fn chat_session_endpoints_match_java_contract() {
    let app = app::build_router(test_config());

    let list_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/chat/sessions")
                .body(Body::empty())
                .expect("构造请求失败"),
        )
        .await
        .expect("执行请求失败");
    assert_eq!(list_response.status(), StatusCode::OK);
    let list_body = body_json(list_response).await;
    assert_eq!(list_body["data"][0]["sessionId"], "session-1");
    assert_eq!(list_body["data"][0]["messagePairCount"], 1);

    let info_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/chat/session/session-1")
                .body(Body::empty())
                .expect("构造请求失败"),
        )
        .await
        .expect("执行请求失败");
    let info_body = body_json(info_response).await;
    assert_eq!(info_body["data"]["sessionId"], "session-1");

    let messages_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/chat/session/session-1/messages")
                .body(Body::empty())
                .expect("构造请求失败"),
        )
        .await
        .expect("执行请求失败");
    let messages_body = body_json(messages_response).await;
    assert_eq!(messages_body["data"]["messageHistory"][0]["role"], "user");

    let clear_response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/chat/clear")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"Id":"session-1"}"#))
                .expect("构造请求失败"),
        )
        .await
        .expect("执行请求失败");
    let clear_body = body_json(clear_response).await;
    assert_eq!(clear_body["message"], "会话历史已清空");
}

#[tokio::test]
async fn chat_session_missing_paths_return_java_style_errors() {
    let app = app::build_router(test_config());

    let clear_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/chat/clear")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"Id":""}"#))
                .expect("构造请求失败"),
        )
        .await
        .expect("执行请求失败");
    assert_eq!(clear_response.status(), StatusCode::OK);
    let clear_body = body_json(clear_response).await;
    assert_eq!(clear_body["message"], "会话ID不能为空");

    let missing_response = app
        .oneshot(
            Request::builder()
                .uri("/api/chat/session/bad-id")
                .body(Body::empty())
                .expect("构造请求失败"),
        )
        .await
        .expect("执行请求失败");
    let missing_body = body_json(missing_response).await;
    assert_eq!(missing_body["message"], "会话不存在");
}

#[tokio::test]
async fn incidents_endpoint_returns_seed_data_with_java_field_names() {
    let app = app::build_router(test_config());

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/incidents")
                .body(Body::empty())
                .expect("构造请求失败"),
        )
        .await
        .expect("执行请求失败");

    assert_eq!(response.status(), StatusCode::OK);

    let body = body_json(response).await;
    assert_eq!(body["code"], 200);
    assert_eq!(body["data"][0]["id"], "incident-1");
    assert_eq!(body["data"][0]["title"], "HighCPUUsage payment-service");
    assert_eq!(body["data"][0]["latestRunStatus"], "COMPLETED");
}

#[tokio::test]
async fn incident_detail_run_and_action_endpoints_match_java_contract() {
    let app = app::build_router(test_config());

    let detail_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/incidents/incident-1")
                .body(Body::empty())
                .expect("构造请求失败"),
        )
        .await
        .expect("执行请求失败");
    assert_eq!(detail_response.status(), StatusCode::OK);
    let detail_body = body_json(detail_response).await;
    assert_eq!(detail_body["data"]["id"], "incident-1");
    assert_eq!(detail_body["data"]["diagnosisRuns"][0]["runId"], "run-1");

    let runs_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/incidents/incident-1/runs")
                .body(Body::empty())
                .expect("构造请求失败"),
        )
        .await
        .expect("执行请求失败");
    let runs_body = body_json(runs_response).await;
    assert_eq!(runs_body["data"][0]["status"], "COMPLETED");

    let similar_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/incidents/incident-1/similar-cases?topK=3")
                .body(Body::empty())
                .expect("构造请求失败"),
        )
        .await
        .expect("执行请求失败");
    let similar_body = body_json(similar_response).await;
    assert_eq!(similar_body["code"], 500);
    assert!(similar_body["message"]
        .as_str()
        .expect("错误响应应包含 message")
        .contains("DASHSCOPE_API_KEY"));

    let archive_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/incidents/incident-1/archive-case")
                .body(Body::empty())
                .expect("构造请求失败"),
        )
        .await
        .expect("执行请求失败");
    let archive_body = body_json(archive_response).await;
    assert_eq!(archive_body["code"], 500);

    let diagnose_response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/incidents/incident-1/diagnose")
                .body(Body::empty())
                .expect("构造请求失败"),
        )
        .await
        .expect("执行请求失败");
    let diagnose_body = body_json(diagnose_response).await;
    assert_eq!(diagnose_body["data"]["incidentId"], "incident-1");
    assert_eq!(diagnose_body["data"]["status"], "QUEUED");
}

#[tokio::test]
async fn missing_incident_returns_404_api_response() {
    let app = app::build_router(test_config());

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/incidents/missing")
                .body(Body::empty())
                .expect("构造请求失败"),
        )
        .await
        .expect("执行请求失败");

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let body = body_json(response).await;
    assert_eq!(body["code"], 404);
    assert_eq!(body["message"], "事故不存在");
}

#[tokio::test]
async fn knowledge_endpoints_return_java_style_contracts() {
    let app = app::build_router(test_config());

    let tasks_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/knowledge/index-tasks")
                .body(Body::empty())
                .expect("构造请求失败"),
        )
        .await
        .expect("执行请求失败");
    assert_eq!(tasks_response.status(), StatusCode::OK);
    let tasks_body = body_json(tasks_response).await;
    assert_eq!(tasks_body["code"], 200);
    assert!(tasks_body["data"].is_array());

    let blank_search_response = app
        .oneshot(
            Request::builder()
                .uri("/api/knowledge/search?query=%20%20%20&topK=99")
                .body(Body::empty())
                .expect("构造请求失败"),
        )
        .await
        .expect("执行请求失败");
    assert_eq!(blank_search_response.status(), StatusCode::BAD_REQUEST);
    let blank_search_body = body_json(blank_search_response).await;
    assert_eq!(blank_search_body["message"], "query 不能为空");
}

#[tokio::test]
async fn upload_endpoint_creates_index_task() {
    let app = app::build_router(test_config());
    let boundary = "X-ONCALL-BOUNDARY";
    let body = format!(
        "--{boundary}\r\ncontent-disposition: form-data; name=\"file\"; filename=\"runbook.md\"\r\ncontent-type: text/markdown\r\n\r\n# CPU\r\n\r\ncheck cpu\r\n--{boundary}--\r\n"
    );

    let upload_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/upload")
                .header(
                    "content-type",
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .body(Body::from(body))
                .expect("构造请求失败"),
        )
        .await
        .expect("执行请求失败");
    assert_eq!(upload_response.status(), StatusCode::OK);
    let upload_body = body_json(upload_response).await;
    assert_eq!(upload_body["data"]["fileName"], "runbook.md");
    assert_eq!(upload_body["data"]["status"], "INDEXING");
    let task_id = upload_body["data"]["taskId"]
        .as_str()
        .expect("上传响应应包含 taskId");

    let status_response = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/upload/status/{task_id}"))
                .body(Body::empty())
                .expect("构造请求失败"),
        )
        .await
        .expect("执行请求失败");
    assert_eq!(status_response.status(), StatusCode::OK);
    let status_body = body_json(status_response).await;
    assert_eq!(status_body["data"]["taskId"], task_id);
}
