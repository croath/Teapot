//! # teapot-core
//!
//! Core library that exposes **ChatGPT-compatible** and **Claude-compatible** HTTP APIs
//! and fulfills requests via provider HTTP backends with in-memory access tokens.

pub mod api;
pub mod auth;
pub mod config;
pub mod error;
pub mod models;
pub mod paths;
pub mod providers;
pub mod stream;

pub use api::{
  ApiSurface, AppState, ChatGptSurface, ClaudeSurface, ServerHandle, anthropic_get_model,
  anthropic_model_list, build_router, build_router_with_surfaces, default_surface_list,
  mount_surfaces, openai_get_model, openai_model_list, serve, serve_with_shutdown, start_server,
};
pub use auth::{
  AuthMethod, AuthStore, LoginOptions, default_auth_dir, default_auth_path, open_url,
};
pub use config::{Config, default_config_paths};
pub use error::{
  AnthropicErrorBody, AnthropicErrorDetail, AppError, AppResult, ClaudeError, ClaudeResult,
  OpenAiError, OpenAiErrorBody, OpenAiErrorDetail, OpenAiResult,
};
pub use paths::{
  APP_IDENTIFIER, DATA_DIR_ENV, MigrateReport, copy_file_if_missing, data_local_dir,
  default_config_file, ensure_legacy_migrated, migrate_from_dirs, migrate_legacy_project_data,
  project_dirs, remove_legacy_dirs,
};
pub use providers::{
  AntigravityProvider, AuthEntry, ClaudeProvider, CodexCliProvider, CodexProvider, ExecRequest,
  ExecResponse, ExecStream, ExecStreamEvent, ModelInfo, ModelsCache, ModelsStore,
  NativeModelCatalog, PinnedProvider, PromptRequest, Provider, ProviderAuth, ProviderEvent,
  ProviderExecutor, ProviderKind, ProviderModel, ProviderRuntime, ProviderSession, SpawnSpec,
  StdoutCodec, VertexProvider, VertexSession, XaiProvider, all_providers, augmented_path,
  default_models_dir, expand_args, family_for_name, flatten_messages, import_service_account,
  offered_providers, pinned_provider, provider_by_name, provider_for, resolve_binary, stdin_prompt,
};
