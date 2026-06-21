use serde::{Deserialize, Serialize};

#[derive(Serialize)]
pub struct ApiResponse<T> {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
}

impl<T> ApiResponse<T> {
    pub fn success(data: T) -> Self {
        Self {
            code: 200,
            message: "success".to_string(),
            data: Some(data),
        }
    }

    pub fn success_message(message: impl Into<String>) -> Self {
        Self {
            code: 200,
            message: message.into(),
            data: None,
        }
    }

    pub fn error(code: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }
}

#[derive(Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
}

#[derive(Serialize)]
pub struct ReadinessResponse {
    pub status: &'static str,
}

#[derive(Serialize)]
pub struct ApiErrorResponse {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ChatRequest {
    #[serde(rename = "Id", alias = "id", alias = "ID")]
    pub id: Option<String>,
    #[serde(rename = "Question", alias = "question", alias = "QUESTION")]
    pub question: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ClearRequest {
    #[serde(rename = "Id", alias = "id", alias = "ID")]
    pub id: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct AiOpsRequest {
    #[serde(
        rename = "alertContext",
        alias = "alert_context",
        alias = "alertcontext"
    )]
    pub alert_context: Option<String>,
    #[serde(rename = "alertId", alias = "alert_id", alias = "alertid")]
    pub alert_id: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatResponse {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

impl ChatResponse {
    pub fn success(answer: impl Into<String>) -> Self {
        Self {
            success: true,
            answer: Some(answer.into()),
            error_message: None,
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self {
            success: false,
            answer: None,
            error_message: Some(message.into()),
        }
    }
}

#[derive(Serialize)]
pub struct SseMessage {
    #[serde(rename = "type")]
    pub message_type: &'static str,
    pub data: Option<String>,
}

impl SseMessage {
    pub fn content(data: impl Into<String>) -> Self {
        Self {
            message_type: "content",
            data: Some(data.into()),
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self {
            message_type: "error",
            data: Some(message.into()),
        }
    }

    pub fn done() -> Self {
        Self {
            message_type: "done",
            data: None,
        }
    }
}
