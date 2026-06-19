use crate::{config::AppConfig, domain::rag::SearchResult};
use reqwest::Client;
use serde_json::{json, Value};
use tracing::{info, warn};

#[derive(Clone)]
pub struct VectorRerankService {
    api_key: Option<String>,
    model: String,
    rerank_url: String,
    client: Client,
}

impl VectorRerankService {
    pub fn new(config: &AppConfig) -> Self {
        Self {
            api_key: config.dashscope_api_key.clone(),
            model: config.dashscope_rerank_model.clone(),
            rerank_url: config.dashscope_rerank_url.clone(),
            client: Client::new(),
        }
    }

    pub async fn rerank(
        &self,
        query: &str,
        candidates: &[SearchResult],
        top_k: usize,
    ) -> Vec<SearchResult> {
        if candidates.is_empty() {
            return Vec::new();
        }

        let Some(api_key) = self.api_key.as_ref() else {
            return defensive_copy_top_k(candidates, top_k);
        };

        info!(
            "开始调用阿里 GTE-Rerank 重排, 候选文档数: {}, 目标返回数: {}",
            candidates.len(),
            top_k
        );

        let body = json!({
            "model": self.model,
            "input": {
                "query": query,
                "documents": candidates.iter().map(|item| item.content.clone()).collect::<Vec<_>>(),
            },
            "parameters": {
                "top_n": top_k,
                "return_documents": false,
            }
        });

        let response = match self
            .client
            .post(&self.rerank_url)
            .bearer_auth(api_key)
            .json(&body)
            .send()
            .await
        {
            Ok(response) => response,
            Err(error) => {
                warn!("Rerank API 网络请求异常: {}", error);
                return defensive_copy_top_k(candidates, top_k);
            }
        };

        let status = response.status();
        let payload: Value = match response.json().await {
            Ok(payload) => payload,
            Err(error) => {
                warn!("解析 rerank 响应失败: {}", error);
                return defensive_copy_top_k(candidates, top_k);
            }
        };

        if !status.is_success() {
            warn!("Rerank API 调用失败: HTTP {}, {}", status, payload);
            return defensive_copy_top_k(candidates, top_k);
        }

        let Some(results) = payload["output"]["results"].as_array() else {
            warn!("Rerank API 返回格式异常: {}", payload);
            return defensive_copy_top_k(candidates, top_k);
        };

        let mut final_results = Vec::new();
        for item in results {
            let Some(index) = item["index"].as_u64() else {
                continue;
            };
            let Some(original) = candidates.get(index as usize) else {
                continue;
            };
            let mut copy = original.clone();
            copy.score = item["relevance_score"]
                .as_f64()
                .unwrap_or(original.score as f64) as f32;
            final_results.push(copy);
        }

        if final_results.is_empty() {
            defensive_copy_top_k(candidates, top_k)
        } else {
            final_results
        }
    }
}

pub fn defensive_copy_top_k(candidates: &[SearchResult], top_k: usize) -> Vec<SearchResult> {
    candidates.iter().take(top_k).cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::defensive_copy_top_k;
    use crate::domain::rag::SearchResult;

    #[test]
    fn defensive_copy_limits_to_top_k() {
        let results = vec![
            SearchResult {
                id: "1".to_string(),
                content: "a".to_string(),
                score: 0.1,
                metadata: "{}".to_string(),
            },
            SearchResult {
                id: "2".to_string(),
                content: "b".to_string(),
                score: 0.2,
                metadata: "{}".to_string(),
            },
        ];

        assert_eq!(defensive_copy_top_k(&results, 1), vec![results[0].clone()]);
    }
}
