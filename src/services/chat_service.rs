use crate::{config::AppConfig, domain::chat::ChatMessage, error::AppError};
use rig::{
    client::CompletionClient,
    completion::Prompt,
    providers::openai,
};

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

    pub fn build_system_prompt(&self, history: &[ChatMessage]) -> String {
        if history.is_empty() {
            return "你是一个专业的智能助手。请基于当前问题给出准确、直接的回答。"
                .to_string();
        }

        let mut prompt = String::from("你是一个专业的智能助手。\n\n--- 对话历史 ---\n");
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
            return Ok(format!("oncall-agent-rs received [{}]: {}", session_id, question));
        };

        let client = openai::Client::builder()
            .api_key(api_key)
            .base_url(&self.base_url)
            .build()
            .map_err(|error| AppError::internal(format!("初始化 rig OpenAI client 失败: {error}")))?;

        let agent = client
            .agent(&self.model)
            .preamble(system_prompt)
            .build();

        agent
            .prompt(question)
            .await
            .map_err(|error| AppError::internal(format!("rig agent 调用失败: {error}")))
    }
}
