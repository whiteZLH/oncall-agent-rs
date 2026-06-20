//! AI Ops 多 Agent 编排所用的诊断工具集合。
//! 对应 Java 版本 `org.example.agent.tool` 包：
//! - [`metrics_tools`]：Prometheus 告警 / 指标趋势查询
//! - [`logs_tools`]：CLS 日志 / 日志主题查询
//!
//! 时间与内部文档工具复用 [`crate::services::chat_service`] 中的
//! `GetCurrentDateTimeTool` 与 `QueryInternalDocsTool`。

pub mod evidence;
pub mod logs_tools;
pub mod metrics_tools;
