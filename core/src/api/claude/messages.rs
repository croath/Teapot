//! Anthropic Messages: POST /claude/v1/messages
//!
//! Provider backends are temporarily removed — requests return an error.

use axum::extract::State;
use axum::response::Response;

use super::json::ClaudeJson;
use crate::api::state::AppState;
use crate::error::{AppError, ClaudeResult};
use crate::models::anthropic::MessagesRequest;

pub async fn create_message(
  State(_state): State<AppState>,
  ClaudeJson(req): ClaudeJson<MessagesRequest>,
) -> ClaudeResult<Response> {
  if req.messages.is_empty() {
    return Err(AppError::BadRequest("messages must not be empty".into()).into());
  }

  Err(
    AppError::ProviderNotFound("no providers registered (backends temporarily removed)".into())
      .into(),
  )
}
