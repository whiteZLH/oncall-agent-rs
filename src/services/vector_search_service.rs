use crate::{
    config::AppConfig,
    domain::memory::PrivateMemorySearchResult,
    error::AppError,
};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{cmp::Ordering, fs, path::PathBuf};

#[derive(Clone)]
pub struct VectorSearchService {
    api_key: Option<String>,
    api_base_url: String,
    embedding_model: String,
    rerank_model: String,
    rerank_url: String,
    store_path: PathBuf,
    client: Client,
}

impl VectorSearchService {
    pub fn new(config: &AppConfig) -> Self {
        Self {
            api_key: config.dashscope_api_key.clone(),
            api_base_url: config.dashscope_api_base_url.clone(),
            embedding_model: config.dashscope_embedding_model.clone(),
            rerank_model: config.dashscope_rerank_model.clone(),
            rerank_url: config.dashscope_rerank_url.clone(),
            store_path: PathBuf::from(&config.private_memory_store_path),
            client: Client::new(),
        }
    }

    pub async fn search_session_memories(
        &self,
        query: &str,
        session_id: &str,
        top_k: usize,
    ) -> Result<Vec<PrivateMemorySearchResult>, AppError> {
        if session_id.trim().is_empty() || query.trim().is_empty() {
            return Ok(Vec::new());
        }
        let top_k = top_k.max(1);

        let records = self.load_session_memories(session_id)?;
        if records.is_empty() {
            return Ok(Vec::new());
        }

        let query_embedding = self.generate_embedding(query).await.ok();
        let mut candidates = records
            .into_iter()
            .map(|record| {
                let score = match query_embedding.as_ref().zip(record.embedding.as_ref()) {
                    Some((query_vector, memory_vector)) if query_vector.len() == memory_vector.len() => {
                        cosine_similarity(query_vector, memory_vector)
                    }
                    _ => lexical_score(query, &record.content),
                };
                PrivateMemorySearchResult {
                    id: record.id,
                    content: record.content,
                    score,
                    metadata: serde_json::to_string(&record.metadata).unwrap_or_else(|_| "{}".to_string()),
                }
            })
            .collect::<Vec<_>>();

        candidates.sort_by(|left, right| {
            right
                .score
                .partial_cmp(&left.score)
                .unwrap_or(Ordering::Equal)
        });

        let candidate_k = top_k.max(candidates.len().min(10));
        candidates.truncate(candidate_k);

        if candidates.len() > top_k {
            if let Ok(reranked) = self.rerank(query, &candidates, top_k).await {
                return Ok(reranked);
            }
        }

        candidates.truncate(top_k);
        Ok(candidates)
    }

    pub async fn generate_embedding(&self, content: &str) -> Result<Vec<f32>, AppError> {
        let api_key = self
            .api_key
            .as_ref()
            .ok_or_else(|| AppError::internal("DASHSCOPE_API_KEY 未配置"))?;
        let url = format!(
            "{}/services/embeddings/text-embedding/text-embedding",
            self.api_base_url.trim_end_matches('/')
        );
        let body = json!({
            "model": self.embedding_model,
            "input": {
                "texts": [content]
            },
        });

        let response = self
            .client
            .post(url)
            .bearer_auth(api_key)
            .json(&body)
            .send()
            .await
            .map_err(|error| AppError::internal(format!("调用 embedding 接口失败: {error}")))?;

        let status = response.status();
        let payload: Value = response
            .json()
            .await
            .map_err(|error| AppError::internal(format!("解析 embedding 响应失败: {error}")))?;
        if !status.is_success() {
            return Err(AppError::internal(format!(
                "embedding 请求失败: HTTP {status}, payload={payload}"
            )));
        }

        let Some(items) = payload["output"]["embeddings"].as_array() else {
            return Err(AppError::internal("embedding 响应缺少 output.embeddings 数组"));
        };
        let Some(first) = items.first() else {
            return Err(AppError::internal("embedding 响应 embeddings 为空"));
        };
        let Some(vector) = first["embedding"].as_array() else {
            return Err(AppError::internal("embedding 响应缺少向量"));
        };

        Ok(vector
            .iter()
            .filter_map(|value| value.as_f64())
            .map(|value| value as f32)
            .collect())
    }

    pub fn append_memory(&self, session_id: &str, record: StoredMemory) -> Result<(), AppError> {
        let mut records = self.load_session_memories(session_id)?;
        records.push(record);
        self.save_session_memories(session_id, &records)
    }

    async fn rerank(
        &self,
        query: &str,
        candidates: &[PrivateMemorySearchResult],
        top_k: usize,
    ) -> Result<Vec<PrivateMemorySearchResult>, AppError> {
        let api_key = self
            .api_key
            .as_ref()
            .ok_or_else(|| AppError::internal("DASHSCOPE_API_KEY 未配置"))?;
        let body = json!({
            "model": self.rerank_model,
            "input": {
                "query": query,
                "documents": candidates.iter().map(|item| item.content.clone()).collect::<Vec<_>>(),
            },
            "parameters": {
                "top_n": top_k,
                "return_documents": false,
            }
        });

        let response = self
            .client
            .post(&self.rerank_url)
            .bearer_auth(api_key)
            .json(&body)
            .send()
            .await
            .map_err(|error| AppError::internal(format!("调用 rerank 接口失败: {error}")))?;

        let status = response.status();
        let payload: Value = response
            .json()
            .await
            .map_err(|error| AppError::internal(format!("解析 rerank 响应失败: {error}")))?;
        if !status.is_success() {
            return Err(AppError::internal(format!(
                "rerank 请求失败: HTTP {status}, payload={payload}"
            )));
        }

        let Some(results) = payload["output"]["results"].as_array() else {
            return Err(AppError::internal("rerank 响应缺少 output.results"));
        };

        let mut reranked = Vec::new();
        for item in results {
            let Some(index) = item["index"].as_u64() else {
                continue;
            };
            let Some(candidate) = candidates.get(index as usize) else {
                continue;
            };
            reranked.push(PrivateMemorySearchResult {
                id: candidate.id.clone(),
                content: candidate.content.clone(),
                score: item["relevance_score"].as_f64().unwrap_or(candidate.score as f64) as f32,
                metadata: candidate.metadata.clone(),
            });
        }

        if reranked.is_empty() {
            return Err(AppError::internal("rerank 未返回有效结果"));
        }
        Ok(reranked)
    }

