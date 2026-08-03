//! HTTP API surface (ChatGPT + Claude compatible).

mod auth;
pub mod chatgpt;
pub mod claude;
mod models;
mod server;
mod state;

pub use models::{
  anthropic_get_model, anthropic_model_list, list_discovered, openai_get_model, openai_model_list,
};
pub use server::{build_router, serve, serve_with_shutdown, start_server, ServerHandle};
pub use state::AppState;
