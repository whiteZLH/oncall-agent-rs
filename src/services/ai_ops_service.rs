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
        chat_service::{GetCurrentDateTimeTool, QueryInternalDocsTool},
        diagnosis_report_service::augment_alert_context,
        incident_service::EvidenceCollector,
        vector_search_service::VectorSearchService,
    },
};
use rig::{
    client::CompletionClient,
    completion::{Prompt, PromptError},
    providers::openai,
    tool::ToolDyn,
};
use serde_json::Value;
use std::sync::Arc;
use tracing::{info, warn};

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

    /// 动态构建方法工具集合。
    /// 始终包含：时间、内部文档、Prometheus 告警、指标趋势；
    /// 当 `cls_mock_enabled` 为真时（对应 Java 的 mock 模式注册 QueryLogsTools），
    /// 追加日志查询与日志主题工具。
    fn build_method_tools(&self, collector: Option<&Arc<EvidenceCollector>>) -> Vec<Box<dyn ToolDyn>> {
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

    /// 执行 AI Ops 告警分析流程。
    ///
    /// * `alert_context` —— 告警上下文信息（可为空）
    ///
    /// 返回最终的《告警分析报告》Markdown 文本。
    pub async fn execute_ai_ops_analysis(
        &self,
        alert_context: Option<&str>,
    ) -> Result<String, AppError> {
        self.execute_internal(alert_context, None).await
    }

    /// 在指定诊断 run 上下文中执行 AI Ops（对齐 Java 5 参数 `executeAiOpsAnalysis`）：
    /// 先用已累积证据增强告警上下文（`augmentAlertContext`），并以 `RecordingTool`
    /// 包装工具，实时记录证据并向工具返回注入 `_diagnosisEvidenceId`。
    pub async fn execute_ai_ops_analysis_for_run(
        &self,
        alert_context: Option<&str>,
        collector: Arc<EvidenceCollector>,
    ) -> Result<String, AppError> {
        let evidence = collector.snapshot_evidence();
        let augmented = augment_alert_context(alert_context.unwrap_or(""), &evidence);
        self.execute_internal(Some(&augmented), Some(collector)).await
    }

    async fn execute_internal(
        &self,
        alert_context: Option<&str>,
        collector: Option<Arc<EvidenceCollector>>,
    ) -> Result<String, AppError> {
        info!("开始执行 AI Ops 多 Agent 协作流程");

        let Some(api_key) = self.api_key.clone() else {
            // 无 API key（开发环境）：返回占位报告，保证接口可用
            let ctx = alert_context.unwrap_or("");
            return Ok(format!(
                "# 告警分析报告\n\n（未配置模型 API Key，无法执行多 Agent 分析）\n\n## 当前告警上下文\n{ctx}"
            ));
        };

        let client = openai::CompletionsClient::builder()
            .api_key(&api_key)
            .base_url(&self.base_url)
            .build()
            .map_err(|error| AppError::internal(format!("初始化 rig OpenAI client 失败: {error}")))?;

        // 构建任务提示（注入告警上下文）
        let mut task = TASK_PROMPT.to_string();
        if let Some(ctx) = alert_context.map(str::trim).filter(|ctx| !ctx.is_empty()) {
            task.push_str("\n\n## 当前告警上下文\n");
            task.push_str(ctx);
            task.push_str("\n\n请基于以上告警上下文进行分析和处理。");
            info!("已注入告警上下文，长度: {}", ctx.chars().count());
        }

        // 每个 planner+executor 对算 2 步，max_steps 与原来的 max_rounds 预算大致相当
        let max_steps = self.max_rounds * 2;
        info!(
            "调用 Supervisor 开始编排，最大路由步数: {}, 单 Agent 最大工具轮次: {}",
            max_steps, self.agent_max_turns
        );

        // planner_plan / executor_feedback 对应 Java OverAllState 的两个 outputKey
        let mut planner_plan = String::new();
        let mut executor_feedback = String::new();
        let mut last_planner = String::new();

        for step in 0..max_steps {
            // 1) Supervisor 路由：由大模型在 Planner / Executor / FINISH 间决定下一步
            let action = self
                .run_supervisor(&client, &task, &planner_plan, &executor_feedback)
                .await?;
            info!(
                "[AI Ops] 第 {} 步：Supervisor 路由决策 = {:?}",
                step + 1,
                action
            );

            // FINISH：planner 已产出最终报告则直接收尾；否则逼一次 planner 收尾
            let action = if matches!(action, SupervisorAction::Finish) {
                if planner_finished(&planner_plan) {
                    info!("[AI Ops] Supervisor 判定 FINISH，提取最终报告");
                    return Ok(extract_report(&planner_plan));
                }
                info!("[AI Ops] Supervisor 判定 FINISH 但尚无最终报告，转交 Planner 收尾");
                SupervisorAction::Planner
            } else {
                action
            };

            if matches!(action, SupervisorAction::Executor) {
                // Executor：执行 Planner 计划的第一步
                if planner_plan.is_empty() {
                    warn!("[AI Ops] Supervisor 选择 Executor 但尚无 Planner 计划，跳过本步");
                    continue;
                }
                info!("[AI Ops] 第 {} 步：调用 Executor Agent 执行首个步骤", step + 1);
                let executor_preamble = EXECUTOR_PROMPT.replace("{planner_plan}", &planner_plan);
                let executor_message =
                    format!("Planner 最新输出如下，请只执行其中的第一步：\n{planner_plan}");
                executor_feedback = self
                    .run_agent(
                        &client,
                        "executor_agent",
                        &executor_preamble,
                        executor_message,
                        collector.as_ref(),
                    )
                    .await?;
            } else {
                // Planner：拆解 / 再规划（Finish 已在上方转为 Planner 收尾）
                let planner_preamble = build_planner_preamble(&task, &executor_feedback);
                let planner_message = if planner_plan.is_empty() {
                    task.clone()
                } else {
                    format!(
                        "Executor 最近反馈：\n{executor_feedback}\n\n请基于反馈继续规划；若证据已充分，请直接输出完整 Markdown《告警分析报告》（decision=FINISH，纯 Markdown，不要 JSON）。"
                    )
                };

                info!("[AI Ops] 第 {} 步：调用 Planner Agent", step + 1);
                planner_plan = self
                    .run_agent(
                        &client,
                        "planner_agent",
                        &planner_preamble,
                        planner_message,
                        collector.as_ref(),
                    )
                    .await?;
                last_planner = planner_plan.clone();

                if planner_finished(&planner_plan) {
                    info!("[AI Ops] Planner 输出 FINISH，提取最终报告");
                    return Ok(extract_report(&planner_plan));
                }
            }
        }

        // 达到最大路由步数：强制 Planner 收尾输出报告
        warn!("[AI Ops] 达到最大路由步数 {}，强制收尾输出报告", max_steps);
        let planner_preamble = build_planner_preamble(&task, &executor_feedback);
        let forced = self
            .run_agent(
                &client,
                "planner_agent",
                &planner_preamble,
                "已达到最大编排轮次，请立即基于已有证据输出完整 Markdown《告警分析报告》（decision=FINISH，纯 Markdown，不要 JSON）。如证据不足，请在报告中如实说明缺失证据。".to_string(),
                collector.as_ref(),
            )
            .await;

        match forced {
            Ok(report) if !report.trim().is_empty() => Ok(extract_report(&report)),
            _ => Ok(extract_report(&last_planner)),
        }
    }

    async fn run_agent(
        &self,
        client: &openai::CompletionsClient,
        name: &str,
        preamble: &str,
        message: String,
        collector: Option<&Arc<EvidenceCollector>>,
    ) -> Result<String, AppError> {
        let agent = client
            .agent(&self.model)
            .name(name)
            // 对齐 Java DashScopeModelConfig 的 AI Ops 参数：temperature 0.3 / maxToken 8000 / topP 0.9
            .temperature(0.3)
            .max_tokens(8000)
            // rig 0.36 的 AgentBuilder 无 top_p 方法，topP 经 additional_params 注入请求体
            .additional_params(serde_json::json!({ "top_p": 0.9 }))
            .preamble(preamble)
            .tools(self.build_method_tools(collector))
            .build();

        agent
            .prompt(message)
            .max_turns(self.agent_max_turns)
            .await
            .map_err(|error: PromptError| map_prompt_error(error, self.agent_max_turns))
    }

    /// Supervisor 路由：由大模型在 Planner / Executor / FINISH 之间选择下一步动作。
    /// 对应 Java `SupervisorAgent` 的调度职责；以一次轻量 LLM 调用产出路由决策，
    /// 本身不挂方法工具（只路由，不调诊断工具）。
    async fn run_supervisor(
        &self,
        client: &openai::CompletionsClient,
        task: &str,
        planner_plan: &str,
        executor_feedback: &str,
    ) -> Result<SupervisorAction, AppError> {
        let has_plan = !planner_plan.trim().is_empty();
        let preamble = format!("{SUPERVISOR_PROMPT}\n\n{SUPERVISOR_ROUTING_SUFFIX}");

        let planner_state = if has_plan { planner_plan } else { "（暂无）" };
        let feedback_state = if executor_feedback.trim().is_empty() {
            "（暂无）"
        } else {
            executor_feedback
        };
        let message = format!(
            "## 当前任务\n{task}\n\n## Planner 最新输出\n{planner_state}\n\n## Executor 最新反馈\n{feedback_state}\n\n请决定下一步动作，并只输出 JSON。"
        );

        let agent = client
            .agent(&self.model)
            .name("ai_ops_supervisor")
            // 与 planner/executor 同样对齐 Java AI Ops 模型参数：temperature 0.3 / maxToken 8000 / topP 0.9
            .temperature(0.3)
            .max_tokens(8000)
            .additional_params(serde_json::json!({ "top_p": 0.9 }))
            .preamble(&preamble)
            .build();

        let out = agent
            .prompt(message)
            .await
            .map_err(|error: PromptError| map_prompt_error(error, self.agent_max_turns))?;

        Ok(parse_supervisor_action(&out, has_plan))
    }
}

