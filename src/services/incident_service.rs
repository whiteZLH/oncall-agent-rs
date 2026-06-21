use crate::{
    config::AppConfig,
    domain::{
        diagnosis::{DiagnosisEvidence, DiagnosisRun},
        incident::{ArchiveResult, IncidentRecord, IncidentSummary, SearchResult},
    },
    error::AppError,
    services::{
        diagnosis_report_service::constrain_report,
        milvus_service::MilvusDocument,
        vector_embedding_service::{generate_sparse_vector, VectorEmbeddingService},
        vector_search_service::VectorSearchService,
    },
};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

pub struct IncidentService {
    incidents: Arc<Mutex<HashMap<String, IncidentRecord>>>,
    vector_search_service: VectorSearchService,
    embedding_service: VectorEmbeddingService,
}

impl IncidentService {
    pub fn new(config: &AppConfig) -> Self {
        let incident = seed_incident();
        Self {
            incidents: Arc::new(Mutex::new(HashMap::from([(incident.id.clone(), incident)]))),
            vector_search_service: VectorSearchService::new(config),
            embedding_service: VectorEmbeddingService::new(config),
        }
    }

    pub fn list(&self) -> Vec<IncidentSummary> {
        let mut summaries = self
            .incidents
            .lock()
            .expect("事故存储锁已损坏")
            .values()
            .map(summary)
            .collect::<Vec<_>>();
        summaries.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
        summaries
    }

    pub fn get(&self, incident_id: &str) -> Option<IncidentRecord> {
        self.incidents
            .lock()
            .expect("事故存储锁已损坏")
            .get(incident_id)
            .cloned()
    }

    pub fn runs(&self, incident_id: &str) -> Option<Vec<DiagnosisRun>> {
        self.get(incident_id)
            .map(|incident| incident.diagnosis_runs)
    }

    pub fn diagnose(&self, incident_id: &str) -> Option<DiagnosisRun> {
        let mut incidents = self.incidents.lock().expect("事故存储锁已损坏");
        let incident = incidents.get_mut(incident_id)?;
        let now = now_millis();
        let alert_context = build_alert_context(incident);
        let run = DiagnosisRun {
            run_id: format!("run-{}", &Uuid::new_v4().simple().to_string()[..12]),
            incident_id: incident_id.to_string(),
            status: "QUEUED".to_string(),
            created_at: now,
            alert_context: alert_context.clone(),
            current_step: "等待诊断任务开始".to_string(),
            progress_message: "诊断任务已入队".to_string(),
            evidence: vec![context_evidence(
                "alert_context",
                "注入给 AI 的告警上下文",
                &alert_context,
                now,
            )],
            ..DiagnosisRun::default()
        };
        incident.diagnosis_runs.push(run.clone());
        incident.updated_at = now;
        Some(run)
    }

    pub async fn archive_case(&self, incident_id: &str) -> Result<ArchiveResult, AppError> {
        let incident = self
            .get(incident_id)
            .ok_or_else(|| AppError::bad_request(format!("Incident 不存在: {incident_id}")))?;
        let run = latest_completed_run(&incident)
            .ok_or_else(|| AppError::bad_request("Incident 尚无已完成诊断，不能写入历史案例"))?;

        let document = build_case_document(&incident, &run);
        let expr = format!(
            "metadata[\"doc_type\"] == \"incident_case\" && metadata[\"incident_id\"] == \"{}\"",
            escape_expr(&incident.id)
        );
        self.vector_search_service
            .milvus_service()
            .delete_by_expr(&expr)
            .await?;
        self.vector_search_service
            .milvus_service()
            .load_collection()
            .await?;
        self.vector_search_service
            .milvus_service()
            .insert(&[MilvusDocument {
                id: document.id.clone(),
                content: document.content.clone(),
                vector: self
                    .embedding_service
                    .generate_embedding(&document.content)
                    .await?,
                sparse_vector: generate_sparse_vector(&document.content),
                metadata: document.metadata,
            }])
            .await?;

        Ok(ArchiveResult {
            success: true,
            incident_id: incident_id.to_string(),
            document_id: Some(document.id),
            message: "历史案例已写入知识库".to_string(),
        })
    }

    pub async fn similar_cases(
        &self,
        incident_id: &str,
        top_k: usize,
    ) -> Result<Vec<SearchResult>, AppError> {
        let incident = self
            .get(incident_id)
            .ok_or_else(|| AppError::bad_request(format!("Incident 不存在: {incident_id}")))?;
        let query = build_incident_case_query(&incident);
        self.vector_search_service
            .search_incident_cases(&query, top_k.max(1))
            .await
    }

