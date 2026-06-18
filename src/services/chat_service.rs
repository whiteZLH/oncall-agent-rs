use crate::{
    config::AppConfig,
    domain::{chat::ChatMessage, memory::PrivateMemorySearchResult},
    error::AppError,
};
use rig::{client::CompletionClient, completion::Prompt, providers::openai};
use tracing::warn;

#[derive(Clone)]
pub struct ChatService {
    api_key: Option<String>,
    base_url: String,
    model: String,
}

impl ChatService {
    pub fn new(config: &AppConfig) -> Self {
        Self {
            api_key: config.dashscope_api_key.clone(),
            base_url: config.dashscope_base_url.clone(),
            model: config.dashscope_chat_model.clone(),
        }
    }

    pub fn log_available_tools(&self) {}

    pub fn build_system_prompt(
        &self,
        history: &[ChatMessage],
        private_memories: &[PrivateMemorySearchResult],
    ) -> String {
        let mut prompt = match std::fs::read_to_string("prompts/chat-system-prompt.txt") {
            Ok(content) => content,
            Err(error) => {
                warn!("无法加载系统提示词资源文件，使用默认提示词: {}", error);
                "你是一个专业的智能助手，可以获取当前时间、查询内部文档知识库，以及查询 Prometheus 告警信息。\n\n"
                    .to_string()
            }
        };
        prompt.push_str("\n\n");

        // 添加私人长期记忆
        if !private_memories.is_empty() {
            prompt.push_str("--- 私人记忆 ---\n");
            for memory in private_memories {
                if !memory.content.trim().is_empty() {
                    prompt.push_str("- ");
                    prompt.push_str(&memory.content);
                    prompt.push('\n');
                }
            }
            prompt.push_str("--- 私人记忆结束 ---\n\n");
        }

        // 添加历史消息
        if history.is_empty() {
            prompt.push_str("请基于以上对话历史，回答用户的新问题。");
            return prompt;
        }

        prompt.push_str("--- 对话历史 ---\n");
        for message in history {
            match message.role.as_str() {
                "user" => {
                    prompt.push_str("用户: ");
                    prompt.push_str(&message.content);
                    prompt.push('\n');
                }
                "assistant" => {
                    prompt.push_str("助手: ");
                    prompt.push_str(&message.content);
                    prompt.push('\n');
                }
                _ => {}
            }
        }
        prompt.push_str("--- 对话历史结束 ---\n\n请基于以上对话历史，回答用户的新问题。");
        prompt
    }

    pub async fn execute_chat(
        &self,
        session_id: &str,
        question: &str,
        system_prompt: &str,
    ) -> Result<String, AppError> {
        let Some(api_key) = self.api_key.as_ref() else {
            return Ok(format!(
                "oncall-agent-rs received [{}]: {}",
                session_id, question
            ));
        };

        let client = openai::Client::builder()
            .api_key(api_key)
            .base_url(&self.base_url)
            .build()
            .map_err(|error| {
                AppError::internal(format!("初始化 rig OpenAI client 失败: {error}"))
            })?;

        let agent = client.agent(&self.model).preamble(system_prompt).build();

        agent
            .prompt(question)
            .await
            .map_err(|error| AppError::internal(format!("rig agent 调用失败: {error}")))
    }
}

#[cfg(test)]
mod tests {
    use super::ChatService;
    use crate::{
        config::AppConfig,
        domain::{chat::ChatMessage, memory::PrivateMemorySearchResult},
    };
    use std::{net::Ipv4Addr, time::Duration};

    fn test_config() -> AppConfig {
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
            private_memory_store_path: "./target/test-private-memories".to_string(),
        }
    }

    #[test]
    fn build_system_prompt_includes_private_memories() {
        let service = ChatService::new(&test_config());
        let prompt = service.build_system_prompt(
            &[ChatMessage {
                role: "user".to_string(),
                content: "继续上次的话题".to_string(),
            }],
            &[PrivateMemorySearchResult {
                id: "memory-1".to_string(),
                content: "[用户私人记忆] 用户偏好使用中文回答".to_string(),
                score: 0.95,
                metadata: "{}".to_string(),
            }],
        );

        assert!(prompt.contains("--- 私人记忆 ---"));
        assert!(prompt.contains("[用户私人记忆] 用户偏好使用中文回答"));
        assert!(prompt.contains("--- 对话历史 ---"));
    }
}
