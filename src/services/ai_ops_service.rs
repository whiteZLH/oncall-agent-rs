//! AI Ops 智能运维服务
//! 对应 Java 版本 `AiOpsService`，负责多 Agent 协作的告警分析流程。
//!
//! 编排结构与 Java 版保持一致：Supervisor 调度 Planner 与 Executor，
//! 执行「规划 → 执行 → 再规划」的闭环，直到 Planner 输出 `decision=FINISH`
//! 的《告警分析报告》。Java 版借助 spring-ai-alibaba-graph 的 `SupervisorAgent`
//! 由大模型路由；本实现以确定性的 Supervisor 循环复刻同一闭环，
//! Planner / Executor 的系统提示词、工具集合与调用逻辑均与 Java 版逐字对齐。

use crate::{
    agent::{
        evidence::RecordingTool,
        logs_tools::{GetAvailableLogTopicsTool, QueryLogsTool},
        metrics_tools::{QueryMetricTrendTool, QueryPrometheusAlertsTool},
    },
    config::AppConfig,
    error::AppError,
    services::{
        agent_runtime::{OverAllState, ReactAgent, SupervisorAgent},
        chat_service::{GetCurrentDateTimeTool, QueryInternalDocsTool},
        diagnosis_report_service::augment_alert_context,
        incident_service::EvidenceCollector,
        vector_search_service::VectorSearchService,
    },
};
use rig::{
    client::CompletionClient,
    completion::{Prompt, PromptError, ToolDefinition},
    providers::openai,
    tool::{ToolDyn, ToolError},
};
use serde::Deserialize;
use std::{future::Future, pin::Pin, sync::Arc};
use tokio::sync::Mutex;
use tracing::{info, warn};

/// 对齐 Java 版 `DashScopeChatModel` 在 `AiOpsService` 方法参数中的角色。
/// Rust 侧仍由配置构造模型参数，但公开入口显式接收该对象，避免把模型选择
/// 隐藏在服务内部。
#[derive(Clone, Debug)]
pub struct AiOpsChatModel {
    api_key: Option<String>,
    base_url: String,
    model: String,
}

/// 对齐 Java 版 `ToolCallback[]`。
/// rig 的 `ToolDyn` 是有所有权的对象，每次构建 Agent 都需要一组新的工具实例，
/// 所以这里用工厂表达“可重复创建的 ToolCallback”。
pub type AiOpsToolCallback = Arc<dyn Fn() -> Box<dyn ToolDyn> + Send + Sync>;

#[derive(Clone)]
pub struct AiOpsService {
    api_key: Option<String>,
    base_url: String,
    model: String,
    agent_max_turns: usize,
    max_rounds: usize,
    vector_search_service: Option<VectorSearchService>,
    prometheus_base_url: String,
    prometheus_timeout_secs: u64,
    prometheus_mock_enabled: bool,
    cls_mock_enabled: bool,
}

impl AiOpsService {
    pub fn new(config: &AppConfig) -> Self {
        Self {
            api_key: config.dashscope_api_key.clone(),
            // 工具调用走 Chat Completions，与 ChatService 流式路径保持同一 base_url
            base_url: config.dashscope_api_base_url.clone(),
            model: config.ai_ops_chat_model.clone(),
            agent_max_turns: config.ai_ops_agent_max_turns,
            max_rounds: config.ai_ops_max_rounds,
            vector_search_service: None,
            prometheus_base_url: config.prometheus_base_url.clone(),
            prometheus_timeout_secs: config.prometheus_timeout_secs,
            prometheus_mock_enabled: config.prometheus_mock_enabled,
            cls_mock_enabled: config.cls_mock_enabled,
        }
    }

    pub fn with_vector_search(mut self, vector_search_service: VectorSearchService) -> Self {
        self.vector_search_service = Some(vector_search_service);
        self
    }

    /// 返回 AI Ops 使用的模型配置，对齐 Java Controller 中传入的 `dashScopeChatModelAiOps`。
    pub fn chat_model(&self) -> AiOpsChatModel {
        AiOpsChatModel {
            api_key: self.api_key.clone(),
            base_url: self.base_url.clone(),
            model: self.model.clone(),
        }
    }

    /// 返回外部工具回调集合。当前 Rust 版尚无 Spring MCP registry，对齐 Java 形态
    /// 先保留显式参数位；内置方法工具仍由 `build_method_tools` 添加。
    pub fn tool_callbacks(&self) -> Vec<AiOpsToolCallback> {
        Vec::new()
    }

    /// 动态构建方法工具集合。
    /// 始终包含：时间、内部文档、Prometheus 告警、指标趋势；
    /// 当 `cls_mock_enabled` 为真时（对应 Java 的 mock 模式注册 QueryLogsTools），
    /// 追加日志查询与日志主题工具。
    fn build_method_tools(
        &self,
        tool_callbacks: &[AiOpsToolCallback],
        collector: Option<&Arc<EvidenceCollector>>,
    ) -> Vec<Box<dyn ToolDyn>> {
        let mut tools: Vec<Box<dyn ToolDyn>> = vec![
            Box::new(GetCurrentDateTimeTool),
            Box::new(QueryInternalDocsTool {
                vector_search_service: self.vector_search_service.clone(),
            }),
            Box::new(QueryPrometheusAlertsTool {
                base_url: self.prometheus_base_url.clone(),
                timeout_secs: self.prometheus_timeout_secs,
                mock_enabled: self.prometheus_mock_enabled,
            }),
            Box::new(QueryMetricTrendTool {
                base_url: self.prometheus_base_url.clone(),
                timeout_secs: self.prometheus_timeout_secs,
                mock_enabled: self.prometheus_mock_enabled,
            }),
        ];
        if self.cls_mock_enabled {
            tools.push(Box::new(QueryLogsTool { mock_enabled: true }));
            tools.push(Box::new(GetAvailableLogTopicsTool));
        }
        tools.extend(self.guard_mcp_tool_callbacks(tool_callbacks));
        // 诊断 run 路径：用 RecordingTool 包装每个工具，实时记录证据并注入 evidence id
        // （对齐 Java 5 参数 executeAiOpsAnalysis 的 wrapToolCallbacks；普通 /ai_ops 路径 collector 为 None，不包装）。
        match collector {
            Some(collector) => tools
                .into_iter()
                .map(|tool| {
                    Box::new(RecordingTool::new(tool, Arc::clone(collector))) as Box<dyn ToolDyn>
                })
                .collect(),
            None => tools,
        }
    }

