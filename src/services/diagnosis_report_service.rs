//! 诊断报告证据守卫与上下文增强。
//! 对应 Java 版本 `org.example.service.DiagnosisReportService`。
//!
//! - [`augment_alert_context`]：调用 AI Ops 前，将已持久化的工具 evidence 拼成证据表 +
//!   报告规则，注入到告警上下文（对齐 `augmentAlertContext`）。
//! - [`constrain_report`]：报告生成后，解析正文中的 `[evidence: ev-xxx]` 引用并做证据校验，
//!   末尾追加「## 证据校验」小节（对齐 `guardReport`/`constrainReport`）。
//!
//! 与 Java 的唯一差异：evidence 引用提取以小写 `[evidence:` 前缀匹配（提示词约定的格式），
//! 未复刻 Java 正则的大小写不敏感分支。

use crate::domain::diagnosis::DiagnosisEvidence;

/// 证据表 / 校验小节中摘要列的最大长度（对齐 Java `SUMMARY_LIMIT`）。
const SUMMARY_LIMIT: usize = 180;

/// 将工具 evidence 表与报告规则拼接到告警上下文末尾。
pub fn augment_alert_context(alert_context: &str, evidence: &[DiagnosisEvidence]) -> String {
    let mut builder = alert_context.trim().to_string();
    let table = build_evidence_table(evidence);
    if !table.is_empty() {
        builder.push_str("\n\n");
        builder.push_str(&table);
    }
    builder.push_str("\n\n");
    builder.push_str(report_rules());
    builder
}

/// 生成「可用工具证据表（仅成功）」与「失败工具证据说明」两张 Markdown 表。
pub fn build_evidence_table(evidence: &[DiagnosisEvidence]) -> String {
    let usable = usable_tool_evidence(evidence);
    let failed = failed_tool_evidence(evidence);
    if usable.is_empty() && failed.is_empty() {
        return String::new();
    }

    let mut builder = String::new();
    if !usable.is_empty() {
        builder.push_str("## 可用工具证据表（仅成功）\n");
        builder.push_str("| Evidence ID | 类型 | 工具 | 时间范围 | 状态 | 摘要 |\n");
        builder.push_str("|---|---|---|---|---|---|\n");
        for item in &usable {
            builder.push_str(&format!(
                "| {} | {} | {} | {} | success | {} |\n",
                cell(&item.id),
                cell(&item.evidence_type),
                cell(&item.tool_name),
                cell(&item.time_range),
                cell(&compact(&item.summary, SUMMARY_LIMIT)),
            ));
        }
    }
    if !failed.is_empty() {
        if !builder.is_empty() {
            builder.push('\n');
        }
        builder.push_str("## 失败工具证据说明\n");
        builder.push_str("| Evidence ID | 工具 | 状态 | 错误码 | 摘要 |\n");
        builder.push_str("|---|---|---|---|---|\n");
        for item in &failed {
            builder.push_str(&format!(
                "| {} | {} | failed | {} | {} |\n",
                cell(&item.id),
                cell(&item.tool_name),
                cell(&item.error_code),
                cell(&compact(&item.summary, SUMMARY_LIMIT)),
            ));
        }
    }
    builder.trim().to_string()
}

