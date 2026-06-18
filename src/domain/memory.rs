use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrivateMemorySearchResult {
    pub id: String,
    pub content: String,
    pub score: f32,
    pub metadata: String,
}