    /// 标记诊断开始（对齐 Java `markRunRunning`）。
    pub fn mark_running(&self, incident_id: &str, run_id: &str) {
        mutate_run(&self.incidents, incident_id, run_id, |run| {
            let now = now_millis();
            run.status = "RUNNING".to_string();
            run.started_at = now;
            run.current_step = "正在拆解诊断任务".to_string();
            run.progress_message = "AI Ops 诊断已开始".to_string();
            run.current_tool = String::new();
        });
    }

    /// 标记正在等待工具返回（对齐 Java `markRunWaitingTool`）。
    pub fn mark_waiting_tool(
        &self,
        incident_id: &str,
        run_id: &str,
        tool_name: &str,
        query_params: &str,
    ) {
        mutate_run(&self.incidents, incident_id, run_id, |run| {
            apply_waiting_tool(run, tool_name, query_params);
        });
    }

    /// 追加一条工具证据并回到 RUNNING（对齐 Java `addToolEvidence`）。
    pub fn add_tool_evidence(&self, incident_id: &str, run_id: &str, evidence: DiagnosisEvidence) {
        mutate_run(&self.incidents, incident_id, run_id, |run| {
            apply_tool_evidence(run, evidence);
        });
    }

    /// 更新 run 的告警上下文并同步 alert_context 证据（对齐 Java `updateRunAlertContext`）。
    pub fn update_run_alert_context(&self, incident_id: &str, run_id: &str, alert_context: &str) {
        mutate_run(&self.incidents, incident_id, run_id, |run| {
            run.alert_context = alert_context.to_string();
            if let Some(evidence) = run
                .evidence
                .iter_mut()
                .find(|item| item.evidence_type == "alert_context")
            {
                evidence.content = alert_context.to_string();
                evidence.summary = alert_context.to_string();
                evidence.raw_fragment = alert_context.to_string();
            } else {
                run.evidence.push(context_evidence(
                    "alert_context",
                    "注入给 AI 的告警上下文",
                    alert_context,
                    now_millis(),
                ));
            }
        });
    }

    /// 完成诊断：写入经证据守卫后处理的报告（对齐 Java `completeRun` + `guardReport`）。
    pub fn complete_run(&self, incident_id: &str, run_id: &str, report: &str) {
        mutate_run(&self.incidents, incident_id, run_id, |run| {
            let now = now_millis();
            if run.started_at == 0 {
                run.started_at = now;
            }
            run.status = "COMPLETED".to_string();
            run.completed_at = now;
            let guarded = constrain_report(report, &run.evidence);
            run.report = guarded;
            run.error_message = String::new();
            run.current_tool = String::new();
            run.current_step = "诊断完成".to_string();
            run.progress_message = "已生成诊断报告".to_string();
        });
    }

    /// 标记诊断失败（对齐 Java `failRun`）。
    pub fn fail_run(&self, incident_id: &str, run_id: &str, error_message: &str) {
        mutate_run(&self.incidents, incident_id, run_id, |run| {
            let now = now_millis();
            if run.started_at == 0 {
                run.started_at = now;
            }
            run.status = "FAILED".to_string();
            run.completed_at = now;
            run.error_message = error_message.to_string();
            run.current_tool = String::new();
            run.current_step = "诊断失败".to_string();
            run.progress_message = error_message.to_string();
            run.evidence.push(context_evidence(
                "failure_reason",
                "诊断失败原因",
                error_message,
                now,
            ));
        });
    }

    /// 构建绑定到指定 run 的证据收集器，供 AI Ops 工具装饰器实时写入证据
    /// （对齐 Java `DiagnosisEvidenceRecorder.withRun` 的 run 作用域）。
    pub fn evidence_collector(&self, incident_id: &str, run_id: &str) -> EvidenceCollector {
        EvidenceCollector {
            incidents: Arc::clone(&self.incidents),
            incident_id: incident_id.to_string(),
            run_id: run_id.to_string(),
        }
    }
}

struct CaseDocument {
    id: String,
    content: String,
    metadata: Value,
}

