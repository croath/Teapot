//! Shared error types for the teaport core library.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;
use thiserror::Error;

/// Application-level error returned by API handlers and agent runners.
#[derive(Debug, Error)]
pub enum AppError {
    #[error("agent not found: {0}")]
    AgentNotFound(String),

    #[error("agent binary not found: {0}")]
    AgentBinaryMissing(String),

    #[error("agent execution failed: {0}")]
    AgentFailed(String),

    #[error("invalid request: {0}")]
    BadRequest(String),

    #[error("unauthorized: {0}")]
    Unauthorized(String),

    #[error("internal error: {0}")]
    Internal(String),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Json(#[from] serde_json::Error),

    #[error(transparent)]
    Anyhow(#[from] anyhow::Error),
}

impl AppError {
    pub fn status_code(&self) -> StatusCode {
        match self {
            Self::AgentNotFound(_) | Self::AgentBinaryMissing(_) => StatusCode::NOT_FOUND,
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            Self::AgentFailed(_) | Self::Internal(_) | Self::Io(_) | Self::Json(_) | Self::Anyhow(_) => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        }
    }

    pub fn error_type(&self) -> &'static str {
        match self {
            Self::AgentNotFound(_) | Self::AgentBinaryMissing(_) => "not_found_error",
            Self::BadRequest(_) => "invalid_request_error",
            Self::Unauthorized(_) => "authentication_error",
            _ => "api_error",
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = self.status_code();
        let body = Json(json!({
            "error": {
                "message": self.to_string(),
                "type": self.error_type(),
                "code": null
            }
        }));
        (status, body).into_response()
    }
}

/// Result alias using [`AppError`].
pub type AppResult<T> = Result<T, AppError>;