/// 解析报告中引用的 evidence id，按成功/失败/未知/缺失分类校验，
/// 末尾追加「## 证据校验」小节。重复调用前会先剥离已有的同名小节（幂等）。
pub fn constrain_report(report: &str, evidence: &[DiagnosisEvidence]) -> String {
    let safe_report = strip_existing_validation_section(report);
    let safe_report = safe_report.trim();

    let tool_evidence = tool_evidence(evidence);
    let usable_evidence = usable_tool_evidence(evidence);
    let all_ids: Vec<&str> = tool_evidence
        .iter()
        .map(|item| item.id.as_str())
        .filter(|id| !id.is_empty())
        .collect();
    let usable_ids: Vec<&str> = usable_evidence
        .iter()
        .map(|item| item.id.as_str())
        .filter(|id| !id.is_empty())
        .collect();

    let cited_ids = extract_evidence_ids(safe_report);
    let unknown_ids: Vec<String> = cited_ids
        .iter()
        .filter(|id| !all_ids.contains(&id.as_str()))
        .cloned()
        .collect();
    let invalid_ids: Vec<String> = cited_ids
        .iter()
        .filter(|id| all_ids.contains(&id.as_str()) && !usable_ids.contains(&id.as_str()))
        .cloned()
        .collect();
    let known_cited_ids: Vec<String> = cited_ids
        .iter()
        .filter(|id| !unknown_ids.contains(id) && !invalid_ids.contains(id))
        .cloned()
        .collect();

    let missing_evidence = find_missing_evidence(
        safe_report,
        &usable_evidence,
        &known_cited_ids,
        &tool_evidence,
    );
    let confidence = confidence(
        &unknown_ids,
        &invalid_ids,
        &missing_evidence,
        &known_cited_ids,
    );
    let status = validation_status(&unknown_ids, &invalid_ids, &missing_evidence);

    format!(
        "{safe_report}\n\n---\n\n\
         ## 证据校验\n\
         - 校验状态: {status}\n\
         - 置信度: {confidence}\n\
         - 已引用证据: {cited}\n\
         - 未知证据: {unknown}\n\
         - 无效/失败证据: {invalid}\n\
         - 缺失证据: {missing}\n\
         - 可用证据: {available}\n\
         - 约束说明: 以上校验仅基于已持久化的工具 evidence；未被 evidence 支撑的结论应按证据不足处理。\n",
        cited = if known_cited_ids.is_empty() {
            "无".to_string()
        } else {
            known_cited_ids.join(", ")
        },
        unknown = if unknown_ids.is_empty() {
            "无".to_string()
        } else {
            unknown_ids.join(", ")
        },
        invalid = invalid_evidence_summary(&invalid_ids, &tool_evidence),
        missing = if missing_evidence.is_empty() {
            "无".to_string()
        } else {
            missing_evidence.join("; ")
        },
        available = available_evidence_summary(&usable_evidence),
    )
}

fn find_missing_evidence(
    report: &str,
    usable_evidence: &[&DiagnosisEvidence],
    known_cited_ids: &[String],
    tool_evidence: &[&DiagnosisEvidence],
) -> Vec<String> {
    let mut missing = Vec::new();
    if !tool_evidence.is_empty() && known_cited_ids.is_empty() {
        missing.push("报告没有引用任何工具 evidence id".to_string());
    }
    if contains_resource_claim(report)
        && !has_cited_successful_tool(known_cited_ids, usable_evidence, "queryMetricTrend")
    {
        missing.push("资源类结论缺少成功的 queryMetricTrend 趋势 evidence".to_string());
    }
    let has_successful_logs =
        has_cited_successful_tool(known_cited_ids, usable_evidence, "queryLogs");
    let contains_jvm_claim = contains_jvm_evidence_claim(report);
    let has_successful_jvm_text = has_cited_evidence_text(
        known_cited_ids,
        usable_evidence,
        &[
            "GC",
            "Full GC",
            "gc_",
            "OutOfMemory",
            "OutOfMemoryError",
            "OOM",
        ],
    );
    if contains_log_claim(report)
        && !has_successful_logs
        && !(contains_jvm_claim && has_successful_jvm_text)
    {
        missing.push("日志/异常结论缺少成功的 queryLogs evidence".to_string());
    }
    if contains_jvm_claim && !has_successful_logs && !has_successful_jvm_text {
        missing.push("GC/Full GC 结论缺少对应日志或 JVM 指标 evidence".to_string());
    }
    missing
}

/// 删除报告中已存在的「## 证据校验」小节（到下一个二级标题或结尾），保证 guard 幂等。
fn strip_existing_validation_section(report: &str) -> String {
    let mut lines: Vec<&str> = Vec::new();
    let mut skipping = false;
    for line in report.lines() {
        let trimmed = line.trim();
        if trimmed == "## 证据校验" {
            skipping = true;
            continue;
        }
        if skipping {
            // 遇到下一个非「## 证据校验」的二级标题时停止跳过，保留该行。
            if trimmed.starts_with("## ") && trimmed != "## 证据校验" {
                skipping = false;
            } else {
                continue;
            }
        }
        lines.push(line);
    }
    // Java 在剥离后会清除尾部的分隔线 `---`（紧邻校验小节之前那条）。
    let mut joined = lines.join("\n");
    joined = joined.trim_end().to_string();
    while joined.trim_end().ends_with("---") {
        let cut = joined.trim_end().len() - 3;
        joined = joined[..cut].trim_end().to_string();
    }
    joined.trim().to_string()
}

