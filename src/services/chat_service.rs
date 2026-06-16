use crate::{
    error::AppError,
    http::dto::{ChatResponse, MessageEnvelope},
};

pub struct ChatService;

impl ChatService {
    pub fn new() -> Self {
        Self
    }

    pub fn reply(&self, input: MessageEnvelope) -> Result<ChatResponse, AppError> {
        let message = input.message.trim();
        if message.is_empty() {
            return Err(AppError::bad_request("message must not be empty"));
        }

        Ok(ChatResponse {
            reply: format!("oncall-agent-rs received: {}", message),
        })
    }
}