    fn build_method_tools_for_agent(
        &self,
        tool_callbacks: &[AiOpsToolCallback],
        collector: Option<&Arc<EvidenceCollector>>,
        agent: &ReactAgent,
    ) -> Vec<Box<dyn ToolDyn>> {
        self.build_method_tools(tool_callbacks, collector)
            .into_iter()
            .filter(|tool| agent.allows_tool(&tool.name()))
            .collect()
    }

    fn guard_mcp_tool_callbacks(
        &self,
        tool_callbacks: &[AiOpsToolCallback],
    ) -> Vec<Box<dyn ToolDyn>> {
        tool_callbacks
            .iter()
            .map(|factory| {
                let tool = factory();
                if let Some(dependency) = mcp_dependency_name(&tool.name()) {
                    Box::new(GuardedMcpTool::new(tool, dependency)) as Box<dyn ToolDyn>
                } else {
                    tool
                }
            })
            .collect()
    }

    /// 执行 AI Ops 告警分析流程。
    ///
    /// * `alert_context` —— 告警上下文信息（可为空）
    ///
    /// 返回 Java 风格的多 Agent 编排状态；最终报告需通过 `extract_final_report` 提取。
    pub async fn execute_ai_ops_analysis(
        &self,
        alert_context: Option<&str>,
    ) -> Result<Option<OverAllState>, AppError> {
        let chat_model = self.chat_model();
        let tool_callbacks = self.tool_callbacks();
        self.execute_ai_ops_analysis_with_context(&chat_model, &tool_callbacks, alert_context)
            .await
    }

    /// 对齐 Java `executeAiOpsAnalysis(chatModel, toolCallbacks)`。
    pub async fn execute_ai_ops_analysis_with_model(
        &self,
        chat_model: &AiOpsChatModel,
        tool_callbacks: &[AiOpsToolCallback],
    ) -> Result<Option<OverAllState>, AppError> {
        self.execute_ai_ops_analysis_with_context(chat_model, tool_callbacks, None)
            .await
    }

    /// 对齐 Java `executeAiOpsAnalysis(chatModel, toolCallbacks, alertContext)`。
    pub async fn execute_ai_ops_analysis_with_context(
        &self,
        chat_model: &AiOpsChatModel,
        tool_callbacks: &[AiOpsToolCallback],
        alert_context: Option<&str>,
    ) -> Result<Option<OverAllState>, AppError> {
        self.execute_ai_ops_analysis_internal(chat_model, tool_callbacks, alert_context, None)
            .await
    }

    /// 在指定诊断 run 上下文中执行 AI Ops（对齐 Java 5 参数 `executeAiOpsAnalysis`）：
    /// 先用已累积证据增强告警上下文（`augmentAlertContext`），并以 `RecordingTool`
    /// 包装工具，实时记录证据并向工具返回注入 `_diagnosisEvidenceId`。
    pub async fn execute_ai_ops_analysis_for_run(
        &self,
        alert_context: Option<&str>,
        collector: Arc<EvidenceCollector>,
    ) -> Result<Option<OverAllState>, AppError> {
        let chat_model = self.chat_model();
        let tool_callbacks = self.tool_callbacks();
        let incident_id = collector.incident_id().to_string();
        let run_id = collector.run_id().to_string();
        self.execute_ai_ops_analysis_with_run(
            &chat_model,
            &tool_callbacks,
            alert_context,
            &incident_id,
            &run_id,
            Some(collector),
        )
        .await
    }

    /// 对齐 Java `executeAiOpsAnalysis(chatModel, toolCallbacks, alertContext, incidentId, runId)`。
    /// `collector` 对应 Java 的 `DiagnosisEvidenceRecorder.withRun(...)` 运行作用域；
    /// 缺失 collector 或 incident/run id 为空时，退化为普通 internal 调用。
    pub async fn execute_ai_ops_analysis_with_run(
        &self,
        chat_model: &AiOpsChatModel,
        tool_callbacks: &[AiOpsToolCallback],
        alert_context: Option<&str>,
        incident_id: &str,
        run_id: &str,
        collector: Option<Arc<EvidenceCollector>>,
    ) -> Result<Option<OverAllState>, AppError> {
        let evidence_bound_context = self.build_evidence_bound_alert_context(
            alert_context,
            incident_id,
            run_id,
            collector.as_ref(),
        );
        if collector.is_none() || incident_id.trim().is_empty() || run_id.trim().is_empty() {
            return self
                .execute_ai_ops_analysis_internal(
                    chat_model,
                    tool_callbacks,
                    Some(&evidence_bound_context),
                    None,
                )
                .await;
        }

        self.execute_ai_ops_analysis_internal(
            chat_model,
            tool_callbacks,
            Some(&evidence_bound_context),
            collector,
        )
        .await
    }

    fn build_evidence_bound_alert_context(
        &self,
        alert_context: Option<&str>,
        incident_id: &str,
        run_id: &str,
        collector: Option<&Arc<EvidenceCollector>>,
    ) -> String {
        if incident_id.trim().is_empty() || run_id.trim().is_empty() {
            return alert_context.unwrap_or("").to_string();
        }
        let Some(collector) = collector else {
            return alert_context.unwrap_or("").to_string();
        };
        let evidence = collector.snapshot_evidence();
        augment_alert_context(alert_context.unwrap_or(""), &evidence)
    }