/// 提取报告中所有 `[evidence: ev-xxx]` 引用的 id（保序去重）。
fn extract_evidence_ids(report: &str) -> Vec<String> {
    let mut ids: Vec<String> = Vec::new();
    let mut rest = report;
    const MARKER: &str = "[evidence:";
    while let Some(pos) = rest.find(MARKER) {
        rest = &rest[pos + MARKER.len()..];
        let Some(end) = rest.find(']') else {
            break;
        };
        // 对齐 Java 正则 `[^\]\s]+`：取闭合括号前第一个非空白片段。
        let id = rest[..end]
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_string();
        if !id.is_empty() && !ids.contains(&id) {
            ids.push(id);
        }
        rest = &rest[end + 1..];
    }
    ids
}

fn has_cited_successful_tool(
    cited_ids: &[String],
    usable_evidence: &[&DiagnosisEvidence],
    tool_name: &str,
) -> bool {
    cited_ids.iter().any(|id| {
        usable_evidence
            .iter()
            .any(|item| &item.id == id && item.success && item.tool_name == tool_name)
    })
}

fn has_cited_evidence_text(
    cited_ids: &[String],
    usable_evidence: &[&DiagnosisEvidence],
    needles: &[&str],
) -> bool {
    cited_ids.iter().any(|id| {
        usable_evidence.iter().any(|item| {
            &item.id == id && {
                let text = format!("{} {} {}", item.summary, item.content, item.raw_fragment);
                contains_any(&text, needles)
            }
        })
    })
}

fn contains_resource_claim(report: &str) -> bool {
    contains_any(
        report,
        &[
            "CPU",
            "cpu",
            "内存",
            "memory",
            "错误率",
            "error_rate",
            "P99",
            "延迟",
            "latency",
            "restart",
            "重启",
        ],
    )
}

fn contains_log_claim(report: &str) -> bool {
    contains_any(
        report,
        &[
            "日志",
            "log",
            "ERROR",
            "OOM",
            "OutOfMemory",
            "异常日志",
            "错误日志",
        ],
    )
}

fn contains_jvm_evidence_claim(report: &str) -> bool {
    contains_any(
        report,
        &[
            "OOM",
            "OutOfMemory",
            "Full GC",
            "GC 风暴",
            "GC overhead",
            "Garbage Collection",
            "gc_",
        ],
    )
}

fn contains_any(text: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| text.contains(needle))
}

fn confidence(
    unknown_ids: &[String],
    invalid_ids: &[String],
    missing_evidence: &[String],
    known_cited_ids: &[String],
) -> &'static str {
    if !unknown_ids.is_empty() || !invalid_ids.is_empty() {
        return "低";
    }
    if missing_evidence.is_empty() && !known_cited_ids.is_empty() {
        return "高";
    }
    "中"
}

fn validation_status(
    unknown_ids: &[String],
    invalid_ids: &[String],
    missing_evidence: &[String],
) -> &'static str {
    if !unknown_ids.is_empty() || !invalid_ids.is_empty() {
        return "失败";
    }
    if !missing_evidence.is_empty() {
        return "需补证";
    }
    "通过"
}

