use crate::{
    config::AppConfig,
    domain::chat::ChatMessage,
    error::AppError,
    services::vector_search_service::{StoredMemory, StoredMemoryMetadata, VectorSearchService},
};
use reqwest::Client;
use serde_json::{json, Value};
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

const DOC_TYPE_CHAT_MEMORY: &str = "chat_memory";

#[derive(Clone)]
pub struct MemoryExtractionService {
    api_key: Option<String>,
    chat_base_url: String,
    chat_model: String,
    client: Client,
    vector_search_service: VectorSearchService,
}

impl MemoryExtractionService {
    pub fn new(config: &AppConfig) -> Self {
        Self {
            api_key: config.dashscope_api_key.clone(),
            chat_base_url: config.dashscope_base_url.clone(),
            chat_model: config.dashscope_chat_model.clone(),
            client: Client::new(),
            vector_search_service: VectorSearchService::new(config),
        }
    }

    pub async fn extract_and_store(
        &self,
        session_id: &str,
        history_to_archive: &[ChatMessage],
    ) -> Result<(), AppError> {
        if session_id.trim().is_empty() || history_to_archive.is_empty() || self.api_key.is_none() {
            return Ok(());
        }

        let mut conversation = String::new();
        for message in history_to_archive {
            conversation.push_str(&message.role);
            conversation.push_str(": ");
            conversation.push_str(&message.content);
            conversation.push('\n');
        }

        let response_text = self
            .extract_facts_from_conversation(&conversation)
            .await?
            .trim()
            .to_string();
        if response_text.is_empty() || response_text.eq_ignore_ascii_case("NONE") {
            return Ok(());
        }

        for fact in response_text.split(';') {
            let fact = fact.trim();
            if fact.is_empty() {
                continue;
            }

            let content = format!("[用户私人记忆] {fact}");
            let embedding = self.vector_search_service.generate_embedding(&content).await.ok();
            self.vector_search_service.append_memory(
                session_id,
                StoredMemory {
                    id: Uuid::new_v4().to_string(),
                    content,
                    embedding,
                    metadata: StoredMemoryMetadata {
                        source: "chat_memory".to_string(),
                        doc_type: DOC_TYPE_CHAT_MEMORY.to_string(),
                        session_id: session_id.to_string(),
                        timestamp: now_millis(),
                    },
                },
            )?;
        }

        Ok(())
    }

    async fn extract_facts_from_conversation(&self, conversation: &str) -> Result<String, AppError> {
        let api_key = self
            .api_key
            .as_ref()
            .ok_or_else(|| AppError::internal("DASHSCOPE_API_KEY 未配置"))?;
        let url = format!("{}/chat/completions", self.chat_base_url.trim_end_matches('/'));
        let body = json!({
            "model": self.chat_model,
            "messages": [
                {
                    "role": "system",
                    "content": "你是记忆分析专家。请分析以下用户和AI的对话，提取出其中长期有价值的技术事实、用户偏好、业务上下文或排障结论。\n规则：\n1. 提取的事实必须客观、清晰、简短，以第一人称（用户视角）或者第三人称陈述。\n2. 忽略寒暄、无意义的提问或暂时性的信息。\n3. 如果对话中没有任何值得长期记忆的价值，请严格且仅输出 'NONE'。\n4. 如果有多个事实，请用分号 ';' 隔开。"
                },
                {
                    "role": "user",
                    "content": format!("对话记录如下：\n{conversation}")
                }
            ]
        });

        let response = self
            .client
            .post(url)
            .bearer_auth(api_key)
            .json(&body)
            .send()
            .await
            .map_err(|error| AppError::internal(format!("调用记忆提炼模型失败: {error}")))?;

        let status = response.status();
        let payload: Value = response
            .json()
            .await
            .map_err(|error| AppError::internal(format!("解析记忆提炼响应失败: {error}")))?;
        if !status.is_success() {
            return Err(AppError::internal(format!(
                "记忆提炼请求失败: HTTP {status}, payload={payload}"
            )));
        }

        payload["choices"][0]["message"]["content"]
            .as_str()
            .map(ToOwned::to_owned)
            .ok_or_else(|| AppError::internal("记忆提炼响应缺少 content"))
    }
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("系统时间早于 Unix 纪元")
        .as_millis() as i64
}