    async fn execute_ai_ops_analysis_internal(
        &self,
        chat_model: &AiOpsChatModel,
        tool_callbacks: &[AiOpsToolCallback],
        alert_context: Option<&str>,
        collector: Option<Arc<EvidenceCollector>>,
    ) -> Result<Option<OverAllState>, AppError> {
        info!("开始执行 AI Ops 多 Agent 协作流程");

        let sub_agents = self.build_ai_ops_sub_agents(chat_model, tool_callbacks);

        // 构建 Supervisor Agent
        let supervisor_agent = self.build_supervisor_agent(chat_model, sub_agents);

        // 构建任务提示（注入告警上下文）
        let mut task = TASK_PROMPT.to_string();

        if let Some(ctx) = alert_context.map(str::trim).filter(|ctx| !ctx.is_empty()) {
            task.push_str("\n\n## 当前告警上下文\n");
            task.push_str(ctx);
            task.push_str("\n\n请基于以上告警上下文进行分析和处理。");
            info!("已注入告警上下文，长度: {}", ctx.chars().count());
        }

        let invocation =
            AiOpsInvocationContext::new(chat_model, tool_callbacks, collector.as_ref())?;

        info!("调用 Supervisor Agent 开始编排...");
        self.invoke_supervisor(&supervisor_agent, &task, invocation)
            .await
    }

    /// 对齐 Java `invokeSupervisor(supervisorAgent, taskPrompt)`：
    /// Rust/rig 需要的 client、工具工厂与证据记录器通过 invocation 上下文显式承载。
    async fn invoke_supervisor(
        &self,
        supervisor_agent: &SupervisorAgent,
        task_prompt: &str,
        invocation: AiOpsInvocationContext<'_>,
    ) -> Result<Option<OverAllState>, AppError> {
        info!(
            "调用 Supervisor handoff 编排，最大 handoff 轮次: {}, 子 Agent 最大工具轮次: {}",
            supervisor_agent.max_turns(),
            self.agent_max_turns
        );

        let mut state = OverAllState::new();
        state.insert_text("input", task_prompt.to_string());
        let shared_state = Arc::new(Mutex::new(state));
        let client = Arc::clone(&invocation.client);
        let handoff_tools = self.build_handoff_tools(
            client,
            invocation.tool_callbacks,
            invocation.collector.cloned(),
            Arc::clone(&shared_state),
            supervisor_agent.sub_agents(),
        );

        let runtime_agent = invocation
            .client
            .agent(supervisor_agent.model())
            .name(supervisor_agent.name())
            .temperature(0.3)
            .max_tokens(8000)
            .additional_params(serde_json::json!({ "top_p": 0.9 }))
            .preamble(&supervisor_agent.build_handoff_preamble())
            .tools(handoff_tools)
            .build();

        let report = runtime_agent
            .prompt(task_prompt.to_string())
            .max_turns(supervisor_agent.max_turns())
            .await
            .map_err(|error: PromptError| map_prompt_error(error, supervisor_agent.max_turns()))?;

        let mut state = shared_state.lock().await.clone();
        if !report.trim().is_empty() {
            state.insert_assistant_message("supervisor_output", report);
        }
        Ok(Some(state))
    }

    fn build_ai_ops_sub_agents(
        &self,
        chat_model: &AiOpsChatModel,
        tool_callbacks: &[AiOpsToolCallback],
    ) -> Vec<ReactAgent> {
        let mut agents = vec![
            self.build_metrics_agent(chat_model, tool_callbacks),
            self.build_knowledge_agent(chat_model, tool_callbacks),
            self.build_remediation_agent(chat_model, tool_callbacks),
        ];
        if self.cls_mock_enabled {
            agents.push(self.build_logs_agent(chat_model, tool_callbacks));
        }
        agents
    }

    fn build_metrics_agent(
        &self,
        chat_model: &AiOpsChatModel,
        tool_callbacks: &[AiOpsToolCallback],
    ) -> ReactAgent {
        ReactAgent::new(
            "metrics_agent",
            &chat_model.model,
            METRICS_AGENT_PROMPT,
            self.agent_max_turns,
        )
        .with_description("负责 Prometheus 活跃告警、指标趋势和资源状态判断")
        .with_tool_metadata(
            self.build_method_tools_array(),
            self.tool_callback_names(tool_callbacks),
        )
        .with_allowed_tools(vec![
            "getCurrentDateTime".to_string(),
            "queryPrometheusAlerts".to_string(),
            "queryMetricTrend".to_string(),
        ])
    }

    fn build_logs_agent(
        &self,
        chat_model: &AiOpsChatModel,
        tool_callbacks: &[AiOpsToolCallback],
    ) -> ReactAgent {
        ReactAgent::new(
            "logs_agent",
            &chat_model.model,
            LOGS_AGENT_PROMPT,
            self.agent_max_turns,
        )
        .with_description("负责日志主题选择、错误日志检索和日志证据摘要")
        .with_tool_metadata(
            self.build_method_tools_array(),
            self.tool_callback_names(tool_callbacks),
        )
        .with_allowed_tools(vec![
            "getCurrentDateTime".to_string(),
            "getAvailableLogTopics".to_string(),
            "queryLogs".to_string(),
        ])
    }

    fn build_knowledge_agent(
        &self,
        chat_model: &AiOpsChatModel,
        tool_callbacks: &[AiOpsToolCallback],
    ) -> ReactAgent {
        ReactAgent::new(
            "knowledge_agent",
            &chat_model.model,
            KNOWLEDGE_AGENT_PROMPT,
            self.agent_max_turns,
        )
        .with_description("负责查询内部文档、Runbook 和历史处理建议")
        .with_tool_metadata(
            self.build_method_tools_array(),
            self.tool_callback_names(tool_callbacks),
        )
        .with_allowed_tools(vec![
            "getCurrentDateTime".to_string(),
            "queryInternalDocs".to_string(),
        ])
    }

    fn build_remediation_agent(
        &self,
        chat_model: &AiOpsChatModel,
        tool_callbacks: &[AiOpsToolCallback],
    ) -> ReactAgent {
        ReactAgent::new(
            "remediation_agent",
            &chat_model.model,
            REMEDIATION_AGENT_PROMPT,
            self.agent_max_turns,
        )
        .with_description("负责根据已有证据提出处理方案、风险和回滚建议")
        .with_tool_metadata(
            self.build_method_tools_array(),
            self.tool_callback_names(tool_callbacks),
        )
        .with_allowed_tools(vec!["getCurrentDateTime".to_string()])
    }

    fn build_supervisor_agent(
        &self,
        chat_model: &AiOpsChatModel,
        sub_agents: Vec<ReactAgent>,
    ) -> SupervisorAgent {
        SupervisorAgent::builder()
            .name("ai_ops_supervisor")
            .description("负责调度专业子 Agent 的多 Agent 控制器")
            .model(&chat_model.model)
            .system_prompt(SupervisorAgent::default_system_prompt())
            .sub_agents(sub_agents)
            .max_turns(self.max_rounds * 2)
            .build()
    }