fn invalid_evidence_summary(
    invalid_ids: &[String],
    tool_evidence: &[&DiagnosisEvidence],
) -> String {
    if invalid_ids.is_empty() {
        return "无".to_string();
    }
    invalid_ids
        .iter()
        .map(|id| {
            let evidence = tool_evidence.iter().find(|item| &item.id == id);
            let mut reason = evidence
                .map(|item| item.error_code.clone())
                .unwrap_or_default();
            if reason.is_empty() {
                reason = match evidence {
                    Some(item) if !item.success => "success=false".to_string(),
                    _ => "unusable".to_string(),
                };
            }
            format!("{id}({reason})")
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn available_evidence_summary(usable_evidence: &[&DiagnosisEvidence]) -> String {
    if usable_evidence.is_empty() {
        return "无".to_string();
    }
    usable_evidence
        .iter()
        .map(|item| {
            let state = if item.success { "success" } else { "failed" };
            let code = if item.error_code.is_empty() {
                String::new()
            } else {
                format!(",{}", item.error_code)
            };
            format!("{}({},{}{})", item.id, item.tool_name, state, code)
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// 工具类 evidence：`type == "tool_call"` 或 `tool_name` 非空（对齐 Java `toolEvidence`）。
fn tool_evidence(evidence: &[DiagnosisEvidence]) -> Vec<&DiagnosisEvidence> {
    evidence
        .iter()
        .filter(|item| item.evidence_type == "tool_call" || !item.tool_name.trim().is_empty())
        .collect()
}

fn usable_tool_evidence(evidence: &[DiagnosisEvidence]) -> Vec<&DiagnosisEvidence> {
    tool_evidence(evidence)
        .into_iter()
        .filter(|item| item.success && item.error_code != "CIRCUIT_OPEN")
        .collect()
}

fn failed_tool_evidence(evidence: &[DiagnosisEvidence]) -> Vec<&DiagnosisEvidence> {
    tool_evidence(evidence)
        .into_iter()
        .filter(|item| !item.success || item.error_code == "CIRCUIT_OPEN")
        .collect()
}

fn report_rules() -> &'static str {
    "## 报告证据约束\n\
     - 最终报告中的每个根因、症状和处理建议必须引用上方成功工具 Evidence ID，格式为 [evidence: ev-xxxx]。\n\
     - 资源类结论（CPU、内存、错误率、P99、重启）必须引用 queryMetricTrend 证据。\n\
     - 日志、异常、GC/Full GC、OOM 结论必须引用 queryLogs 或明确包含该信号的 JVM/日志证据。\n\
     - 禁止使用未出现在成功证据表中的 evidence id；失败或熔断 evidence 只能说明证据缺失，不能支撑事实结论。\n\
     - 最终报告需要包含“置信度”和“缺失证据”判断。"
}

/// 转义表格单元格中的 `|` 与换行（对齐 Java `cell`）。
fn cell(value: &str) -> String {
    let value = if value.trim().is_empty() { "" } else { value };
    value.replace('|', "\\|").replace('\n', " ")
}

/// 压缩空白并按字符数截断（对齐 Java `compact`）。
fn compact(value: &str, limit: usize) -> String {
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let chars: Vec<char> = normalized.chars().collect();
    if chars.len() <= limit {
        return normalized;
    }
    let cut = limit.saturating_sub(1);
    let mut result: String = chars[..cut].iter().collect();
    result.push('…');
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool_ev(id: &str, tool: &str, success: bool) -> DiagnosisEvidence {
        DiagnosisEvidence {
            id: id.to_string(),
            evidence_type: "tool_call".to_string(),
            tool_name: tool.to_string(),
            summary: format!("{tool} 摘要"),
            success,
            ..DiagnosisEvidence::default()
        }
    }

    #[test]
    fn augment_appends_table_and_rules() {
        let evidence = vec![tool_ev("ev-1", "queryMetricTrend", true)];
        let out = augment_alert_context("告警上下文", &evidence);
        assert!(out.starts_with("告警上下文"));
        assert!(out.contains("## 可用工具证据表（仅成功）"));
        assert!(out.contains("ev-1"));
        assert!(out.contains("## 报告证据约束"));
    }

    #[test]
    fn constrain_passes_when_resource_claim_cites_metric_trend() {
        let evidence = vec![tool_ev("ev-1", "queryMetricTrend", true)];
        let report = "## 根因\nCPU 飙升 [evidence: ev-1]";
        let out = constrain_report(report, &evidence);
        assert!(out.contains("## 证据校验"));
        assert!(out.contains("- 校验状态: 通过"));
        assert!(out.contains("- 置信度: 高"));
        assert!(out.contains("- 已引用证据: ev-1"));
    }

    #[test]
    fn constrain_flags_unknown_evidence_id() {
        let evidence = vec![tool_ev("ev-1", "queryMetricTrend", true)];
        let report = "结论 [evidence: ev-unknown]";
        let out = constrain_report(report, &evidence);
        assert!(out.contains("- 校验状态: 失败"));
        assert!(out.contains("- 置信度: 低"));
        assert!(out.contains("- 未知证据: ev-unknown"));
    }

    #[test]
    fn constrain_flags_missing_metric_trend_for_resource_claim() {
        let evidence = vec![tool_ev("ev-1", "queryLogs", true)];
        let report = "CPU 使用率过高 [evidence: ev-1]";
        let out = constrain_report(report, &evidence);
        assert!(out.contains("- 校验状态: 需补证"));
        assert!(out.contains("资源类结论缺少成功的 queryMetricTrend"));
    }

    #[test]
    fn constrain_is_idempotent() {
        let evidence = vec![tool_ev("ev-1", "queryMetricTrend", true)];
        let report = "CPU [evidence: ev-1]";
        let once = constrain_report(report, &evidence);
        let twice = constrain_report(&once, &evidence);
        assert_eq!(once, twice);
        assert_eq!(twice.matches("## 证据校验").count(), 1);
    }
}
