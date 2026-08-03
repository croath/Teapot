//! Claude / Anthropic compatible routes under `/claude/v1`.
//!
//! Compatible endpoints:
//! - `POST /messages`
//! - `GET  /models`
//! - `GET  /models/{model_id}`

pub mod json;
mod messages;
pub mod models;

use axum::Router;
use axum::routing::{get, post};

use super::state::AppState;

pub fn router() -> Router<AppState> {
  Router::new()
    .route("/messages", post(messages::create_message))
    .route("/models", get(models::list_models))
    .route("/models/{model_id}", get(models::get_model))
}
