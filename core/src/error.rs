//! Shared error types for the teapot core library.
//!
//! Wire formats by surface:
//! - **ChatGPT-compatible** (`/chatgpt/…`): [`OpenAiError`] / [`OpenAiResult`]
//! - **Claude-compatible** (`/claude/…`): [`ClaudeError`] / [`ClaudeResult`]
//! - **Other APIs** (health, future internal routes): [`AppError`] / [`AppResult`]

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use thiserror::Error;

/// Application-level error used by providers, stores, and non-compatible APIs.
#[derive(Debug, Error)]
pub enum AppError {
  #[error("provider not found: {0}")]
  ProviderNotFound(String),

  #[error("provider binary not found: {0}")]
  ProviderBinaryMissing(String),

  #[error("provider execution failed: {0}")]
  ProviderFailed(String),

  #[error("invalid request: {0}")]
  BadRequest(String),

  #[error("not found: {0}")]
  NotFound(String),

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
      Self::ProviderNotFound(_) | Self::ProviderBinaryMissing(_) | Self::NotFound(_) => {
        StatusCode::NOT_FOUND
      }
      Self::BadRequest(_) => StatusCode::BAD_REQUEST,
      Self::Unauthorized(_) => StatusCode::UNAUTHORIZED,
      Self::ProviderFailed(_)
      | Self::Internal(_)
      | Self::Io(_)
      | Self::Json(_)
      | Self::Anyhow(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
  }

  /// Shared logical error class (used by OpenAI / Anthropic / Teapot wire bodies).
  pub fn error_type(&self) -> &'static str {
    match self {
      Self::ProviderNotFound(_) | Self::ProviderBinaryMissing(_) | Self::NotFound(_) => {
        "not_found_error"
      }
      Self::BadRequest(_) => "invalid_request_error",
      Self::Unauthorized(_) => "authentication_error",
      _ => "api_error",
    }
  }

  /// Render as an OpenAI-compatible error response.
  pub fn into_openai_response(self) -> Response {
    OpenAiError(self).into_response()
  }

  /// Render as an Anthropic / Claude-compatible error response.
  pub fn into_anthropic_response(self) -> Response {
    ClaudeError(self).into_response()
  }
}

/// Teapot-native error body for non-compatible APIs.
///
/// ```json
/// {
///   "error": "invalid request: …",
///   "type": "invalid_request_error"
/// }
/// ```
#[derive(Debug, Clone, Serialize)]
struct TeapotErrorBody {
  error: String,
  #[serde(rename = "type")]
  error_type: &'static str,
}

impl IntoResponse for AppError {
  fn into_response(self) -> Response {
    let status = self.status_code();
    let body = Json(TeapotErrorBody {
      error: self.to_string(),
      error_type: self.error_type(),
    });
    (status, body).into_response()
  }
}

// ---------------------------------------------------------------------------
// OpenAI-compatible
// ---------------------------------------------------------------------------

/// OpenAI-compatible error envelope returned by ChatGPT-compatible handlers.
///
/// ```json
/// {
///   "error": {
///     "message": "…",
///     "type": "invalid_request_error",
///     "param": null,
///     "code": null
///   }
/// }
/// ```
#[derive(Debug, Clone, Serialize)]
pub struct OpenAiErrorBody {
  pub error: OpenAiErrorDetail,
}

/// Detail object inside [`OpenAiErrorBody`].
#[derive(Debug, Clone, Serialize)]
pub struct OpenAiErrorDetail {
  pub message: String,
  #[serde(rename = "type")]
  pub error_type: String,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub param: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub code: Option<String>,
}

impl OpenAiErrorBody {
  /// Build a body with `type: invalid_request_error` (typical JSON / validation failure).
  pub fn invalid_request(message: impl Into<String>) -> Self {
    Self {
      error: OpenAiErrorDetail {
        message: message.into(),
        error_type: "invalid_request_error".into(),
        param: None,
        code: None,
      },
    }
  }

  pub fn new(
    message: impl Into<String>,
    error_type: impl Into<String>,
    param: Option<String>,
    code: Option<String>,
  ) -> Self {
    Self {
      error: OpenAiErrorDetail {
        message: message.into(),
        error_type: error_type.into(),
        param,
        code,
      },
    }
  }

