use crate::services::{chat_service::ChatService, incident_service::IncidentService};

pub struct AppState {
    pub chat_service: ChatService,
    pub incident_service: IncidentService,
}

impl AppState {
    pub fn new(chat_service: ChatService, incident_service: IncidentService) -> Self {
        Self {
            chat_service,
            incident_service,
        }
    }
}
