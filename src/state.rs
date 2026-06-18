use crate::config::AppConfig;
use crate::services::{
    chat_service::ChatService, incident_service::IncidentService,
    memory_extraction_service::MemoryExtractionService, session_manager::SessionManager,
    vector_search_service::VectorSearchService,
};

pub struct AppState {
    pub config: AppConfig,
    pub chat_service: ChatService,
    pub vector_search_service: VectorSearchService,
    pub memory_extraction_service: MemoryExtractionService,
    pub session_manager: SessionManager,
    pub incident_service: IncidentService,
}

impl AppState {
    pub fn new(
        config: AppConfig,
        chat_service: ChatService,
        vector_search_service: VectorSearchService,
        memory_extraction_service: MemoryExtractionService,
        session_manager: SessionManager,
        incident_service: IncidentService,
    ) -> Self {
        Self {
            config,
            chat_service,
            vector_search_service,
            memory_extraction_service,
            session_manager,
            incident_service,
        }
    }
}