/// Supervisor 的路由动作（对应 Java SupervisorAgent 在 planner/executor/FINISH 间的选择）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SupervisorAction {
    Planner,
    Executor,
    Finish,
}

/// 解析 Supervisor 的路由决策。期望输出形如 `{"next":"PLANNER|EXECUTOR|FINISH"}`。
/// 容错截取首个 `{` 到末个 `}` 的 JSON 子串；解析失败时回退启发式：
/// 尚无 Planner 计划 → Planner，否则 → Executor，并 `warn!` 记录原始输出。
fn parse_supervisor_action(out: &str, has_plan: bool) -> SupervisorAction {
    let content = strip_code_fences(out);
    let json_slice = match (content.find('{'), content.rfind('}')) {
        (Some(start), Some(end)) if end > start => &content[start..=end],
        _ => content,
    };
    if let Ok(value) = serde_json::from_str::<Value>(json_slice) {
        if let Some(next) = value.get("next").and_then(Value::as_str) {
            let next = next.trim();
            if next.eq_ignore_ascii_case("PLANNER") {
                return SupervisorAction::Planner;
            }
            if next.eq_ignore_ascii_case("EXECUTOR") {
                return SupervisorAction::Executor;
            }
            if next.eq_ignore_ascii_case("FINISH") {
                return SupervisorAction::Finish;
            }
        }
    }

    let fallback = if has_plan {
        SupervisorAction::Executor
    } else {
        SupervisorAction::Planner
    };
    warn!(
        "[AI Ops] 无法解析 Supervisor 路由决策，回退为 {:?}；原始输出: {}",
        fallback, out
    );
    fallback
}