    fn build_handoff_tools(
        &self,
        client: Arc<openai::CompletionsClient>,
        tool_callbacks: &[AiOpsToolCallback],
        collector: Option<Arc<EvidenceCollector>>,
        state: Arc<Mutex<OverAllState>>,
        sub_agents: &[ReactAgent],
    ) -> Vec<Box<dyn ToolDyn>> {
        sub_agents
            .iter()
            .cloned()
            .map(|agent| {
                Box::new(HandoffTool::new(
                    self.clone(),
                    Arc::clone(&client),
                    tool_callbacks.to_vec(),
                    collector.clone(),
                    Arc::clone(&state),
                    agent,
                )) as Box<dyn ToolDyn>
            })
            .collect()
    }

    fn build_method_tools_array(&self) -> Vec<String> {
        let mut tools = vec![
            "getCurrentDateTime".to_string(),
            "queryInternalDocs".to_string(),
            "queryPrometheusAlerts".to_string(),
            "queryMetricTrend".to_string(),
        ];
        if self.cls_mock_enabled {
            tools.push("queryLogs".to_string());
            tools.push("getAvailableLogTopics".to_string());
        }
        tools
    }

    fn tool_callback_names(&self, tool_callbacks: &[AiOpsToolCallback]) -> Vec<String> {
        tool_callbacks
            .iter()
            .map(|factory| factory().name())
            .collect()
    }

    pub fn extract_final_report(&self, state: &OverAllState) -> Option<String> {
        info!("开始提取最终报告...");
        let report = state
            .assistant_message("supervisor_output")
            .or_else(|| state.assistant_message("planner_plan"))?
            .text()
            .trim();
        if report.is_empty() {
            warn!("最终报告为空");
            return None;
        }
        info!("成功提取到最终报告，长度: {}", report.len());
        Some(report.to_string())
    }

    fn write_agent_output(&self, state: &mut OverAllState, agent: &ReactAgent, output: String) {
        if let Some(output_key) = agent.output_key() {
            state.insert_assistant_message(output_key, output);
        }
    }

    async fn run_agent(
        &self,
        client: &openai::CompletionsClient,
        tool_callbacks: &[AiOpsToolCallback],
        agent: &ReactAgent,
        message: String,
        state: &OverAllState,
        collector: Option<&Arc<EvidenceCollector>>,
    ) -> Result<String, AppError> {
        let preamble = agent.render_system_prompt(state);
        let runtime_agent = client
            .agent(agent.model())
            .name(agent.name())
            // 对齐 Java DashScopeModelConfig 的 AI Ops 参数：temperature 0.3 / maxToken 8000 / topP 0.9
            .temperature(0.3)
            .max_tokens(8000)
            // rig 0.36 的 AgentBuilder 无 top_p 方法，topP 经 additional_params 注入请求体
            .additional_params(serde_json::json!({ "top_p": 0.9 }))
            .preamble(&preamble)
            .tools(self.build_method_tools_for_agent(tool_callbacks, collector, agent))
            .build();

        runtime_agent
            .prompt(message)
            .max_turns(agent.max_turns())
            .await
            .map_err(|error: PromptError| map_prompt_error(error, agent.max_turns()))
    }
}

/// Rust/rig 对 Java 框架隐式运行上下文的显式封装。
struct AiOpsInvocationContext<'a> {
    client: Arc<openai::CompletionsClient>,
    tool_callbacks: &'a [AiOpsToolCallback],
    collector: Option<&'a Arc<EvidenceCollector>>,
}

impl<'a> AiOpsInvocationContext<'a> {
    fn new(
        chat_model: &AiOpsChatModel,
        tool_callbacks: &'a [AiOpsToolCallback],
        collector: Option<&'a Arc<EvidenceCollector>>,
    ) -> Result<Self, AppError> {
        let api_key = chat_model
            .api_key
            .as_ref()
            .ok_or_else(|| AppError::internal("DASHSCOPE_API_KEY 未配置"))?;

        let client = openai::CompletionsClient::builder()
            .api_key(api_key)
            .base_url(&chat_model.base_url)
            .build()
            .map_err(|error| {
                AppError::internal(format!("初始化 rig OpenAI client 失败: {error}"))
            })?;

        Ok(Self {
            client: Arc::new(client),
            tool_callbacks,
            collector,
        })
    }
}

#[derive(Clone, Deserialize)]
struct HandoffArgs {
    task: String,
    #[serde(default)]
    context: Option<String>,
}

struct HandoffTool {
    service: AiOpsService,
    client: Arc<openai::CompletionsClient>,
    tool_callbacks: Vec<AiOpsToolCallback>,
    collector: Option<Arc<EvidenceCollector>>,
    state: Arc<Mutex<OverAllState>>,
    agent: ReactAgent,
    tool_name: String,
}

impl HandoffTool {
    fn new(
        service: AiOpsService,
        client: Arc<openai::CompletionsClient>,
        tool_callbacks: Vec<AiOpsToolCallback>,
        collector: Option<Arc<EvidenceCollector>>,
        state: Arc<Mutex<OverAllState>>,
        agent: ReactAgent,
    ) -> Self {
        let tool_name = format!("transfer_to_{}", agent.name());
        Self {
            service,
            client,
            tool_callbacks,
            collector,
            state,
            agent,
            tool_name,
        }
    }

    fn format_message(&self, args: &HandoffArgs) -> String {
        let mut message = format!(
            "## Supervisor 委派任务\n{}\n\n请只完成你的专业职责，返回可供 Supervisor 综合的证据、分析和不确定性；不要输出最终用户报告。",
            args.task.trim()
        );
        if let Some(context) = args
            .context
            .as_deref()
            .map(str::trim)
            .filter(|ctx| !ctx.is_empty())
        {
            message.push_str("\n\n## Supervisor 提供的上下文\n");
            message.push_str(context);
        }
        message
    }
}

impl ToolDyn for HandoffTool {
    fn name(&self) -> String {
        self.tool_name.clone()
    }

