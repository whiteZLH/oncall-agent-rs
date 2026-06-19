use crate::{
    config::AppConfig,
    domain::{
        diagnosis::{DiagnosisEvidence, DiagnosisRun},
        incident::{ArchiveResult, IncidentRecord, IncidentSummary, SearchResult},
    },
    error::AppError,
    services::{
        milvus_service::MilvusDocument,
        vector_embedding_service::{generate_sparse_vector, VectorEmbeddingService},
        vector_search_service::VectorSearchService,
    },
};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

pub struct IncidentService {
    incidents: Mutex<HashMap<String, IncidentRecord>>,
    vector_search_service: VectorSearchService,
    embedding_service: VectorEmbeddingService,
}

impl IncidentService {
    pub fn new(config: &AppConfig) -> Self {
        let incident = seed_incident();
        Self {
            incidents: Mutex::new(HashMap::from([(incident.id.clone(), incident)])),
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
        let run = DiagnosisRun {
            run_id: format!("run-{}", Uuid::new_v4()),
            incident_id: incident_id.to_string(),
            status: "QUEUED".to_string(),
            created_at: now,
            alert_context: build_alert_context(incident),
            progress_message: "诊断任务已创建，等待执行".to_string(),
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