fn build_planner_preamble(task: &str, executor_feedback: &str) -> String {
    let feedback = if executor_feedback.trim().is_empty() {
        "（暂无）"
    } else {
        executor_feedback
    };
    PLANNER_PROMPT
        .replace("{input}", task)
        .replace("{executor_feedback}", feedback)
}

/// 去除 Markdown / JSON 代码围栏，返回内部内容。
fn strip_code_fences(text: &str) -> &str {
    let trimmed = text.trim();
    if let Some(rest) = trimmed.strip_prefix("```") {
        // 跳过首行围栏标记（```json / ```markdown / ```）
        let body = rest.split_once('\n').map(|(_, body)| body).unwrap_or("");
        return body.trim().strip_suffix("```").unwrap_or(body).trim();
    }
    trimmed
}

/// 判断 Planner 是否已输出最终报告（FINISH）。
/// 与 Java 一致：执行阶段输出 JSON（decision != FINISH）；FINISH 时直接输出 Markdown。
fn planner_finished(out: &str) -> bool {
    let content = strip_code_fences(out);
    if content.starts_with('{') {
        if let Ok(value) = serde_json::from_str::<Value>(content) {
            return value
                .get("decision")
                .and_then(Value::as_str)
                .map(|decision| decision.eq_ignore_ascii_case("FINISH"))
                .unwrap_or(false);
        }
        return false;
    }
    // 非 JSON：视为最终 Markdown 报告
    true
}