    fn definition(
        &self,
        _prompt: String,
    ) -> Pin<Box<dyn Future<Output = ToolDefinition> + Send + '_>> {
        let description = format!(
            "Delegate a focused subtask to {}. Capability: {}. Returns that agent's evidence and analysis for the supervisor to synthesize.",
            self.agent.name(),
            if self.agent.description().trim().is_empty() {
                "specialized AI Ops analysis"
            } else {
                self.agent.description()
            }
        );
        Box::pin(async move {
            ToolDefinition {
                name: self.tool_name.clone(),
                description,
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "task": {
                            "type": "string",
                            "description": "The focused task for the sub-agent."
                        },
                        "context": {
                            "type": "string",
                            "description": "Optional known facts, evidence ids, and constraints from the supervisor."
                        }
                    },
                    "required": ["task"]
                }),
            }
        })
    }

    fn call(
        &self,
        args: String,
    ) -> Pin<Box<dyn Future<Output = Result<String, ToolError>> + Send + '_>> {
        Box::pin(async move {
            let parsed = match serde_json::from_str::<HandoffArgs>(&args) {
                Ok(parsed) if !parsed.task.trim().is_empty() => parsed,
                Ok(_) => {
                    return Ok(handoff_error_json(
                        self.agent.name(),
                        "INVALID_ARGUMENT",
                        "handoff task 不能为空",
                    ));
                }
                Err(error) => {
                    return Ok(handoff_error_json(
                        self.agent.name(),
                        "INVALID_ARGUMENT",
                        &format!("无法解析 handoff 参数: {error}"),
                    ));
                }
            };

            let state_snapshot = self.state.lock().await.clone();
            let message = self.format_message(&parsed);
            match self
                .service
                .run_agent(
                    self.client.as_ref(),
                    &self.tool_callbacks,
                    &self.agent,
                    message,
                    &state_snapshot,
                    self.collector.as_ref(),
                )
                .await
            {
                Ok(output) => {
                    let mut state = self.state.lock().await;
                    state.record_agent_output(self.agent.name(), output.clone());
                    self.service
                        .write_agent_output(&mut state, &self.agent, output.clone());
                    Ok(serde_json::json!({
                        "success": true,
                        "agent": self.agent.name(),
                        "output": output,
                    })
                    .to_string())
                }
                Err(error) => Ok(handoff_error_json(
                    self.agent.name(),
                    "AGENT_CALL_FAILED",
                    &error.to_string(),
                )),
            }
        })
    }
}

fn handoff_error_json(agent_name: &str, error_code: &str, message: &str) -> String {
    serde_json::json!({
        "success": false,
        "agent": agent_name,
        "errorCode": error_code,
        "message": message,
    })
    .to_string()
}

struct GuardedMcpTool {
    inner: Box<dyn ToolDyn>,
    dependency: String,
}

impl GuardedMcpTool {
    fn new(inner: Box<dyn ToolDyn>, dependency: String) -> Self {
        Self { inner, dependency }
    }
}

impl ToolDyn for GuardedMcpTool {
    fn name(&self) -> String {
        self.inner.name()
    }

    fn definition(
        &self,
        prompt: String,
    ) -> Pin<Box<dyn Future<Output = rig::completion::ToolDefinition> + Send + '_>> {
        self.inner.definition(prompt)
    }

    fn call(
        &self,
        args: String,
    ) -> Pin<Box<dyn Future<Output = Result<String, rig::tool::ToolError>> + Send + '_>> {
        Box::pin(async move {
            let tool_name = self.inner.name();
            match self.inner.call(args).await {
                Ok(output) => Ok(output),
                Err(error) => {
                    warn!(
                        "MCP 工具调用失败，已转换为依赖错误结果, dependency: {}, tool: {}, error: {}",
                        self.dependency, tool_name, error
                    );
                    Ok(dependency_error_json(
                        &self.dependency,
                        &tool_name,
                        "DEPENDENCY_ERROR",
                        &error.to_string(),
                    ))
                }
            }
        })
    }
}

fn mcp_dependency_name(haystack: &str) -> Option<String> {
    let haystack = haystack.to_lowercase();
    if haystack.contains("tavily") {
        return Some("mcp-tavily".to_string());
    }
    if haystack.contains("dbhub")
        || haystack.contains("bytebase")
        || haystack.contains("execute_sql")
        || haystack.contains("execute sql")
        || haystack.contains("readonly sql")
        || haystack.contains("read-only sql")
        || haystack.contains("database schema")
        || haystack.contains("list_tables")
        || haystack.contains("describe_table")
    {
        return Some("mcp-dbhub".to_string());
    }
    None
}

fn dependency_error_json(
    dependency: &str,
    operation: &str,
    error_code: &str,
    message: &str,
) -> String {
    serde_json::json!({
        "success": false,
        "errorCode": error_code,
        "dependency": dependency,
        "tool": operation,
        "message": if message.trim().is_empty() {
            "MCP tool dependency failed"
        } else {
            message
        },
    })
    .to_string()
}

fn map_prompt_error(error: PromptError, max_turns: usize) -> AppError {
    match error {
        PromptError::MaxTurnsError { .. } => AppError::internal(format!(
            "rig agent 调用失败: reached max turn limit: {max_turns}"
        )),
        error => AppError::internal(format!("rig agent 调用失败: {error}")),
    }
}

/// 初始任务提示（对应 Java `taskPrompt`）。
const TASK_PROMPT: &str = r##"你是企业级 SRE Supervisor，接到了自动化告警排查任务。
请按需通过 handoff 工具委派 metrics_agent、logs_agent、knowledge_agent、remediation_agent 等专业子 Agent 收集证据和建议，然后由你综合输出最终《告警分析报告》。
禁止编造虚假数据；连续多次查询失败需诚实反馈无法完成的原因。报告结论必须绑定 evidence id；证据不足时必须显式说明缺失证据。

最终输出必须是纯 Markdown，直接从 "# 告警分析报告" 开始，包含：
- 活跃告警清单
- 告警根因分析
- 处理方案执行或处理建议
- 结论、置信度、缺失证据、后续建议、风险评估
"##;

const METRICS_AGENT_PROMPT: &str = r##"你是 metrics_agent，负责 Prometheus 活跃告警、指标趋势和资源状态判断。

