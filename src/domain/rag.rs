use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    pub id: String,
    pub content: String,
    pub score: f32,
    pub metadata: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SearchTrace {
    pub query: String,
    pub search_label: String,
    pub requested_top_k: usize,
    pub search_k: usize,
    pub search_ef: usize,
    pub filter_expr: String,
    pub candidates: Vec<SearchResult>,
    pub results: Vec<SearchResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct IndexTaskStatus {
    pub task_id: String,
    pub file_name: String,
    pub file_path: String,
    pub status: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileUploadRes {
    pub file_name: String,
    pub file_path: String,
    pub size: u64,
    pub task_id: String,
    pub status: String,
    pub message: String,
}