fn build_case_document(incident: &IncidentRecord, run: &DiagnosisRun) -> CaseDocument {
    let root_cause = extract_section(&run.report, "根因结论")
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| compact(&run.report, 240));
    let action = extract_section(&run.report, "处理建议")
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "未提取到结构化处理动作".to_string());

    let content = format!(
        "# 历史故障案例\nIncident ID: {}\n标题: {}\n级别: {}\n告警名: {}\n服务: {}\n实例: {}\n\n## 根因\n{}\n\n## 处理动作\n{}\n\n## 原始诊断报告\n{}\n",
        incident.id,
        value(&incident.title),
        value(&incident.severity),
        value(&incident.title),
        "",
        "",
        root_cause,
        action,
        compact(&run.report, 2000)
    );
    let id = Uuid::new_v5(
        &Uuid::NAMESPACE_OID,
        format!("incident_case:{}", incident.id).as_bytes(),
    )
    .to_string();

    CaseDocument {
        id,
        content,
        metadata: json!({
            "_source": format!("incident_case:{}", incident.id),
            "doc_type": "incident_case",
            "incident_id": incident.id,
            "run_id": run.run_id,
            "alertname": value(&incident.title),
            "service": "",
            "instance": "",
            "severity": value(&incident.severity),
            "root_cause": root_cause,
            "archived_at": now_millis(),
        }),
    }
}

fn seed_incident() -> IncidentRecord {
    IncidentRecord {
        id: "incident-1".to_string(),
        aggregation_key: "HighCPUUsage:payment-service".to_string(),
        title: "HighCPUUsage payment-service".to_string(),
        status: "OPEN".to_string(),
        severity: "critical".to_string(),
        created_at: 1_718_559_600_000,
        updated_at: 1_718_559_960_000,
        last_alert_at: 1_718_559_990_000,
        alert_count: 2,
        diagnosis_runs: vec![DiagnosisRun {
            run_id: "run-1".to_string(),
            incident_id: "incident-1".to_string(),
            status: "COMPLETED".to_string(),
            created_at: 1_718_559_610_000,
            started_at: 1_718_559_620_000,
            completed_at: 1_718_559_950_000,
            alert_context: "HighCPUUsage payment-service critical".to_string(),
            report: "# HighCPUUsage payment-service\n\n初步判断为 CPU 饱和导致请求延迟上升。"
                .to_string(),
            progress_message: "诊断已完成".to_string(),
            evidence: vec![DiagnosisEvidence {
                id: "evidence-1".to_string(),
                evidence_type: "metrics".to_string(),
                title: "CPU 使用率持续高位".to_string(),
                content: "payment-service CPU 使用率超过 90%。".to_string(),
                tool_name: "queryMetrics".to_string(),
                success: true,
                created_at: 1_718_559_630_000,
                ..DiagnosisEvidence::default()
            }],
            ..DiagnosisRun::default()
        }],
    }
}

fn summary(record: &IncidentRecord) -> IncidentSummary {
    IncidentSummary {
        id: record.id.clone(),
        title: record.title.clone(),
        status: record.status.clone(),
        severity: record.severity.clone(),
        alert_count: record.alert_count,
        latest_run_status: record
            .diagnosis_runs
            .last()
            .map(|run| run.status.clone())
            .unwrap_or_default(),
        created_at: record.created_at,
        updated_at: record.updated_at,
        last_alert_at: record.last_alert_at,
    }
}

fn latest_completed_run(incident: &IncidentRecord) -> Option<DiagnosisRun> {
    incident
        .diagnosis_runs
        .iter()
        .rev()
        .find(|run| run.status == "COMPLETED" && !run.report.trim().is_empty())
        .cloned()
}

