//! Claude / Anthropic compatible routes under `/claude/v1`.
//!
//! Compatible endpoints:
//! - `POST /messages`           — create a message (stream + non-stream)
//! - `GET  /models`             — list models
//! - `GET  /models/{model_id}`  — retrieve model

pub mod json;
mod messages;
pub mod models;

use axum::Router;
use axum::middleware;
use axum::routing::{get, post};

use super::request_log::log_request;
use super::state::AppState;

pub fn router() -> Router<AppState> {
  Router::new()
    .route("/messages", post(messages::create_message))
    .route("/models", get(models::list_models))
    .route("/models/{model_id}", get(models::get_model))
    // Access log for Claude-compatible surface (method/path/status/latency).
    .layer(middleware::from_fn(log_request))
}
