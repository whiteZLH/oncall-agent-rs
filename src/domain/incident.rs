use serde::Serialize;

use super::diagnosis::DiagnosisRun;
pub use super::rag::SearchResult;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IncidentSummary {
    pub id: String,
    pub title: String,
    pub status: String,
    pub severity: String,
    pub alert_count: i32,
    pub latest_run_status: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub last_alert_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IncidentRecord {
    pub id: String,
    pub aggregation_key: String,
    pub title: String,
    pub status: String,
    pub severity: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub last_alert_at: i64,
    pub alert_count: i32,
    pub diagnosis_runs: Vec<DiagnosisRun>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveResult {
    pub success: bool,
    pub incident_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub document_id: Option<String>,
    pub message: String,
}