/// 提取最终报告文本。兼容 JSON 包裹 `finalReport` 字段与纯 Markdown 两种形式。
fn extract_report(out: &str) -> String {
    let content = strip_code_fences(out);
    if content.starts_with('{') {
        if let Ok(value) = serde_json::from_str::<Value>(content) {
            if let Some(report) = value.get("finalReport").and_then(Value::as_str) {
                return report.to_string();
            }
        }
    }
    content.to_string()
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
const TASK_PROMPT: &str = "你是企业级 SRE，接到了自动化告警排查任务。请结合工具调用，执行**规划→执行→再规划**的闭环，并最终按照固定模板输出《告警分析报告》。禁止编造虚假数据，如连续多次查询失败需诚实反馈无法完成的原因。报告结论必须绑定 evidence id；证据不足时必须显式说明缺失证据。";

/// Planner Agent 系统提示词（对应 Java `buildPlannerPrompt`）。
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

/// Supervisor 系统提示词（对应 Java `buildSupervisorSystemPrompt`）。
/// 由 `run_supervisor` 用作 preamble，配合 `SUPERVISOR_ROUTING_SUFFIX` 产出路由决策。
const SUPERVISOR_PROMPT: &str = r##"你是 AI Ops Supervisor，负责调度 planner_agent 与 executor_agent：
1. 当需要拆解任务或重新制定策略时，调用 planner_agent。
2. 当 planner_agent 输出 decision=EXECUTE 时，调用 executor_agent 执行第一步。
3. 根据 executor_agent 的反馈，评估是否需要再次调用 planner_agent，直到 decision=FINISH。
4. FINISH 后，确保向最终用户输出完整的《告警分析报告》，格式必须严格为：
   告警分析报告
---
# 告警处理详情
## 活跃告警清单
## 告警根因分析N
## 处理方案执行N
## 结论。
5. 若步骤涉及腾讯云日志/主题工具，请确保使用连字符区域 ID（ap-guangzhou 等），或省略 region 以采用默认值。
6. 如果发现 Planner/Executor 在同一方向连续 3 次调用工具仍失败或没有数据，必须终止流程，直接输出"任务无法完成"的报告，明确告知失败原因，严禁凭空编造结果。

只允许在 planner_agent、executor_agent 与 FINISH 之间做出选择。
"##;

/// Supervisor 路由输出契约：约束其只产出机器可解析的 JSON 决策（供 `run_supervisor` 解析）。
const SUPERVISOR_ROUTING_SUFFIX: &str = r##"## 输出要求（CRITICAL）

你现在只负责"路由决策"，不要自己调用任何诊断工具、也不要输出报告内容。
请根据「当前任务、Planner 最新输出、Executor 最新反馈」判断下一步该做什么：
- 需要拆解任务或重新规划 -> "PLANNER"
- Planner 已给出可执行步骤（decision=EXECUTE）等待执行 -> "EXECUTOR"
- Planner 已输出最终《告警分析报告》，或证据已足够收尾 -> "FINISH"

只输出如下 JSON，不要任何额外文字、解释或代码围栏：
{"next": "PLANNER"}

其中 next 的取值只能是 PLANNER、EXECUTOR 或 FINISH 三者之一。
"##;
