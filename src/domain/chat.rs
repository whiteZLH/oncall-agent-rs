use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatSessionRecord {
    pub session_id: String,
    pub create_time: i64,
    pub update_time: i64,
    pub message_history: Vec<ChatMessage>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatSessionSummary {
    pub session_id: String,
    pub title: String,
    pub message_pair_count: usize,
    pub create_time: i64,
    pub update_time: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfoResponse {
    pub session_id: String,
    pub message_pair_count: usize,
    pub create_time: i64,
}