职责：
- 根据 Supervisor 委派任务和 {input} 中的告警上下文，查询必要的 Prometheus 指标或活跃告警。
- 对 CPU、内存、错误率、P99 延迟、重启次数等结论，优先调用 queryMetricTrend。
- 只有需要确认全局告警面、关联告警或 firing 状态时，才调用 queryPrometheusAlerts。
- 不要编造数据；工具失败或数据缺失必须如实说明。
- 工具返回中若包含 _diagnosisEvidenceId，输出中必须原样标注 [evidence: ev-xxxx]。

输出给 Supervisor 的内容应包含：关键指标、时间窗口、趋势判断、证据 id、仍缺少的指标证据。
"##;

const LOGS_AGENT_PROMPT: &str = r##"你是 logs_agent，负责日志主题选择、错误日志检索和日志证据摘要。

职责：
- 根据告警类型和 Supervisor 委派任务选择日志主题；能推断主题时直接 queryLogs，不能推断时才 getAvailableLogTopics。
- 主题推断：CPU/内存/磁盘 -> system-metrics；错误率/服务不可用/慢响应/下游依赖 -> application-logs；慢 SQL -> database-slow-query；OOMKilled/CrashLoop/重启/容器崩溃 -> system-events。
- 查询日志时使用连字符 region（如 ap-guangzhou）；不确定时省略 region 使用默认值。
- 不要编造日志；无结果或失败必须说明查询参数和失败原因。
- 工具返回中若包含 _diagnosisEvidenceId，输出中必须原样标注 [evidence: ev-xxxx]。

输出给 Supervisor 的内容应包含：查询主题、关键词/时间范围、关键日志片段摘要、证据 id、缺失证据。
"##;

const KNOWLEDGE_AGENT_PROMPT: &str = r##"你是 knowledge_agent，负责查询内部文档、Runbook 和历史处理建议。

职责：
- 只围绕 Supervisor 委派的问题查询 queryInternalDocs。
- 将文档、Runbook、历史案例中的建议与当前告警上下文区分开，不要把文档建议当作已验证事实。
- 工具返回中若包含 _diagnosisEvidenceId，输出中必须原样标注 [evidence: ev-xxxx]。

输出给 Supervisor 的内容应包含：相关文档结论、适用条件、处理建议、证据 id、不能确认的部分。
"##;

const REMEDIATION_AGENT_PROMPT: &str = r##"你是 remediation_agent，负责基于已有证据提出处理方案、风险和回滚建议。

职责：
- 不直接查询指标、日志或文档；只基于 Supervisor 提供的上下文、子 Agent 结果和当前时间进行分析。
- 将建议分为立即缓解、根因修复、验证步骤、回滚方案。
- 每个建议都要说明依赖的证据；证据不足时标注“证据不足”。
- 不要编造已执行动作。

输出给 Supervisor 的内容应包含：处理建议、预期效果、风险、回滚方式、仍需补充的证据。
"##;

/// Planner Agent 系统提示词（对应 Java `buildPlannerPrompt`）。
#[allow(dead_code)]
const PLANNER_PROMPT: &str = r##"你是 Planner Agent，同时承担 Replanner 角色，负责：
1. 读取当前输入任务 {input} 以及 Executor 的最近反馈 {executor_feedback}。
2. 分析 Prometheus 告警、日志、内部文档等信息，制定可执行的下一步步骤。
3. 在执行阶段，输出 JSON，包含 decision (PLAN|EXECUTE|FINISH)、step 描述、预期要调用的工具、以及必要的上下文。
4. 调用任何腾讯云日志/主题相关工具时，region 参数必须使用连字符格式（如 ap-guangzhou），若不确定请省略以使用默认值。
5. 严格禁止编造数据，只能引用工具返回的真实内容；如果连续 3 次调用同一工具仍失败或返回空结果，需停止该方向并在最终报告的结论部分说明"无法完成"的原因。
6. 遇到 CPU、内存、错误率、P99 延迟、重启次数相关告警时，必须规划调用 queryMetricTrend 查询 15m/1h/6h 中最相关窗口的趋势，再基于趋势证据判断根因。

## 工具调用策略（减少冗余）

- 当前 Incident 告警上下文是首要事实源；如果输入中已经包含告警名称、级别、实例、服务、摘要、标签或注解，优先基于这些字段规划诊断。
- queryPrometheusAlerts 是条件工具，用于确认全局告警面、发现关联告警或校验当前告警是否仍在 firing；不要把 queryPrometheusAlerts 作为默认第一步。
- 推荐证据顺序：当前 Incident 告警上下文 -> 相似历史故障/已提供证据 -> queryMetricTrend -> queryLogs -> queryInternalDocs -> 条件性 queryPrometheusAlerts。
- 只有无法根据告警类型推断日志主题时，才调用 getAvailableLogTopics；能推断时直接规划 queryLogs。
- 日志主题推断规则：CPU/内存/磁盘使用率 -> system-metrics；错误率/服务不可用/慢响应/下游依赖 -> application-logs；慢 SQL/数据库性能 -> database-slow-query；OOMKilled/CrashLoop/重启/容器崩溃 -> system-events。
- Tavily MCP 仅用于查询外部公开资料、官方文档、错误码说明和组件版本差异；不能用外部搜索结果覆盖 Incident、指标、日志或内部知识库中的事实。
- 数据库 MCP 仅用于只读验证业务状态、配置、事件记录和 CMDB 信息；必须先说明要验证的问题，再规划有限范围查询。
- 不要为了“补全流程”调用无关工具；每个工具调用都必须能回答当前诊断问题，并在报告中形成可引用 evidence。

## 最终报告输出要求（CRITICAL）

当 decision=FINISH 时，你必须：
1. **不要输出 JSON 格式**
2. **直接输出完整的 Markdown 格式报告文本**
3. **报告必须严格遵循以下模板**：

