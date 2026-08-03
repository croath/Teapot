//! OpenAI-compatible models API under `/chatgpt/v1`.
//!
//! - `GET /models`           — list models (`ModelList`)
//! - `GET /models/{model}`   — retrieve model (`Model`)
//!
//! Wire shapes match the public OpenAI Models API. Data is obtained from the
//! pinned provider's `models` / `model` methods (preferring the in-memory
//! catalog seeded at bootstrap, with live provider fallback).

use axum::extract::{Path, State};

use super::json::OpenAiJson;
use crate::api::state::AppState;
use crate::error::{AppError, AppResult, OpenAiResult};
use crate::models::openai::{Model, ModelList};

/// `GET /chatgpt/v1/models`
pub async fn list_models(State(state): State<AppState>) -> OpenAiResult<OpenAiJson<ModelList>> {
  tracing::info!("models list");
  let list = openai_model_list(&state).await?;
  tracing::info!(count = list.data.len(), "models list done");
  Ok(OpenAiJson(list))
}

/// `GET /chatgpt/v1/models/{model}`
pub async fn get_model(
  State(state): State<AppState>,
  Path(model): Path<String>,
) -> OpenAiResult<OpenAiJson<Model>> {
  tracing::info!(model = %model, "models get");
  Ok(OpenAiJson(openai_get_model(&state, &model).await?))
}

/// Build an OpenAI-compatible `GET /models` body.
///
/// Prefer the in-memory catalog (filled via each provider's `models()` at
/// bootstrap / periodic refresh). If empty, live-fetch through the pinned
/// provider and store.
pub async fn openai_model_list(state: &AppState) -> AppResult<ModelList> {
  state.runtime.refresh_access_token_if_needed().await?;

  let infos = {
    let cached = state.runtime.models().list().await;
    if !cached.is_empty() {
      cached
    } else {
      state
        .runtime
        .models()
        .fetch_and_store(state.provider.as_ref())
        .await?
    }
  };

  Ok(ModelList::new(
    infos.iter().map(|m| m.to_openai()).collect(),
  ))
}

/// Build an OpenAI-compatible `GET /models/{model}` body.
///
/// Looks up the cached catalog first; on miss, calls the provider's
/// `model(id)` (upstream retrieve or list scan).
pub async fn openai_get_model(state: &AppState, model_id: &str) -> AppResult<Model> {
  let model_id = model_id.trim();
  if model_id.is_empty() {
    return Err(AppError::BadRequest("model id must not be empty".into()));
  }

  state.runtime.refresh_access_token_if_needed().await?;

  if let Some(info) = state.runtime.models().get(model_id).await {
    return Ok(info.to_openai());
  }

  let info = state
    .provider
    .model(model_id)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("model not found: {model_id}")))?;
  Ok(info.to_openai())
}
