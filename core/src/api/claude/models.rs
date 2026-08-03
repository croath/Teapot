//! Anthropic-compatible models API under `/claude/v1`.
//!
//! Reads the in-memory catalog seeded for the pinned [`crate::providers::PinnedProvider`].

use axum::extract::{Path, State};

use super::super::state::AppState;
use super::json::ClaudeJson;
use crate::error::{AppError, AppResult, ClaudeResult};
use crate::models::anthropic::{ModelList, ModelObject};

/// `GET /claude/v1/models`
pub async fn list_models(State(state): State<AppState>) -> ClaudeResult<ClaudeJson<ModelList>> {
  Ok(ClaudeJson(anthropic_model_list(&state).await?))
}

/// `GET /claude/v1/models/{model_id}`
pub async fn get_model(
  State(state): State<AppState>,
  Path(model_id): Path<String>,
) -> ClaudeResult<ClaudeJson<ModelObject>> {
  Ok(ClaudeJson(anthropic_get_model(&state, &model_id).await?))
}

/// Build an Anthropic-compatible `GET /models` body from the pinned provider catalog.
pub async fn anthropic_model_list(state: &AppState) -> AppResult<ModelList> {
  let models = state.runtime.models().list().await;
  let data: Vec<ModelObject> = models.iter().map(|m| m.to_anthropic()).collect();
  let first_id = data.first().map(|m| m.id.clone());
  let last_id = data.last().map(|m| m.id.clone());
  Ok(ModelList {
    data,
    has_more: false,
    first_id,
    last_id,
  })
}

/// Build an Anthropic-compatible `GET /models/{id}` body from the pinned provider catalog.
pub async fn anthropic_get_model(state: &AppState, model_id: &str) -> AppResult<ModelObject> {
  let info = state
    .runtime
    .models()
    .get(model_id)
    .await
    .ok_or_else(|| AppError::NotFound(format!("model not found: {model_id}")))?;
  Ok(info.to_anthropic())
}
