use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DocumentChunk {
    pub content: String,
    pub start_index: i32,
    pub end_index: i32,
    pub chunk_index: i32,
    pub title: String,
}