fn build_incident_case_query(incident: &IncidentRecord) -> String {
    [incident.title.as_str(), incident.severity.as_str()]
        .into_iter()
        .filter(|part| !part.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn build_alert_context(incident: &IncidentRecord) -> String {
    format!(
        "{} severity={} alertCount={}",
        incident.title, incident.severity, incident.alert_count
    )
}

fn extract_section(report: &str, heading: &str) -> Option<String> {
    let marker = format!("## {heading}");
    let start = report.find(&marker)? + marker.len();
    let rest = &report[start..];
    let end = rest.find("\n## ").unwrap_or(rest.len());
    Some(rest[..end].trim().to_string())
}

fn compact(value: &str, max_len: usize) -> String {
    let mut result = value.chars().take(max_len).collect::<String>();
    if value.chars().count() > max_len {
        result.push_str("...");
    }
    result
}

fn value(value: &str) -> &str {
    if value.trim().is_empty() {
        "N/A"
    } else {
        value
    }
}

fn escape_expr(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("系统时间早于 Unix 纪元")
        .as_millis() as i64
}

/// 绑定到具体诊断 run 的证据收集器，由 AI Ops 工具装饰器在每次工具调用时调用。
/// 与 [`IncidentService`] 共享同一份事故存储，写入实时可见
/// （对齐 Java `DiagnosisEvidenceRecorder` 借助单例 `IncidentService` 持久化证据）。
#[derive(Clone)]
pub struct EvidenceCollector {
    incidents: Arc<Mutex<HashMap<String, IncidentRecord>>>,
    incident_id: String,
    run_id: String,
}

impl EvidenceCollector {
    pub fn incident_id(&self) -> &str {
        &self.incident_id
    }

    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    /// 工具调用前：标记 run 进入等待工具状态（对齐 `markRunWaitingTool`）。
    pub fn mark_waiting(&self, tool_name: &str, query_params: &str) {
        mutate_run(&self.incidents, &self.incident_id, &self.run_id, |run| {
            apply_waiting_tool(run, tool_name, query_params);
        });
    }

    /// 工具调用后：追加证据并回到 RUNNING（对齐 `addToolEvidence`）。
    pub fn record(&self, evidence: DiagnosisEvidence) {
        mutate_run(&self.incidents, &self.incident_id, &self.run_id, |run| {
            apply_tool_evidence(run, evidence);
        });
    }

    /// 读取当前 run 已累积的证据快照，供调用 AI Ops 前增强告警上下文。
    pub fn snapshot_evidence(&self) -> Vec<DiagnosisEvidence> {
        let incidents = self.incidents.lock().expect("事故存储锁已损坏");
        incidents
            .get(&self.incident_id)
            .and_then(|incident| {
                incident
                    .diagnosis_runs
                    .iter()
                    .find(|run| run.run_id == self.run_id)
            })
            .map(|run| run.evidence.clone())
            .unwrap_or_default()
    }
}

/// 在锁内定位并修改指定 run，随后刷新 incident 的 `updated_at`。
fn mutate_run<F>(
    incidents: &Mutex<HashMap<String, IncidentRecord>>,
    incident_id: &str,
    run_id: &str,
    mutator: F,
) where
    F: FnOnce(&mut DiagnosisRun),
{
    let mut incidents = incidents.lock().expect("事故存储锁已损坏");
    let Some(incident) = incidents.get_mut(incident_id) else {
        return;
    };
    let mutated = match incident
        .diagnosis_runs
        .iter_mut()
        .find(|run| run.run_id == run_id)
    {
        Some(run) => {
            mutator(run);
            true
        }
        None => false,
    };
    if mutated {
        incident.updated_at = now_millis();
    }
}

/// 标记 run 正在调用某工具（被 IncidentService 与 EvidenceCollector 共用）。
fn apply_waiting_tool(run: &mut DiagnosisRun, tool_name: &str, query_params: &str) {
    if run.started_at == 0 {
        run.started_at = now_millis();
    }
    run.status = "WAITING_TOOL".to_string();
    run.current_tool = tool_name.to_string();
    run.current_step = format!("正在调用工具 {tool_name}");
    run.progress_message = query_params.to_string();
}

/// 追加工具证据并回到 RUNNING（被 IncidentService 与 EvidenceCollector 共用）。
fn apply_tool_evidence(run: &mut DiagnosisRun, evidence: DiagnosisEvidence) {
    let tool_name = evidence.tool_name.clone();
    let summary = evidence.summary.clone();
    run.evidence.push(evidence);
    run.status = "RUNNING".to_string();
    run.current_tool = String::new();
    run.current_step = format!("已完成工具调用 {tool_name}");
    run.progress_message = summary;
}

/// 构造非工具类证据（alert_context / failure_reason 等，对齐 Java `DiagnosisEvidence.of`）。
fn context_evidence(
    evidence_type: &str,
    title: &str,
    content: &str,
    created_at: i64,
) -> DiagnosisEvidence {
    DiagnosisEvidence {
        id: format!("ev-{}", &Uuid::new_v4().simple().to_string()[..12]),
        evidence_type: evidence_type.to_string(),
        title: title.to_string(),
        content: content.to_string(),
        summary: content.to_string(),
        raw_fragment: content.to_string(),
        success: true,
        created_at,
        ..DiagnosisEvidence::default()
    }
}
