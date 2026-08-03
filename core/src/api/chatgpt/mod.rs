//! ChatGPT / OpenAI compatible routes under `/chatgpt/v1`.

mod chat_completions;
mod responses;

use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::Json;
use axum::Router;

use super::models;
use super::state::AppState;
use crate::error::AppResult;
use crate::models::openai::{ModelList, ModelObject};

pub fn router() -> Router<AppState> {
  Router::new()
    .route("/chat/completions", post(chat_completions::chat_completions))
    // Correct spelling plus the typo from the original request for compatibility
    .route("/responses", post(responses::create_response))
    .route("/repsponses", post(responses::create_response))
    .route("/models", get(list_models))
    .route("/models/{model}", get(get_model))
}

/// `GET /chatgpt/v1/models` — list models from all installed agent CLIs.
async fn list_models(State(state): State<AppState>) -> Json<ModelList> {
  Json(models::openai_model_list(&state.config).await)
}

/// `GET /chatgpt/v1/models/{model}` — retrieve one model if its agent is installed.
async fn get_model(
  State(state): State<AppState>,
  Path(model): Path<String>,
) -> AppResult<Json<ModelObject>> {
  Ok(Json(
    models::openai_get_model(&state.config, &model).await?,
  ))
}
