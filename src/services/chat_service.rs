use crate::{
    config::AppConfig,
    domain::{chat::ChatMessage, memory::PrivateMemorySearchResult},
    error::AppError,
};
use rig::{client::CompletionClient, completion::Prompt, providers::openai};
use tracing::{info, warn};

const REACT_AGENT_NAME: &str = "intelligent_assistant";

#[derive(Clone)]
pub struct ChatService {
    api_key: Option<String>,
    base_url: String,
    model: String,
}

#[derive(Clone)]
pub struct ReactAgent {
    name: String,
    model: String,
    system_prompt: String,
    method_tools: Vec<String>,
    tool_callbacks: Vec<String>,
}

impl ChatService {
    pub fn new(config: &AppConfig) -> Self {
        Self {
            api_key: config.dashscope_api_key.clone(),
            base_url: config.dashscope_base_url.clone(),
            model: config.dashscope_chat_model.clone(),
        }
    }

    pub fn build_method_tools_array(&self) -> Vec<String> {
        Vec::new()
    }

    pub fn get_tool_callbacks(&self) -> Vec<String> {
        Vec::new()
    }

    pub fn log_available_tools(&self) {
        let tool_callbacks = self.get_tool_callbacks();
        if tool_callbacks.is_empty() {
            info!("MCP工具未配置，无可用工具");
            return;
        }

        info!("普通聊天可用 MCP 工具列表:");
        for tool_name in tool_callbacks {
            info!(">>> {}", tool_name);
        }
    }

    pub fn chat_model(&self) -> &str {
        &self.model
    }

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

    pub fn create_react_agent(&self, model: &str, system_prompt: &str) -> ReactAgent {
        let agent = ReactAgent {
            name: REACT_AGENT_NAME.to_string(),
            model: model.to_string(),
            system_prompt: system_prompt.to_string(),
            method_tools: self.build_method_tools_array(),
            tool_callbacks: self.get_tool_callbacks(),
        };
        info!(
            "创建 ReactAgent - name: {}, model: {}, method_tools: {}, tools: {}",
            agent.name,
            agent.model,
            agent.method_tools.len(),
            agent.tool_callbacks.len()
        );
        agent
    }

    pub async fn execute_chat(
        &self,
        agent: &ReactAgent,
        question: &str,
    ) -> Result<String, AppError> {
        info!("执行 ReactAgent.call() - 自动处理工具调用");

        let answer = self.call_agent(agent, question).await?;
        info!("ReactAgent 对话完成，答案长度: {}", answer.len());
        Ok(answer)
    }

    async fn call_agent(&self, agent: &ReactAgent, question: &str) -> Result<String, AppError> {
        let Some(api_key) = self.api_key.as_ref() else {
            let answer = format!("oncall-agent-rs received: {}", question);
            return Ok(answer);
        };

        let client = openai::Client::builder()
            .api_key(api_key)
            .base_url(&self.base_url)
            .build()
            .map_err(|error| {
                AppError::internal(format!("初始化 rig OpenAI client 失败: {error}"))
            })?;

        let runtime_agent = client
            .agent(&agent.model)
            .preamble(&agent.system_prompt)
            .build();

        let answer = runtime_agent
            .prompt(question)
            .await
            .map_err(|error| AppError::internal(format!("rig agent 调用失败: {error}")))?;
        Ok(answer)
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

    #[test]
    fn create_react_agent_keeps_system_prompt() {
        let service = ChatService::new(&test_config());
        let agent = service.create_react_agent("qwen-plus", "prompt-with-history");

        assert_eq!(agent.name, "intelligent_assistant");
        assert_eq!(agent.model, "qwen-plus");
        assert_eq!(agent.system_prompt, "prompt-with-history");
        assert!(agent.method_tools.is_empty());
        assert!(agent.tool_callbacks.is_empty());
    }

    #[tokio::test]
    async fn execute_chat_without_api_key_keeps_placeholder_path_after_refactor() {
        let service = ChatService::new(&test_config());
        let agent = service.create_react_agent("qwen-plus", "prompt");

        let answer = service
            .execute_chat(&agent, "继续上次的话题")
            .await
            .expect("无 API key 时也应返回占位答案");

        assert!(answer.contains("继续上次的话题"));
    }
}
