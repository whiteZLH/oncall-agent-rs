use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("{0}")]
    BadRequest(String),
}

impl AppError {
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::BadRequest(message.into())
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {  
        let (status, error) = match self {
            AppError::BadRequest(message) => (StatusCode::BAD_REQUEST, message),
        };

        let body = Json(crate::models::ApiErrorResponse {
            error: "request_failed",
            message: error,
        });

        (status, body).into_response()
    }
}
