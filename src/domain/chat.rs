use serde::{Deserialize, Deserializer, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessage {
    #[serde(default, deserialize_with = "null_to_default")]
    pub role: String,
    #[serde(default, deserialize_with = "null_to_default")]
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatSessionRecord {
    #[serde(default, deserialize_with = "null_to_default")]
    pub session_id: String,
    #[serde(default, deserialize_with = "null_to_default")]
    pub create_time: i64,
    #[serde(default, deserialize_with = "null_to_default")]
    pub update_time: i64,
    #[serde(default, deserialize_with = "null_to_default")]
    pub message_history: Vec<ChatMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatSessionSummary {
    pub session_id: String,
    pub title: String,
    pub message_pair_count: usize,
    pub create_time: i64,
    pub update_time: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfoResponse {
    pub session_id: String,
    pub message_pair_count: usize,
    pub create_time: i64,
}

fn null_to_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}
