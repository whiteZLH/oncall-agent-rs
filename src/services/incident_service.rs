use crate::models::IncidentSummary;

pub struct IncidentService;

impl IncidentService {
    pub fn new() -> Self {
        Self
    }

    pub fn list(&self) -> Vec<IncidentSummary> {
        Vec::new()
    }
}
