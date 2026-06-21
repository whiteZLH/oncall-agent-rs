use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssistantMessage {
    text: String,
}

impl AssistantMessage {
    pub fn new(text: impl Into<String>) -> Self {
        Self { text: text.into() }
    }

    pub fn text(&self) -> &str {
        &self.text
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StateValue {
    AssistantMessage(AssistantMessage),
    Text(String),
}

impl StateValue {
    pub fn as_assistant_message(&self) -> Option<&AssistantMessage> {
        match self {
            StateValue::AssistantMessage(message) => Some(message),
            StateValue::Text(_) => None,
        }
    }

    pub fn as_text(&self) -> &str {
        match self {
            StateValue::AssistantMessage(message) => message.text(),
            StateValue::Text(text) => text,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OverAllState {
    values: HashMap<String, StateValue>,
}

impl OverAllState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn value(&self, key: &str) -> Option<&StateValue> {
        self.values.get(key)
    }

    pub fn assistant_message(&self, key: &str) -> Option<&AssistantMessage> {
        self.value(key).and_then(StateValue::as_assistant_message)
    }

    pub fn text(&self, key: &str) -> Option<&str> {
        self.value(key).map(StateValue::as_text)
    }

    pub fn text_or_empty(&self, key: &str) -> &str {
        self.text(key).unwrap_or("")
    }

    pub fn insert(&mut self, key: impl Into<String>, value: StateValue) {
        self.values.insert(key.into(), value);
    }

    pub fn insert_text(&mut self, key: impl Into<String>, text: impl Into<String>) {
        self.insert(key, StateValue::Text(text.into()));
    }

    pub fn insert_assistant_message(&mut self, key: impl Into<String>, text: impl Into<String>) {
        self.insert(
            key,
            StateValue::AssistantMessage(AssistantMessage::new(text)),
        );
    }

    pub fn record_agent_output(&mut self, agent_name: &str, output: impl Into<String>) {
        self.insert_assistant_message(agent_output_key(agent_name), output);
    }

    pub fn agent_output(&self, agent_name: &str) -> Option<&AssistantMessage> {
        self.assistant_message(&agent_output_key(agent_name))
    }

    pub fn render_template(&self, template: &str) -> String {
        template
            .replace("{input}", self.text_or_empty("input"))
            .replace(
                "{planner_plan}",
                self.text("planner_plan").unwrap_or("（暂无）"),
            )
            .replace(
                "{executor_feedback}",
                self.text("executor_feedback").unwrap_or("（暂无）"),
            )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReactAgent {
    name: String,
    description: String,
    model: String,
    system_prompt: String,
    output_key: Option<String>,
    method_tools: Vec<String>,
    tool_callbacks: Vec<String>,
    allowed_tools: Option<Vec<String>>,
    max_turns: usize,
}

impl ReactAgent {
    pub fn new(
        name: impl Into<String>,
        model: impl Into<String>,
        system_prompt: impl Into<String>,
        max_turns: usize,
    ) -> Self {
        Self {
            name: name.into(),
            description: String::new(),
            model: model.into(),
            system_prompt: system_prompt.into(),
            output_key: None,
            method_tools: Vec::new(),
            tool_callbacks: Vec::new(),
            allowed_tools: None,
            max_turns,
        }
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    pub fn with_output_key(mut self, output_key: impl Into<String>) -> Self {
        self.output_key = Some(output_key.into());
        self
    }

    pub fn with_tool_metadata(
        mut self,
        method_tools: Vec<String>,
        tool_callbacks: Vec<String>,
    ) -> Self {
        self.method_tools = method_tools;
        self.tool_callbacks = tool_callbacks;
        self
    }

    pub fn with_allowed_tools(mut self, allowed_tools: Vec<String>) -> Self {
        self.allowed_tools = Some(allowed_tools);
        self
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn description(&self) -> &str {
        &self.description
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn system_prompt(&self) -> &str {
        &self.system_prompt
    }

    pub fn preamble(&self) -> &str {
        self.system_prompt()
    }

    pub fn render_system_prompt(&self, state: &OverAllState) -> String {
        state.render_template(self.system_prompt())
    }

    pub fn output_key(&self) -> Option<&str> {
        self.output_key.as_deref()
    }

    pub fn max_turns(&self) -> usize {
        self.max_turns
    }

    pub fn method_tools(&self) -> &[String] {
        &self.method_tools
    }

    pub fn tool_callbacks(&self) -> &[String] {
        &self.tool_callbacks
    }

    pub fn allowed_tools(&self) -> Option<&[String]> {
        self.allowed_tools.as_deref()
    }

    pub fn allows_tool(&self, tool_name: &str) -> bool {
        self.allowed_tools
            .as_ref()
            .map(|tools| tools.iter().any(|name| name == tool_name))
            .unwrap_or(true)
    }
}

#[derive(Clone, Debug)]
pub struct SupervisorAgent {
    name: String,
    description: String,
    model: String,
    system_prompt: String,
    sub_agents: Vec<ReactAgent>,
    max_turns: usize,
}

impl SupervisorAgent {
    pub fn builder() -> SupervisorAgentBuilder {
        SupervisorAgentBuilder::default()
    }

    pub fn new(model: impl Into<String>, max_turns: usize) -> Self {
        Self::builder()
            .name("ai_ops_supervisor")
            .description("负责调度专业子 Agent 的多 Agent 控制器")
            .model(model)
            .system_prompt(Self::default_system_prompt())
            .max_turns(max_turns)
            .build()
    }

    pub fn default_system_prompt() -> String {
        SUPERVISOR_PROMPT.to_string()
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn description(&self) -> &str {
        &self.description
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn system_prompt(&self) -> &str {
        &self.system_prompt
    }

    pub fn preamble(&self) -> &str {
        self.system_prompt()
    }

    pub fn sub_agents(&self) -> &[ReactAgent] {
        &self.sub_agents
    }

    pub fn sub_agent(&self, name: &str) -> Option<&ReactAgent> {
        self.sub_agents.iter().find(|agent| agent.name() == name)
    }

    pub fn max_turns(&self) -> usize {
        self.max_turns
    }

    pub fn build_handoff_preamble(&self) -> String {
        let agents = if self.sub_agents.is_empty() {
            "（暂无可委派子 Agent）".to_string()
        } else {
            self.sub_agents
                .iter()
                .map(|agent| {
                    format!(
                        "- {}: {}",
                        agent.name(),
                        if agent.description().trim().is_empty() {
                            "无描述"
                        } else {
                            agent.description()
                        }
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        };
        format!(
            "{}\n\n## 可委派子 Agent\n{}\n\n## Handoff 工具规则\n- 你可以通过 transfer_to_<agent_name> 工具把明确子任务交给对应子 Agent。\n- 每次委派必须给出清晰 task；context 可包含当前已知事实、已获得证据和你希望子 Agent 聚焦的问题。\n- 子 Agent 返回的是证据或分析材料，你需要自行综合判断。\n- 最终答案必须由你输出，不要要求子 Agent 输出最终用户报告。",
            self.system_prompt.trim(),
            agents
        )
    }
}

#[derive(Clone, Debug)]
pub struct SupervisorAgentBuilder {
    name: String,
    description: String,
    model: String,
    system_prompt: String,
    sub_agents: Vec<ReactAgent>,
    max_turns: usize,
}

impl Default for SupervisorAgentBuilder {
    fn default() -> Self {
        Self {
            name: String::new(),
            description: String::new(),
            model: String::new(),
            system_prompt: SupervisorAgent::default_system_prompt(),
            sub_agents: Vec::new(),
            max_turns: 1,
        }
    }
}

impl SupervisorAgentBuilder {
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    pub fn model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    pub fn system_prompt(mut self, system_prompt: impl Into<String>) -> Self {
        self.system_prompt = system_prompt.into();
        self
    }

    pub fn sub_agents(mut self, sub_agents: Vec<ReactAgent>) -> Self {
        self.sub_agents = sub_agents;
        self
    }

    pub fn max_turns(mut self, max_turns: usize) -> Self {
        self.max_turns = max_turns;
        self
    }

    pub fn build(self) -> SupervisorAgent {
        SupervisorAgent {
            name: self.name,
            description: self.description,
            model: self.model,
            system_prompt: self.system_prompt,
            sub_agents: self.sub_agents,
            max_turns: self.max_turns,
        }
    }
}

pub fn agent_output_key(agent_name: &str) -> String {
    format!("agent_output:{agent_name}")
}

/// Supervisor 系统提示词。具体可用子 Agent 会在运行时根据 `sub_agents` 自动追加。
const SUPERVISOR_PROMPT: &str = r##"你是 AI Ops Supervisor，负责把告警排查任务拆给专业子 Agent，并基于子 Agent 返回的证据和分析自行综合最终结论。
你必须遵守：
1. 只通过 handoff 工具委派子任务，不直接调用 Prometheus、日志、文档等诊断工具。
2. 子 Agent 的返回内容只是材料；最终《告警分析报告》必须由你输出。
3. 严禁编造数据；证据不足时必须明确说明缺失证据。
4. 工具或子 Agent 返回中若包含 _diagnosisEvidenceId 或 [evidence: ev-xxxx]，报告引用对应结论时必须标注该 evidence id。
5. 若连续委派仍无法获得有效证据，必须在报告中说明无法完成的原因。
"##;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn react_agent_keeps_core_configuration() {
        let agent = ReactAgent::new("planner_agent", "qwen-plus", "plan prompt", 12)
            .with_description("负责拆解告警、规划与再规划步骤")
            .with_output_key("planner_plan")
            .with_allowed_tools(vec!["queryMetricTrend".to_string()]);

        assert_eq!(agent.name(), "planner_agent");
        assert_eq!(agent.description(), "负责拆解告警、规划与再规划步骤");
        assert_eq!(agent.model(), "qwen-plus");
        assert_eq!(agent.system_prompt(), "plan prompt");
        assert_eq!(agent.preamble(), "plan prompt");
        assert_eq!(agent.output_key(), Some("planner_plan"));
        assert_eq!(agent.max_turns(), 12);
        assert!(agent.method_tools().is_empty());
        assert!(agent.tool_callbacks().is_empty());
        assert_eq!(
            agent.allowed_tools(),
            Some(&["queryMetricTrend".to_string()][..])
        );
        assert!(agent.allows_tool("queryMetricTrend"));
        assert!(!agent.allows_tool("queryLogs"));
    }

    #[test]
    fn react_agent_keeps_tool_metadata() {
        let agent = ReactAgent::new("executor_agent", "qwen-plus", "exec prompt", 12)
            .with_tool_metadata(
                vec!["queryMetricTrend".to_string()],
                vec!["mcp".to_string()],
            );

        assert_eq!(agent.method_tools(), &["queryMetricTrend".to_string()]);
        assert_eq!(agent.tool_callbacks(), &["mcp".to_string()]);
    }

    #[test]
    fn overall_state_stores_assistant_messages() {
        let mut state = OverAllState::new();
        state.insert_assistant_message("planner_plan", "report");
        state.record_agent_output("metrics_agent", "metrics");

        let message = state
            .assistant_message("planner_plan")
            .expect("planner_plan should be an assistant message");
        assert_eq!(message.text(), "report");
        assert_eq!(
            state
                .agent_output("metrics_agent")
                .expect("metrics agent output")
                .text(),
            "metrics"
        );
    }

    #[test]
    fn overall_state_renders_prompt_templates() {
        let mut state = OverAllState::new();
        state.insert_text("input", "task");
        state.insert_assistant_message("planner_plan", "plan");
        state.insert_assistant_message("executor_feedback", "feedback");

        assert_eq!(
            state.render_template("{input}|{planner_plan}|{executor_feedback}"),
            "task|plan|feedback"
        );
    }

    #[test]
    fn supervisor_agent_keeps_sub_agents() {
        let metrics = ReactAgent::new("metrics_agent", "qwen-plus", "metrics", 12)
            .with_description("负责指标诊断");
        let knowledge = ReactAgent::new("knowledge_agent", "qwen-plus", "knowledge", 12)
            .with_description("负责知识库检索");
        let supervisor = SupervisorAgent::builder()
            .name("ai_ops_supervisor")
            .description("负责调度专业子 Agent 的多 Agent 控制器")
            .model("qwen-plus")
            .system_prompt("supervisor")
            .sub_agents(vec![metrics, knowledge])
            .max_turns(12)
            .build();

        assert_eq!(supervisor.name(), "ai_ops_supervisor");
        assert_eq!(
            supervisor.description(),
            "负责调度专业子 Agent 的多 Agent 控制器"
        );
        assert_eq!(supervisor.system_prompt(), "supervisor");
        assert_eq!(supervisor.sub_agents().len(), 2);
        assert_eq!(
            supervisor
                .sub_agent("metrics_agent")
                .expect("metrics agent")
                .description(),
            "负责指标诊断"
        );
    }

    #[test]
    fn supervisor_agent_builds_handoff_preamble_from_sub_agents() {
        let metrics = ReactAgent::new("metrics_agent", "qwen-plus", "metrics", 12)
            .with_description("负责 Prometheus 指标诊断");
        let logs =
            ReactAgent::new("logs_agent", "qwen-plus", "logs", 12).with_description("负责日志检索");
        let supervisor = SupervisorAgent::builder()
            .name("ai_ops_supervisor")
            .description("负责调度专业子 Agent 的多 Agent 控制器")
            .model("qwen-plus")
            .system_prompt("supervisor")
            .sub_agents(vec![metrics, logs])
            .max_turns(12)
            .build();

        let preamble = supervisor.build_handoff_preamble();

        assert!(preamble.contains("- metrics_agent: 负责 Prometheus 指标诊断"));
        assert!(preamble.contains("- logs_agent: 负责日志检索"));
        assert!(preamble.contains("transfer_to_<agent_name>"));
        assert!(preamble.contains("最终答案必须由你输出"));
    }
}