```
# 告警分析报告

---

## 📋 活跃告警清单

| 告警名称 | 级别 | 目标服务 | 首次触发时间 | 最新触发时间 | 状态 |
|---------|------|----------|-------------|-------------|------|
| [告警1名称] | [级别] | [服务名] | [时间] | [时间] | 活跃 |
| [告警2名称] | [级别] | [服务名] | [时间] | [时间] | 活跃 |

---

## 🔍 告警根因分析1 - [告警名称]

### 告警详情
- **告警级别**: [级别]
- **受影响服务**: [服务名]
- **持续时间**: [X分钟]

### 症状描述
[根据监控指标描述症状]

### 日志证据
[引用查询到的关键日志]

### 根因结论
[基于证据得出的根本原因]

---

## 🛠️ 处理方案执行1 - [告警名称]

### 已执行的排查步骤
1. [步骤1]
2. [步骤2]

### 处理建议
[给出具体的处理建议]

### 预期效果
[说明预期的效果]

---

## 🔍 告警根因分析2 - [告警名称]
[如果有第2个告警，重复上述格式]

---

## 📊 结论

### 整体评估
[总结所有告警的整体情况]

### 关键发现
- [发现1]
- [发现2]

### 置信度
[高/中/低；说明置信度来自哪些 evidence id，以及哪些结论仍缺少证据]

### 缺失证据
- [缺失证据1；如果没有，写“无”]
- [缺失证据2；如果没有，写“无”]

### 后续建议
1. [建议1]
2. [建议2]

### 风险评估
[评估当前风险等级和影响范围]
```

**重要提醒**：
- 最终输出必须是纯 Markdown 文本，不要包含 JSON 结构
- 不要使用 "finalReport": "..." 这样的格式
- 直接从 "# 告警分析报告" 开始输出
- 所有内容必须基于工具查询的真实数据，严禁编造
- 工具返回中若包含 _diagnosisEvidenceId，报告中引用对应结论时必须标注 [evidence: ev-xxxx]
- 每个根因、症状和处理建议必须绑定 evidence id；不能被 evidence id 支撑的判断必须写“证据不足”
- 报告必须明确写出“置信度”和“缺失证据”
- 资源、错误率、延迟、重启类结论必须引用 queryMetricTrend 的趋势 evidence id；如果趋势查询失败，必须明确说明趋势证据缺失
- 根因、症状和处理建议应尽量引用 evidence id；无法拿到证据 id 时，必须说明证据来源缺失
- 如果某个步骤失败，在结论中如实说明，不要跳过
"##;

/// Executor Agent 系统提示词（对应 Java `buildExecutorPrompt`）。
#[allow(dead_code)]
const EXECUTOR_PROMPT: &str = r##"你是 Executor Agent，负责读取 Planner 最新输出 {planner_plan}，只执行其中的第一步。
- 确认步骤所需的工具与参数，尤其是 region 参数要使用连字符格式（ap-guangzhou）；若 Planner 未给出则使用默认区域。
- 调用相应的工具并收集结果，如工具返回错误或空数据，需要将失败原因、请求参数一并记录，并停止进一步调用该工具（同一工具失败达到 3 次时应直接返回 FAILED）。
- 执行 CPU、内存、错误率、P99 延迟或重启次数排查时，必须优先调用 queryMetricTrend，传入 metric、service、instance、window、step，获取趋势摘要后再继续日志或文档查询。
- 已有明确告警上下文时，不要重复查询活动告警；只有 Planner 明确要求确认全局告警面、关联告警或 firing 状态时，才调用 queryPrometheusAlerts。
- 能从告警类型推断日志主题时，直接调用 queryLogs；只有 Planner 未给出主题且无法从告警类型推断时，才调用 getAvailableLogTopics。
- 日志主题推断规则：CPU/内存/磁盘使用率 -> system-metrics；错误率/服务不可用/慢响应/下游依赖 -> application-logs；慢 SQL/数据库性能 -> database-slow-query；OOMKilled/CrashLoop/重启/容器崩溃 -> system-events。
- 调用 Tavily MCP 时，只能查询公开资料、官方文档、错误码或版本差异，并在反馈中注明其属于外部参考。
- 调用数据库 MCP 时只能执行只读查询；禁止执行 INSERT / UPDATE / DELETE / DROP / ALTER / TRUNCATE / CREATE 等写入或结构变更语句，查询必须限制字段、时间范围和返回行数。
- 将日志、指标、文档等证据整理成结构化摘要，标注对应的告警名称或资源，方便 Planner 填充"告警根因分析 / 处理方案执行"章节。
- 工具返回中若包含 _diagnosisEvidenceId，必须把该 id 原样写入 evidence 列表，格式为 [evidence: ev-xxxx]。
- 以 JSON 形式返回执行状态、证据以及给 Planner 的建议，写入 executor_feedback，严禁编造未实际查询到的内容。


