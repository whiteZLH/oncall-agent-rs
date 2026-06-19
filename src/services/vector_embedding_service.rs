use crate::{config::AppConfig, error::AppError};
use reqwest::Client;
use serde_json::{json, Value};
use std::collections::BTreeMap;

const MAX_EMBEDDING_BATCH_SIZE: usize = 10;

#[derive(Clone)]
pub struct VectorEmbeddingService {
    api_key: Option<String>,
    api_base_url: String,
    model: String,
    client: Client,
}

impl VectorEmbeddingService {
    pub fn new(config: &AppConfig) -> Self {
        Self {
            api_key: config.dashscope_api_key.clone(),
            api_base_url: config.dashscope_api_base_url.clone(),
            model: config.dashscope_embedding_model.clone(),
            client: Client::new(),
        }
    }

    pub async fn generate_embedding(&self, content: &str) -> Result<Vec<f32>, AppError> {
        if content.trim().is_empty() {
            return Err(AppError::bad_request("内容不能为空"));
        }
        let embeddings = self.generate_embeddings(&[content.to_string()]).await?;
        embeddings
            .into_iter()
            .next()
            .ok_or_else(|| AppError::internal("DashScope API 返回空向量列表"))
    }

    pub async fn generate_query_vector(&self, query: &str) -> Result<Vec<f32>, AppError> {
        self.generate_embedding(query).await
    }

    pub async fn generate_embeddings(
        &self,
        contents: &[String],
    ) -> Result<Vec<Vec<f32>>, AppError> {
        if contents.is_empty() {
            return Ok(Vec::new());
        }
        for (index, content) in contents.iter().enumerate() {
            if content.trim().is_empty() {
                return Err(AppError::bad_request(format!(
                    "内容列表第 {index} 项不能为空"
                )));
            }
        }

        let mut embeddings = Vec::with_capacity(contents.len());
        for batch in contents.chunks(MAX_EMBEDDING_BATCH_SIZE) {
            embeddings.extend(self.generate_embedding_batch(batch).await?);
        }
        Ok(embeddings)
    }

    async fn generate_embedding_batch(&self, batch: &[String]) -> Result<Vec<Vec<f32>>, AppError> {
        let api_key = self
            .api_key
            .as_ref()
            .ok_or_else(|| AppError::internal("DASHSCOPE_API_KEY 未配置"))?;
        let url = format!(
            "{}/services/embeddings/text-embedding/text-embedding",
            self.api_base_url.trim_end_matches('/')
        );
        let body = json!({
            "model": self.model,
            "input": {
                "texts": batch
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

        parse_embedding_items(&payload, batch.len())
    }
}

pub fn generate_sparse_vector(content: &str) -> BTreeMap<i64, f32> {
    let mut sparse = BTreeMap::new();
    if content.trim().is_empty() {
        return sparse;
    }

    for ch in content.to_lowercase().chars() {
        if ch.is_whitespace() {
            continue;
        }
        let hash = java_string_hash(&ch.to_string()).abs() as i64 % 1_000_000;
        *sparse.entry(hash).or_insert(0.0) += 1.0;
    }
    sparse
}

fn parse_embedding_items(payload: &Value, batch_size: usize) -> Result<Vec<Vec<f32>>, AppError> {
    let Some(items) = payload["output"]["embeddings"].as_array() else {
        return Err(AppError::internal(
            "embedding 响应缺少 output.embeddings 数组",
        ));
    };
    if items.is_empty() {
        return Err(AppError::internal("embedding 响应 embeddings 为空"));
    }
    if items.len() != batch_size {
        return Err(AppError::internal(
            "批量 DashScope API 返回向量数量与输入数量不一致",
        ));
    }

    let mut reordered: Vec<Option<Vec<f32>>> = vec![None; batch_size];
    for (fallback_index, item) in items.iter().enumerate() {
        let index = item["text_index"]
            .as_u64()
            .or_else(|| item["textIndex"].as_u64())
            .map(|value| value as usize)
            .unwrap_or(fallback_index);
        if index >= batch_size {
            return Err(AppError::internal("批量 DashScope API 返回无效 textIndex"));
        }
        if reordered[index].is_some() {
            return Err(AppError::internal("批量 DashScope API 返回重复 textIndex"));
        }
        let Some(vector) = item["embedding"].as_array() else {
            return Err(AppError::internal("embedding 响应缺少向量"));
        };
        reordered[index] = Some(
            vector
                .iter()
                .filter_map(|value| value.as_f64())
                .map(|value| value as f32)
                .collect(),
        );
    }

    reordered
        .into_iter()
        .map(|item| item.ok_or_else(|| AppError::internal("批量 DashScope API 返回缺失 textIndex")))
        .collect()
}

fn java_string_hash(value: &str) -> i32 {
    let mut hash = 0i32;
    for unit in value.encode_utf16() {
        hash = hash.wrapping_mul(31).wrapping_add(unit as i32);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::generate_sparse_vector;

    #[test]
    fn sparse_vector_matches_java_character_hashing() {
        let sparse = generate_sparse_vector("Aa 中");

        assert_eq!(sparse.get(&97), Some(&2.0));
        assert_eq!(sparse.get(&20013), Some(&1.0));
        assert_eq!(sparse.len(), 2);
    }
}
