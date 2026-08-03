//! ChatGPT / OpenAI compatible routes under `/chatgpt/v1`.
//!
//! Compatible endpoints:
//! - `POST /chat/completions`   — create chat completion (stream + non-stream)
//! - `POST /responses`          — create response (stream + non-stream)
//! - `POST /responses/compact`  — compact a response (JSON or stream)
//! - `GET  /models`             — list models
//! - `GET  /models/{model}`     — retrieve model

mod chat_completions;
pub mod json;
pub mod models;
mod responses;

use axum::Router;
use axum::middleware;
use axum::routing::{get, post};
pub use json::OpenAiJson;

use super::request_log::log_request;
use super::state::AppState;

pub fn router() -> Router<AppState> {
  Router::new()
    .route(
      "/chat/completions",
      post(chat_completions::chat_completions),
    )
    .route("/responses/compact", post(responses::compact_response))
    .route("/responses", post(responses::create_response))
    .route("/models", get(models::list_models))
    .route("/models/{model}", get(models::get_model))
    // Access log for ChatGPT-compatible surface (method/path/status/latency).
    .layer(middleware::from_fn(log_request))
}
