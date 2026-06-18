use crate::services::{
    chat_service::ChatService, incident_service::IncidentService, session_manager::SessionManager,
};

pub struct AppState {
    pub chat_service: ChatService,
    pub session_manager: SessionManager,
    pub incident_service: IncidentService,
}

impl AppState {
    pub fn new(
        chat_service: ChatService,
        session_manager: SessionManager,
        incident_service: IncidentService,
    ) -> Self {
        Self {
            chat_service,
            session_manager,
            incident_service,
        }
    }
}