输出示例：
{
  "status": "SUCCESS",
  "summary": "近1小时未见 error 日志，仅有 info",
  "evidence": "...",
  "nextHint": "建议转向高占用进程"
}
"##;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::AppConfig,
        services::agent_runtime::{AssistantMessage, StateValue},
    };
    use std::{net::Ipv4Addr, time::Duration};

    fn test_config() -> AppConfig {
        AppConfig {
            host: Ipv4Addr::new(127, 0, 0, 1),
            port: 3000,
            allowed_origin: "*".to_string(),
            request_timeout: Duration::from_secs(30),
            log_filter: "info".to_string(),
            static_dir: "./static".to_string(),
            redis_url: None,
            chat_history_path: "./data/test-chat-history".to_string(),
            session_ttl_secs: 3600,
            dashscope_api_key: None,
            dashscope_base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1".to_string(),
            dashscope_api_base_url: "https://dashscope.aliyuncs.com/api/v1".to_string(),
            dashscope_responses_rectifier_enabled: false,
            dashscope_chat_model: "qwen-plus".to_string(),
            chat_agent_max_turns: 6,
            dashscope_embedding_model: "text-embedding-v4".to_string(),
            dashscope_rerank_model: "gte-rerank-v2".to_string(),
            dashscope_rerank_url:
                "https://dashscope.aliyuncs.com/api/v1/services/rerank/text-rerank/text-rerank"
                    .to_string(),
            milvus_host: "localhost".to_string(),
            milvus_port: 19530,
            milvus_username: String::new(),
            milvus_password: String::new(),
            milvus_database: "default".to_string(),
            milvus_timeout_ms: 5000,
            rag_candidate_k: 20,
            rag_search_ef: 64,
            upload_path: "./data/uploads".to_string(),
            upload_allowed_extensions: vec!["md".to_string(), "txt".to_string()],
            document_chunk_max_size: 800,
            document_chunk_overlap: 120,
            private_memory_recall_enabled: false,
            private_memory_recall_top_k: 5,
            private_memory_store_path: "./data/private-memory".to_string(),
            prometheus_base_url: "http://localhost:9090".to_string(),
            prometheus_timeout_secs: 5,
            prometheus_mock_enabled: true,
            cls_mock_enabled: true,
            ai_ops_chat_model: "qwen-plus".to_string(),
            ai_ops_agent_max_turns: 12,
            ai_ops_max_rounds: 8,
        }
    }

    #[test]
    fn extract_final_report_returns_none_when_state_missing_planner_plan() {
        let service = AiOpsService::new(&test_config());
        let state = OverAllState::new();

        assert_eq!(service.extract_final_report(&state), None);
    }

    #[test]
    fn extract_final_report_returns_report_from_assistant_message() {
        let service = AiOpsService::new(&test_config());
        let mut state = OverAllState::new();
        state.insert_assistant_message("supervisor_output", "# 告警分析报告\n\n测试报告内容");

        assert_eq!(
            service.extract_final_report(&state).as_deref(),
            Some("# 告警分析报告\n\n测试报告内容")
        );
    }

    #[test]
    fn extract_final_report_falls_back_to_planner_plan_for_compatibility() {
        let service = AiOpsService::new(&test_config());
        let mut state = OverAllState::new();
        state.insert_assistant_message("planner_plan", "# 旧报告");

        assert_eq!(
            service.extract_final_report(&state).as_deref(),
            Some("# 旧报告")
        );
    }

    #[test]
    fn extract_final_report_returns_none_when_report_is_not_assistant_message() {
        let service = AiOpsService::new(&test_config());
        let mut state = OverAllState::new();
        state.insert(
            "supervisor_output",
            StateValue::Text("just a string".to_string()),
        );

        assert_eq!(service.extract_final_report(&state), None);
    }

    #[test]
    fn build_ai_ops_sub_agents_use_specialized_names_and_tools() {
        let service = AiOpsService::new(&test_config());
        let chat_model = service.chat_model();
        let tool_callbacks = service.tool_callbacks();

        let agents = service.build_ai_ops_sub_agents(&chat_model, &tool_callbacks);

        assert!(agents.iter().any(|agent| agent.name() == "metrics_agent"));
        assert!(agents.iter().any(|agent| agent.name() == "logs_agent"));
        assert!(agents.iter().any(|agent| agent.name() == "knowledge_agent"));
        assert!(agents
            .iter()
            .any(|agent| agent.name() == "remediation_agent"));

        let metrics = agents
            .iter()
            .find(|agent| agent.name() == "metrics_agent")
            .expect("metrics agent");
        assert!(metrics.allows_tool("queryMetricTrend"));
        assert!(!metrics.allows_tool("queryLogs"));

        let remediation = agents
            .iter()
            .find(|agent| agent.name() == "remediation_agent")
            .expect("remediation agent");
        assert!(remediation.allows_tool("getCurrentDateTime"));
        assert!(!remediation.allows_tool("queryMetricTrend"));
    }

    #[test]
    fn build_ai_ops_sub_agents_skip_logs_agent_when_cls_disabled() {
        let mut config = test_config();
        config.cls_mock_enabled = false;
        let service = AiOpsService::new(&config);
        let chat_model = service.chat_model();
        let tool_callbacks = service.tool_callbacks();

        let agents = service.build_ai_ops_sub_agents(&chat_model, &tool_callbacks);

        assert!(!agents.iter().any(|agent| agent.name() == "logs_agent"));
    }

    #[test]
    fn build_method_tools_for_agent_filters_by_allowed_tools() {
        let service = AiOpsService::new(&test_config());
        let agent = ReactAgent::new("metrics_agent", "qwen-plus", "metrics", 12)
            .with_allowed_tools(vec!["queryMetricTrend".to_string()]);

        let tool_names = service
            .build_method_tools_for_agent(&[], None, &agent)
            .into_iter()
            .map(|tool| tool.name())
            .collect::<Vec<_>>();

        assert_eq!(tool_names, vec!["queryMetricTrend".to_string()]);
    }

    #[tokio::test]
    async fn handoff_tool_exposes_transfer_name_and_schema() {
        let service = AiOpsService::new(&test_config());
        let client = openai::CompletionsClient::builder()
            .api_key("test")
            .base_url("https://example.com")
            .build()
            .expect("client");
        let agent = ReactAgent::new("metrics_agent", "qwen-plus", "metrics", 12)
            .with_description("负责指标诊断");
        let tool = HandoffTool::new(
            service,
            Arc::new(client),
            Vec::new(),
            None,
            Arc::new(Mutex::new(OverAllState::new())),
            agent,
        );

        let definition = tool.definition(String::new()).await;

        assert_eq!(tool.name(), "transfer_to_metrics_agent");
        assert_eq!(definition.name, "transfer_to_metrics_agent");
        assert!(definition.description.contains("负责指标诊断"));
        assert_eq!(
            definition
                .parameters
                .get("required")
                .and_then(|value| value.as_array())
                .and_then(|values| values.first())
                .and_then(|value| value.as_str()),
            Some("task")
        );
    }

    #[tokio::test]
    async fn execute_ai_ops_analysis_without_api_key_returns_config_error() {
        let service = AiOpsService::new(&test_config());
        let chat_model = service.chat_model();
        let tool_callbacks = service.tool_callbacks();

        let error = service
            .execute_ai_ops_analysis_with_context(&chat_model, &tool_callbacks, Some("CPU 告警"))
            .await
            .expect_err("缺少 DASHSCOPE_API_KEY 时应返回错误");

        assert!(
            matches!(error, AppError::Internal(message) if message == "DASHSCOPE_API_KEY 未配置")
        );
    }

    #[test]
    fn extract_final_report_returns_none_for_blank_assistant_message() {
        let service = AiOpsService::new(&test_config());
        let mut state = OverAllState::new();
        state.insert(
            "supervisor_output",
            StateValue::AssistantMessage(AssistantMessage::new("  ")),
        );

        assert_eq!(service.extract_final_report(&state), None);
    }
}
