//! Claude / Anthropic compatible routes under `/claude/v1`.

mod messages;

use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::Json;
use axum::Router;

use super::models;
use super::state::AppState;
use crate::error::AppResult;
use crate::models::anthropic::{ModelList, ModelObject};

pub fn router() -> Router<AppState> {
  Router::new()
    .route("/messages", post(messages::create_message))
    .route("/models", get(list_models))
    .route("/models/{model_id}", get(get_model))
}

/// `GET /claude/v1/models` — list models from all installed agent CLIs.
async fn list_models(State(state): State<AppState>) -> Json<ModelList> {
  Json(models::anthropic_model_list(&state.config).await)
}

/// `GET /claude/v1/models/{model_id}` — retrieve one model if its agent is installed.
async fn get_model(
  State(state): State<AppState>,
  Path(model_id): Path<String>,
) -> AppResult<Json<ModelObject>> {
  Ok(Json(
    models::anthropic_get_model(&state.config, &model_id).await?,
  ))
}
