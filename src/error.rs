use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use thiserror::Error;

use crate::http::dto::ApiErrorResponse;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("{0}")]
    BadRequest(String),
    #[error("{0}")]
    Internal(String),
}

impl AppError {
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::BadRequest(message.into())
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal(message.into())
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, code, message) = match self {
            AppError::BadRequest(message) => (StatusCode::BAD_REQUEST, 400, message),
            AppError::Internal(message) => (StatusCode::INTERNAL_SERVER_ERROR, 500, message),
        };

        let body = Json(ApiErrorResponse {
            code,
            message,
            request_id: None,
        });

        (status, body).into_response()
    }
}
