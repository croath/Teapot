//! HTTP API surface (ChatGPT + Claude compatible).

mod auth;
pub mod chatgpt;
pub mod claude;
mod request_log;
mod server;
mod state;
pub mod surface;

pub use chatgpt::models::{openai_get_model, openai_model_list};
pub use claude::models::{anthropic_get_model, anthropic_model_list};
pub use server::{
  ServerHandle, build_router, build_router_with_surfaces, serve, serve_with_shutdown, start_server,
};
pub use state::AppState;
pub use surface::{
  ApiSurface, ChatGptSurface, ClaudeSurface, default_surface_list, mount_surfaces,
};
