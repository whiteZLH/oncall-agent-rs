//! AI Ops 工具的诊断证据装饰器。
//! 对应 Java 版本 `org.example.service.DiagnosisEvidenceRecorder` 的
//! `RecordingToolCallback` + `recordToolCall`：在每次工具调用前后记录证据，
//! 并把 `_diagnosisEvidenceId` / `_diagnosisEvidenceSuccess` 注入工具返回，
//! 供 Planner/Executor 在报告中以 `[evidence: ev-xxxx]` 引用。
//!
//! 包装发生在 [`rig::tool::ToolDyn`] 层（与 Java 拦截 `ToolCallback.call(String)`
//! 同一层），因此能拿到模型传入的原始 JSON 入参作为 `query_params`，
//! 且无需各工具的 `Args` 实现 `Serialize`。

use crate::domain::diagnosis::DiagnosisEvidence;
use crate::services::incident_service::EvidenceCollector;
use rig::completion::ToolDefinition;
use rig::tool::{ToolDyn, ToolError};
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

/// 摘要最大长度（对齐 Java `SUMMARY_LIMIT`）。
const SUMMARY_LIMIT: usize = 500;
/// 原始返回片段最大长度（对齐 Java `RAW_FRAGMENT_LIMIT`）。
const RAW_FRAGMENT_LIMIT: usize = 6000;

/// 包装任意 [`ToolDyn`] 工具，在调用前后记录诊断证据并注入证据 id。
/// 对齐 Java `RecordingToolCallback`。
pub struct RecordingTool {
    inner: Box<dyn ToolDyn>,
    collector: Arc<EvidenceCollector>,
}

impl RecordingTool {
    pub fn new(inner: Box<dyn ToolDyn>, collector: Arc<EvidenceCollector>) -> Self {
        Self { inner, collector }
    }
}

impl ToolDyn for RecordingTool {
    fn name(&self) -> String {
        self.inner.name()
    }

    fn definition(
        &self,
        prompt: String,
    ) -> Pin<Box<dyn Future<Output = ToolDefinition> + Send + '_>> {
        self.inner.definition(prompt)
    }

    fn call(
        &self,
        args: String,
    ) -> Pin<Box<dyn Future<Output = Result<String, ToolError>> + Send + '_>> {
        Box::pin(async move {
            let tool_name = self.inner.name();
            // 工具调用前：标记 run 进入等待工具状态（对齐 Java markWaiting）。
            self.collector.mark_waiting(&tool_name, &args);

            match self.inner.call(args.clone()).await {
                Ok(raw) => {
                    let success = detect_success(&raw);
                    let evidence = build_tool_evidence(&tool_name, &args, &raw, success, None);
                    let evidence_id = evidence.id.clone();
                    self.collector.record(evidence);
                    Ok(decorate_result(&raw, &evidence_id, success))
                }
                Err(error) => {
                    // 工具失败：记录失败证据（success=false），再原样向上传递错误（对齐 Java rethrow）。
                    let message = error.to_string();
                    let evidence =
                        build_tool_evidence(&tool_name, &args, "", false, Some(&message));
                    self.collector.record(evidence);
                    Err(error)
                }
            }
        })
    }
}

/// 构造一条 `tool_call` 证据（对齐 Java `DiagnosisEvidence.toolCall` + `recordToolCall`）。
fn build_tool_evidence(
    tool_name: &str,
    query_params: &str,
    raw_result: &str,
    success: bool,
    error_message: Option<&str>,
) -> DiagnosisEvidence {
    DiagnosisEvidence {
        id: format!("ev-{}", &Uuid::new_v4().simple().to_string()[..12]),
        evidence_type: "tool_call".to_string(),
        title: format!("工具调用: {tool_name}"),
        tool_name: tool_name.to_string(),
        query_params: query_params.to_string(),
        time_range: "工具调用".to_string(),
        summary: summarize(raw_result, error_message),
        raw_fragment: truncate(raw_result, RAW_FRAGMENT_LIMIT),
        success,
        error_message: error_message.unwrap_or_default().to_string(),
        error_code: extract_error_code(raw_result),
        created_at: now_millis(),
        ..DiagnosisEvidence::default()
    }
}

/// 判定工具返回是否成功（对齐 Java `detectSuccess`）。
fn detect_success(raw_result: &str) -> bool {
    if raw_result.trim().is_empty() {
        return true;
    }
    if let Ok(value) = serde_json::from_str::<Value>(raw_result) {
        if let Some(success) = value.get("success").and_then(Value::as_bool) {
            return success;
        }
        if let Some(status) = value.get("status").and_then(Value::as_str) {
            let status = status.to_lowercase();
            return !(status == "error" || status == "failed" || status == "failure");
        }
        return true;
    }
    // 非 JSON：仅当文本明确包含 "success":false 时判失败。
    !raw_result.to_lowercase().replace(' ', "").contains("\"success\":false")
}

/// 生成证据摘要（对齐 Java `summarize`，上限 [`SUMMARY_LIMIT`]）。
fn summarize(raw_result: &str, error_message: Option<&str>) -> String {
    if let Some(message) = error_message {
        if !message.trim().is_empty() {
            return truncate(&format!("工具调用失败: {message}"), SUMMARY_LIMIT);
        }
    }
    if raw_result.trim().is_empty() {
        return "工具调用完成，返回为空".to_string();
    }
    if let Ok(value) = serde_json::from_str::<Value>(raw_result) {
        if let Some(message) = value.get("message").and_then(Value::as_str) {
            if !message.trim().is_empty() {
                return truncate(message, SUMMARY_LIMIT);
            }
        }
        if let Some(status) = value.get("status").and_then(Value::as_str) {
            if !status.trim().is_empty() {
                return truncate(&format!("status={status}"), SUMMARY_LIMIT);
            }
        }
    }
    truncate(raw_result, SUMMARY_LIMIT)
}