    fn load_session_memories(&self, session_id: &str) -> Result<Vec<StoredMemory>, AppError> {
        let path = self.session_memory_path(session_id);
        if !path.exists() {
            return Ok(Vec::new());
        }

        let content = fs::read_to_string(&path)
            .map_err(|error| AppError::internal(format!("读取私人记忆文件失败: {error}")))?;
        serde_json::from_str(&content)
            .map_err(|error| AppError::internal(format!("解析私人记忆文件失败: {error}")))
    }

    fn save_session_memories(
        &self,
        session_id: &str,
        records: &[StoredMemory],
    ) -> Result<(), AppError> {
        fs::create_dir_all(&self.store_path)
            .map_err(|error| AppError::internal(format!("创建私人记忆目录失败: {error}")))?;
        let content = serde_json::to_string_pretty(records)
            .map_err(|error| AppError::internal(format!("序列化私人记忆失败: {error}")))?;
        fs::write(self.session_memory_path(session_id), content)
            .map_err(|error| AppError::internal(format!("写入私人记忆文件失败: {error}")))
    }

    fn session_memory_path(&self, session_id: &str) -> PathBuf {
        self.store_path.join(format!("{session_id}.json"))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredMemory {
    pub id: String,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedding: Option<Vec<f32>>,
    pub metadata: StoredMemoryMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredMemoryMetadata {
    #[serde(rename = "_source")]
    pub source: String,
    pub doc_type: String,
    pub session_id: String,
    pub timestamp: i64,
}

fn lexical_score(query: &str, content: &str) -> f32 {
    let normalized_content = content.to_lowercase();
    let mut hits = 0.0f32;
    let mut total = 0.0f32;
    for ch in query.to_lowercase().chars().filter(|ch| !ch.is_whitespace()) {
        total += 1.0;
        if normalized_content.contains(ch) {
            hits += 1.0;
        }
    }
    if total == 0.0 {
        0.0
    } else {
        hits / total
    }
}

fn cosine_similarity(left: &[f32], right: &[f32]) -> f32 {
    if left.len() != right.len() || left.is_empty() {
        return 0.0;
    }

    let mut dot = 0.0f32;
    let mut left_norm = 0.0f32;
    let mut right_norm = 0.0f32;
    for (lhs, rhs) in left.iter().zip(right.iter()) {
        dot += lhs * rhs;
        left_norm += lhs * lhs;
        right_norm += rhs * rhs;
    }

    if left_norm == 0.0 || right_norm == 0.0 {
        0.0
    } else {
        dot / (left_norm.sqrt() * right_norm.sqrt())
    }
}

#[cfg(test)]
mod tests {
    use super::{StoredMemory, StoredMemoryMetadata, VectorSearchService};
    use crate::config::AppConfig;
    use std::{net::Ipv4Addr, time::Duration};
    use uuid::Uuid;

    fn test_config(store_path: String) -> AppConfig {
        AppConfig {
            host: Ipv4Addr::new(127, 0, 0, 1),
            port: 3000,
            allowed_origin: "*".to_string(),
            request_timeout: Duration::from_secs(30),
            log_filter: "info".to_string(),
            redis_url: None,
            chat_history_path: "./target/test-chat-history".to_string(),
            session_ttl_secs: 3600,
            dashscope_api_key: None,
            dashscope_base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1".to_string(),
            dashscope_api_base_url: "https://dashscope.aliyuncs.com/api/v1".to_string(),
            dashscope_chat_model: "qwen-plus".to_string(),
            dashscope_embedding_model: "text-embedding-v4".to_string(),
            dashscope_rerank_model: "gte-rerank".to_string(),
            dashscope_rerank_url:
                "https://dashscope.aliyuncs.com/api/v1/services/rerank/text-rerank/text-rerank"
                    .to_string(),
            private_memory_recall_enabled: true,
            private_memory_recall_top_k: 3,
            private_memory_store_path: store_path,
        }
    }

    #[tokio::test]
    async fn search_session_memories_reads_local_private_memories() {
        let store_path = std::env::temp_dir()
            .join(format!("oncall-agent-rs-private-memory-{}", Uuid::new_v4()));
        let service = VectorSearchService::new(&test_config(store_path.to_string_lossy().to_string()));

        service
            .append_memory(
                "session-1",
                StoredMemory {
                    id: "memory-1".to_string(),
                    content: "[用户私人记忆] 用户偏好中文回答".to_string(),
                    embedding: None,
                    metadata: StoredMemoryMetadata {
                        source: "chat_memory".to_string(),
                        doc_type: "chat_memory".to_string(),
                        session_id: "session-1".to_string(),
                        timestamp: 1,
                    },
                },
            )
            .expect("写入测试私人记忆失败");

        let results = service
            .search_session_memories("请继续用中文回答", "session-1", 3)
            .await
            .expect("搜索私人记忆失败");

        assert_eq!(results.len(), 1);
        assert!(results[0].content.contains("中文回答"));
        let _ = std::fs::remove_dir_all(store_path);
    }
}
