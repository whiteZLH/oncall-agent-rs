use crate::domain::incident::IncidentSummary;

pub struct IncidentService;

impl IncidentService {
    pub fn new() -> Self {
        Self
    }

    pub fn list(&self) -> Vec<IncidentSummary> {
        vec![IncidentSummary {
            id: "INC-1001".to_string(),
            title: "API error rate is elevated".to_string(),
            status: "OPEN".to_string(),
        }]
    }
}