/// 提取错误码（对齐 Java `extractErrorCode` 的 JSON 分支；依赖熔断的 P2 暂不接）。
fn extract_error_code(raw_result: &str) -> String {
    if raw_result.trim().is_empty() {
        return String::new();
    }
    if let Ok(value) = serde_json::from_str::<Value>(raw_result) {
        if let Some(code) = value.get("errorCode").and_then(Value::as_str) {
            if !code.trim().is_empty() {
                return code.to_string();
            }
        }
    }
    String::new()
}

/// 向工具返回注入证据 id（对齐 Java `decorateResult`）。
/// JSON 对象 → 末尾追加 `_diagnosisEvidenceId`/`_diagnosisEvidenceSuccess`（保持原字段顺序）；
/// 否则纯文本追加证据脚注。
fn decorate_result(raw_result: &str, evidence_id: &str, success: bool) -> String {
    let trimmed = raw_result.trim();
    if is_json_object(trimmed) {
        // 去掉末尾 `}`，在原有字段后追加，避免 serde 重排序整个对象。
        let body = trimmed[..trimmed.len() - 1].trim_end();
        let separator = if body.ends_with('{') { "" } else { "," };
        return format!(
            "{body}{separator}\"_diagnosisEvidenceId\":\"{evidence_id}\",\"_diagnosisEvidenceSuccess\":{success}}}"
        );
    }
    format!("{raw_result}\n\n诊断证据ID: {evidence_id}")
}

fn is_json_object(value: &str) -> bool {
    value.starts_with('{')
        && value.ends_with('}')
        && serde_json::from_str::<Value>(value)
            .map(|parsed| parsed.is_object())
            .unwrap_or(false)
}

/// 按字符截断并追加省略号（对齐 Java `truncate`）。
fn truncate(value: &str, limit: usize) -> String {
    let chars: Vec<char> = value.chars().collect();
    if chars.len() <= limit {
        return value.to_string();
    }
    let mut result: String = chars[..limit].iter().collect();
    result.push_str("...");
    result
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("系统时间早于 Unix 纪元")
        .as_millis() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_success_reads_success_flag() {
        assert!(detect_success(r#"{"success":true,"message":"ok"}"#));
        assert!(!detect_success(r#"{"success":false,"errorCode":"DEPENDENCY_ERROR"}"#));
    }

    #[test]
    fn detect_success_reads_status_field() {
        assert!(!detect_success(r#"{"status":"error","message":"x"}"#));
        assert!(detect_success(r#"{"status":"success"}"#));
        assert!(detect_success(r#"{"status":"no_results"}"#));
    }

    #[test]
    fn detect_success_defaults_true_and_handles_plain_text() {
        assert!(detect_success(""));
        assert!(detect_success("unix_timestamp_seconds=1"));
        assert!(!detect_success("{ \"success\": false }"));
    }

    #[test]
    fn summarize_prefers_error_then_message() {
        assert_eq!(summarize("{}", Some("超时")), "工具调用失败: 超时");
        assert_eq!(summarize(r#"{"message":"近1h无异常"}"#, None), "近1h无异常");
        assert_eq!(summarize(r#"{"status":"no_results"}"#, None), "status=no_results");
        assert_eq!(summarize("", None), "工具调用完成，返回为空");
    }

    #[test]
    fn truncate_caps_length() {
        assert_eq!(truncate("abc", 5), "abc");
        assert_eq!(truncate("abcdef", 3), "abc...");
    }

    #[test]
    fn decorate_injects_into_json_object_keeping_fields() {
        let out = decorate_result(r#"{"success":true,"alerts":[]}"#, "ev-abc123", true);
        assert!(out.starts_with(r#"{"success":true,"alerts":[]"#));
        assert!(out.contains(r#""_diagnosisEvidenceId":"ev-abc123""#));
        assert!(out.contains(r#""_diagnosisEvidenceSuccess":true"#));
        assert!(serde_json::from_str::<Value>(&out).unwrap().is_object());
    }

    #[test]
    fn decorate_handles_empty_object_and_plain_text() {
        let obj = decorate_result("{}", "ev-1", false);
        assert!(serde_json::from_str::<Value>(&obj).unwrap().is_object());
        assert!(obj.contains(r#""_diagnosisEvidenceId":"ev-1""#));

        let text = decorate_result("unix_timestamp_seconds=1", "ev-2", true);
        assert_eq!(text, "unix_timestamp_seconds=1\n\n诊断证据ID: ev-2");
    }

    #[test]
    fn build_evidence_truncates_and_sets_fields() {
        let evidence = build_tool_evidence(
            "queryMetricTrend",
            r#"{"metric":"cpu_usage"}"#,
            r#"{"success":true,"message":"ok"}"#,
            true,
            None,
        );
        assert_eq!(evidence.evidence_type, "tool_call");
        assert_eq!(evidence.tool_name, "queryMetricTrend");
        assert_eq!(evidence.query_params, r#"{"metric":"cpu_usage"}"#);
        assert!(evidence.id.starts_with("ev-"));
        assert_eq!(evidence.id.len(), 3 + 12);
        assert_eq!(evidence.summary, "ok");
        assert!(evidence.success);
    }
}