  pub fn from_app_error(err: &AppError) -> Self {
    Self::new(err.to_string(), err.error_type(), None, None)
  }
}

/// [`AppError`] wrapper that serializes to OpenAI's error shape.
///
/// Use as the error type for ChatGPT-compatible handlers ([`OpenAiResult`]).
#[derive(Debug)]
pub struct OpenAiError(pub AppError);

impl OpenAiError {
  pub fn bad_request(message: impl Into<String>) -> Self {
    Self(AppError::BadRequest(message.into()))
  }
}

impl From<AppError> for OpenAiError {
  fn from(err: AppError) -> Self {
    Self(err)
  }
}

impl std::fmt::Display for OpenAiError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    self.0.fmt(f)
  }
}

impl std::error::Error for OpenAiError {
  fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
    Some(&self.0)
  }
}

impl IntoResponse for OpenAiError {
  fn into_response(self) -> Response {
    let status = self.0.status_code();
    let body = Json(OpenAiErrorBody::from_app_error(&self.0));
    (status, body).into_response()
  }
}

/// Result alias for ChatGPT-compatible handlers (OpenAI error body).
pub type OpenAiResult<T> = Result<T, OpenAiError>;

// ---------------------------------------------------------------------------
// Anthropic / Claude-compatible
// ---------------------------------------------------------------------------

/// Anthropic Messages API error envelope.
///
/// ```json
/// {
///   "type": "error",
///   "error": {
///     "type": "invalid_request_error",
///     "message": "…"
///   }
/// }
/// ```
#[derive(Debug, Clone, Serialize)]
pub struct AnthropicErrorBody {
  #[serde(rename = "type")]
  pub object_type: &'static str,
  pub error: AnthropicErrorDetail,
}

/// Detail object inside [`AnthropicErrorBody`].
#[derive(Debug, Clone, Serialize)]
pub struct AnthropicErrorDetail {
  #[serde(rename = "type")]
  pub error_type: String,
  pub message: String,
}

impl AnthropicErrorBody {
  pub fn new(message: impl Into<String>, error_type: impl Into<String>) -> Self {
    Self {
      object_type: "error",
      error: AnthropicErrorDetail {
        error_type: error_type.into(),
        message: message.into(),
      },
    }
  }

  /// Build a body with `error.type: invalid_request_error`.
  pub fn invalid_request(message: impl Into<String>) -> Self {
    Self::new(message, "invalid_request_error")
  }

  pub fn from_app_error(err: &AppError) -> Self {
    Self::new(err.to_string(), err.error_type())
  }
}

/// [`AppError`] wrapper that serializes to Anthropic's error shape.
///
/// Use as the error type for Claude-compatible handlers ([`ClaudeResult`]).
#[derive(Debug)]
pub struct ClaudeError(pub AppError);

impl ClaudeError {
  pub fn bad_request(message: impl Into<String>) -> Self {
    Self(AppError::BadRequest(message.into()))
  }
}

impl From<AppError> for ClaudeError {
  fn from(err: AppError) -> Self {
    Self(err)
  }
}

impl std::fmt::Display for ClaudeError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    self.0.fmt(f)
  }
}

impl std::error::Error for ClaudeError {
  fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
    Some(&self.0)
  }
}

impl IntoResponse for ClaudeError {
  fn into_response(self) -> Response {
    let status = self.0.status_code();
    let body = Json(AnthropicErrorBody::from_app_error(&self.0));
    (status, body).into_response()
  }
}

/// Result alias for Claude-compatible handlers (Anthropic error body).
pub type ClaudeResult<T> = Result<T, ClaudeError>;

// ---------------------------------------------------------------------------
// Generic / non-compatible
// ---------------------------------------------------------------------------

/// Result alias using [`AppError`] (Teapot-native error body via [`IntoResponse`]).
///
/// Prefer [`OpenAiResult`] / [`ClaudeResult`] on compatible API surfaces.
pub type AppResult<T> = Result<T, AppError>;
