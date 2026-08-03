//! Shared application state for Axum handlers.

use std::sync::Arc;

use crate::auth::AuthStore;
use crate::config::Config;
use crate::providers::{PinnedProvider, ProviderRuntime};

#[derive(Clone)]
pub struct AppState {
  pub config: Arc<Config>,
  pub auth_store: Arc<AuthStore>,
  /// Provider instance created from the CLI/config pin and reused by all APIs.
  pub provider: Arc<PinnedProvider>,
  /// In-memory access token + models cache for the pinned provider.
  pub runtime: Arc<ProviderRuntime>,
}

impl AppState {
  pub fn new(
    config: Config,
    auth_store: Arc<AuthStore>,
    provider: Arc<PinnedProvider>,
    runtime: Arc<ProviderRuntime>,
  ) -> Self {
    Self {
      config: Arc::new(config),
      auth_store,
      provider,
      runtime,
    }
  }

  /// Typed provider kind for the pinned instance.
  pub fn provider_kind(&self) -> crate::providers::ProviderKind {
    self.provider.kind()
  }
}
