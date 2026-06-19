use crate::{
    config::AppConfig,
    domain::{chat::ChatMessage, memory::PrivateMemorySearchResult},
    error::AppError,
    services::vector_search_service::VectorSearchService,
};
use rig::{
    client::CompletionClient,
    completion::{Prompt, PromptError, ToolDefinition},
    providers::openai,
    tool::{Tool, ToolDyn},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{info, warn};

const REACT_AGENT_NAME: &str = "intelligent_assistant";

#[derive(Clone)]
pub struct ChatService {
    api_key: Option<String>,
    base_url: String,
    model: String,
    max_turns: usize,
    vector_search_service: Option<VectorSearchService>,
}

#[derive(Clone)]
pub struct ReactAgent {
    name: String,
    model: String,
    system_prompt: String,
    method_tools: Vec<String>,
    tool_callbacks: Vec<String>,
    max_turns: usize,
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
struct ChatToolError(String);

#[derive(Clone, Serialize, Deserialize)]
struct GetCurrentDateTimeTool;

#[derive(Clone, Serialize, Deserialize)]
struct GetCurrentDateTimeArgs {}

#[derive(Clone)]
struct QueryInternalDocsTool {
    vector_search_service: Option<VectorSearchService>,
}

#[derive(Clone, Deserialize)]
struct QueryInternalDocsArgs {
    query: String,
}

impl ChatService {
    pub fn new(config: &AppConfig) -> Self {
        Self {
            api_key: config.dashscope_api_key.clone(),
            base_url: config.dashscope_base_url.clone(),
            model: config.dashscope_chat_model.clone(),
            max_turns: config.chat_agent_max_turns,
            vector_search_service: None,
        }
    }

    pub fn with_vector_search(mut self, vector_search_service: VectorSearchService) -> Self {
        self.vector_search_service = Some(vector_search_service);
        self
    }

    pub fn build_method_tools_array(&self) -> Vec<String> {
        self.build_method_tools()
            .iter()
            .map(|tool| tool.name())
            .collect()
    }

    fn build_method_tools(&self) -> Vec<Box<dyn ToolDyn>> {
        vec![
            Box::new(GetCurrentDateTimeTool),
            Box::new(QueryInternalDocsTool {
                vector_search_service: self.vector_search_service.clone(),
            }),
        ]
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
            max_turns: self.max_turns,
        };
        info!(
            "创建 ReactAgent - name: {}, model: {}, method_tools: {}, tools: {}, max_turns: {}",
            agent.name,
            agent.model,
            agent.method_tools.len(),
            agent.tool_callbacks.len(),
            agent.max_turns
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
            .name(&agent.name)
            .preamble(&agent.system_prompt)
            .default_max_turns(agent.max_turns)
            .tools(self.build_method_tools())
            .build();

        let answer = runtime_agent
            .prompt(question)
            .max_turns(agent.max_turns)
            .with_tool_concurrency(1)
            .await
            .map_err(|error| map_prompt_error(error, agent.max_turns))?;
        Ok(answer)
    }
}

impl Tool for GetCurrentDateTimeTool {
    const NAME: &'static str = "getCurrentDateTime";

    type Error = ChatToolError;
    type Args = GetCurrentDateTimeArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Get the current date and time in the user's timezone".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {},
            }),
        }
    }

    async fn call(&self, _args: Self::Args) -> Result<Self::Output, Self::Error> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| ChatToolError(format!("系统时间早于 Unix 纪元: {error}")))?;
        Ok(format!("unix_timestamp_seconds={}", now.as_secs()))
    }
}

impl Tool for QueryInternalDocsTool {
    const NAME: &'static str = "queryInternalDocs";

    type Error = ChatToolError;
    type Args = QueryInternalDocsArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Use this tool to search internal documentation and knowledge base for relevant information.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Search query describing what information you are looking for"
                    }
                },
                "required": ["query"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let Some(vector_search_service) = self.vector_search_service.as_ref() else {
            return Ok(json!({
                "status": "error",
                "message": "Internal document search service is not configured.",
                "query": args.query,
            })
            .to_string());
        };

        match vector_search_service
            .search_similar_documents(&args.query, 3)
            .await
        {
            Ok(results) if results.is_empty() => Ok(json!({
                "status": "no_results",
                "message": "No relevant documents found in the knowledge base.",
                "query": args.query,
            })
            .to_string()),
            Ok(results) => Ok(json!({
                "status": "success",
                "query": args.query,
                "results": results,
            })
            .to_string()),
            Err(error) => Ok(json!({
                "status": "error",
                "message": error.to_string(),
                "query": args.query,
            })
            .to_string()),
        }
    }
}

fn map_prompt_error(error: PromptError, max_turns: usize) -> AppError {
    match error {
        PromptError::MaxTurnsError { .. } => AppError::internal(format!(
            "rig agent 调用失败: reached max turn limit: {max_turns}"
        )),
        error => AppError::internal(format!("rig agent 调用失败: {error}")),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ChatService, GetCurrentDateTimeArgs, GetCurrentDateTimeTool, QueryInternalDocsArgs,
        QueryInternalDocsTool,
    };
    use crate::{
        config::AppConfig,
        domain::{chat::ChatMessage, memory::PrivateMemorySearchResult},
    };
    use rig::tool::Tool;
    use serde_json::Value;
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
            chat_agent_max_turns: 6,
            dashscope_embedding_model: "text-embedding-v4".to_string(),
            dashscope_rerank_model: "gte-rerank".to_string(),
            dashscope_rerank_url:
                "https://dashscope.aliyuncs.com/api/v1/services/rerank/text-rerank/text-rerank"
                    .to_string(),
            milvus_host: "localhost".to_string(),
            milvus_port: 19530,
            milvus_username: String::new(),
            milvus_password: String::new(),
            milvus_database: "default".to_string(),
            milvus_timeout_ms: 10_000,
            rag_candidate_k: 10,
            rag_search_ef: 64,
            upload_path: "./target/uploads".to_string(),
            upload_allowed_extensions: vec!["txt".to_string(), "md".to_string()],
            document_chunk_max_size: 800,
            document_chunk_overlap: 100,
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
        assert_eq!(agent.max_turns, 6);
        assert_eq!(
            agent.method_tools,
            vec![
                "getCurrentDateTime".to_string(),
                "queryInternalDocs".to_string()
            ]
        );
        assert!(agent.tool_callbacks.is_empty());
    }

    #[tokio::test]
    async fn get_current_datetime_tool_returns_non_empty_time() {
        let tool = GetCurrentDateTimeTool;
        let definition = tool.definition(String::new()).await;

        assert_eq!(definition.name, "getCurrentDateTime");
        let output = tool
            .call(GetCurrentDateTimeArgs {})
            .await
            .expect("时间工具应返回当前时间");

        assert!(!output.trim().is_empty());
    }

    #[tokio::test]
    async fn query_internal_docs_tool_returns_unconfigured_error() {
        let tool = QueryInternalDocsTool {
            vector_search_service: None,
        };
        let definition = tool.definition(String::new()).await;

        assert_eq!(definition.name, "queryInternalDocs");
        let output = tool
            .call(QueryInternalDocsArgs {
                query: "cpu runbook".to_string(),
            })
            .await
            .expect("文档工具应返回 JSON");
        let payload: Value = serde_json::from_str(&output).expect("工具输出应是 JSON");

        assert_eq!(payload["status"], "error");
        assert_eq!(payload["query"], "cpu runbook");
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
