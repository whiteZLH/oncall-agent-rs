use serde::{Deserialize, Serialize};

#[derive(Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
}

#[derive(Serialize)]
pub struct ApiErrorResponse {
    pub error: &'static str,
    pub message: String,
}

#[derive(Deserialize)]
pub struct MessageEnvelope {
    pub message: String,
}

#[derive(Serialize)]
pub struct ChatResponse {
    pub reply: String,
}

#[derive(Serialize)]
pub struct IncidentSummary {
    pub id: String,
    pub title: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DiagnosisRun {
    pub run_id: String,
    pub incident_id: String,
    pub status: String,
    pub created_at: i64,
    pub started_at: i64,
    pub completed_at: i64,
    pub alert_context: String,
    pub report: String,
    pub error_message: String,
    pub current_step: String,
    pub progress_message: String,
    pub current_tool: String,
    pub reused_from_run_id: String,
    pub reuse_reason: String,
    pub reuse_confidence: f64,
    pub reuse_validated_at: i64,
    pub evidence: Vec<DiagnosisEvidence>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DiagnosisEvidence {
    pub id: String,
    #[serde(rename = "type")]
    pub evidence_type: String,
    pub title: String,
    pub content: String,
    pub tool_name: String,
    pub query_params: String,
    pub time_range: String,
    pub summary: String,
    pub raw_fragment: String,
    pub success: bool,
    pub error_message: String,
    pub error_code: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DocumentChunk {
    pub content: String,
    pub start_index: i32,
    pub end_index: i32,
    pub chunk_index: i32,
    pub title: String,
}
