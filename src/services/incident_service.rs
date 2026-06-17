use crate::domain::{
    diagnosis::{DiagnosisEvidence, DiagnosisRun},
    incident::{ArchiveResult, IncidentRecord, IncidentSummary, SearchResult},
};
use std::{
    collections::HashMap,
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

pub struct IncidentService {
    incidents: Mutex<HashMap<String, IncidentRecord>>,
}

impl IncidentService {
    pub fn new() -> Self {
        let incident = seed_incident();
        Self {
            incidents: Mutex::new(HashMap::from([(incident.id.clone(), incident)])),
        }
    }

    pub fn list(&self) -> Vec<IncidentSummary> {
        let mut summaries = self
            .incidents
            .lock()
            .expect("incidents lock poisoned")
            .values()
            .map(summary)
            .collect::<Vec<_>>();
        summaries.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
        summaries
    }

    pub fn get(&self, incident_id: &str) -> Option<IncidentRecord> {
        self.incidents
            .lock()
            .expect("incidents lock poisoned")
            .get(incident_id)
            .cloned()
    }

    pub fn runs(&self, incident_id: &str) -> Option<Vec<DiagnosisRun>> {
        self.get(incident_id)
            .map(|incident| incident.diagnosis_runs)
    }

    pub fn diagnose(&self, incident_id: &str) -> Option<DiagnosisRun> {
        let mut incidents = self.incidents.lock().expect("incidents lock poisoned");
        let incident = incidents.get_mut(incident_id)?;
        let now = now_millis();
        let run = DiagnosisRun {
            run_id: format!("run-{}", uuid::Uuid::new_v4()),
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

    pub fn archive_case(&self, incident_id: &str) -> ArchiveResult {
        ArchiveResult {
            success: self.get(incident_id).is_some(),
            incident_id: incident_id.to_string(),
            message: if self.get(incident_id).is_some() {
                "历史案例已写入知识库".to_string()
            } else {
                "Incident 不存在".to_string()
            },
        }
    }

    pub fn similar_cases(&self, incident_id: &str, top_k: usize) -> Vec<SearchResult> {
        if self.get(incident_id).is_none() {
            return Vec::new();
        }

        vec![SearchResult {
            id: "case-1".to_string(),
            content: "历史案例: payment-service 高 CPU 后伴随错误率上升，根因曾为线程池耗尽。"
                .to_string(),
            score: 0.82,
            metadata: r#"{"doc_type":"incident_case","source":"seed"}"#.to_string(),
        }]
        .into_iter()
        .take(top_k)
        .collect()
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

fn build_alert_context(incident: &IncidentRecord) -> String {
    format!(
        "{} severity={} alertCount={}",
        incident.title, incident.severity, incident.alert_count
    )
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_millis() as i64
}
