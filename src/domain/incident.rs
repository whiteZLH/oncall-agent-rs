use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct IncidentSummary {
    pub id: String,
    pub title: String,
    pub status: String,
}
