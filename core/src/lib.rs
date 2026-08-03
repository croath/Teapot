//! # teaport-core
//!
//! Core library that exposes **ChatGPT-compatible** and **Claude-compatible** HTTP APIs
//! and fulfills requests by invoking local agent CLIs (`codex`, `claude`, `grok`,
//! `antigravity-cli`, …).

pub mod agents;
pub mod api;
pub mod config;
pub mod error;
pub mod models;
pub mod stream;

pub use agents::{
  discover_models, is_agent_installed, list_agent_infos, AgentEvent, AgentInfo, AgentRunner,
  AgentSession, DiscoveredModel, ModelSource,
};
pub use api::{
  anthropic_get_model, anthropic_model_list, build_router, list_discovered, openai_get_model,
  openai_model_list, serve, serve_with_shutdown, start_server, AppState, ServerHandle,
};
pub use config::{default_config_paths, AgentConfig, Config};
pub use error::{AppError, AppResult};
