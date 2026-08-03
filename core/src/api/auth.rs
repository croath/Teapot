//! Optional API key authentication middleware.

use axum::extract::Request;
use axum::extract::State;
use axum::http::header::{AUTHORIZATION, HeaderMap};
use axum::middleware::Next;
use axum::response::Response;

use super::state::AppState;
use crate::error::AppError;

/// Extract API key from `Authorization: Bearer ...` or `x-api-key`.
pub fn extract_api_key(headers: &HeaderMap) -> Option<String> {
  if let Some(v) = headers.get("x-api-key").and_then(|v| v.to_str().ok()) {
    return Some(v.to_string());
  }
  if let Some(v) = headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()) {
    let v = v.trim();
    if let Some(rest) = v.strip_prefix("Bearer ").or_else(|| v.strip_prefix("bearer ")) {
      return Some(rest.trim().to_string());
    }
  }
  None
}

/// Axum middleware that enforces `config.api_key` when configured.
pub async fn require_api_key(
  State(state): State<AppState>,
  request: Request,
  next: Next,
) -> Result<Response, AppError> {
  if let Some(expected) = state.config.api_key.as_ref() {
    let provided = extract_api_key(request.headers());
    match provided {
      Some(key) if key == *expected => {}
      Some(_) => {
        return Err(AppError::Unauthorized("invalid API key".into()));
      }
      None => {
        return Err(AppError::Unauthorized(
          "missing API key (Authorization: Bearer or x-api-key)".into(),
        ));
      }
    }
  }
  Ok(next.run(request).await)
}
