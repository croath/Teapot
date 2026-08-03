//! Pinned provider runtime: session bootstrap + background tasks.
//!
//! Models live in [`ModelsCache`] (independent struct). Credentials live on
//! each provider instance (native `StoredAuth` / `VertexSession`).

use std::sync::Arc;
use std::time::Duration;

use tracing::{info, warn};

use crate::auth::AuthStore;
use crate::error::AppResult;
use crate::providers::models_cache::ModelsCache;
use crate::providers::pinned::PinnedProvider;
use crate::providers::{ExecRequest, ExecResponse, ProviderKind};

const TOKEN_POLL_SECS: u64 = 60;
const MODELS_REFRESH_SECS: u64 = 3 * 60 * 60;

/// Runtime for the single pinned provider of a server process.
pub struct ProviderRuntime {
  /// Provider instance created once at bootstrap and reused for all API work.
  /// Holds its own native session credentials.
  provider: Arc<PinnedProvider>,
  auth_store: Arc<AuthStore>,
  /// Independent models cache for this pinned provider.
  models: Arc<ModelsCache>,
}

impl ProviderRuntime {
  /// Bootstrap with an already-constructed provider instance (preferred at serve).
  pub async fn bootstrap_with_provider(
    provider: Arc<PinnedProvider>,
    auth_store: Arc<AuthStore>,
  ) -> AppResult<Arc<Self>> {
    let models = Arc::new(ModelsCache::local(provider.kind())?);
    let runtime = Arc::new(Self {
      provider,
      auth_store,
      models,
    });

    runtime.refresh_access_token().await?;
    runtime
      .models
      .load_or_fetch(runtime.provider.as_ref())
      .await?;
    runtime.spawn_background_tasks();

    Ok(runtime)
  }

  /// Bootstrap by creating the provider for `kind`.
  pub async fn bootstrap(kind: ProviderKind, auth_store: Arc<AuthStore>) -> AppResult<Arc<Self>> {
    Self::bootstrap_with_provider(Arc::new(PinnedProvider::from_kind(kind)), auth_store).await
  }

  pub fn kind(&self) -> ProviderKind {
    self.provider.kind()
  }

  /// The pinned provider instance shared with [`crate::api::state::AppState`].
  pub fn provider(&self) -> &Arc<PinnedProvider> {
    &self.provider
  }

  /// Independent models cache for the pinned provider.
  pub fn models(&self) -> &Arc<ModelsCache> {
    &self.models
  }

  /// Load / mint credentials into the pinned provider's **own** session type.
  pub async fn refresh_access_token(&self) -> AppResult<()> {
    self.provider.load_session(&self.auth_store).await?;
    info!(provider = %self.kind(), "provider session loaded into memory");
    Ok(())
  }

  /// Refresh the provider session only when missing or near expiry.
  pub async fn refresh_access_token_if_needed(&self) -> AppResult<()> {
    self
      .provider
      .refresh_session_if_needed(&self.auth_store)
      .await
  }

  /// Execute a chat request using the pinned provider's own session.
  pub async fn execute(&self, req: &ExecRequest) -> AppResult<ExecResponse> {
    self.provider.execute(req).await
  }

  fn spawn_background_tasks(self: &Arc<Self>) {
    let token_rt = Arc::clone(self);
    tokio::spawn(async move {
      let mut ticker = tokio::time::interval(Duration::from_secs(TOKEN_POLL_SECS));
      ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
      loop {
        ticker.tick().await;
        if let Err(e) = token_rt.refresh_access_token_if_needed().await {
          warn!(error = %e, "background access_token refresh failed");
        }
      }
    });

    let models_rt = Arc::clone(self);
    tokio::spawn(async move {
      let mut ticker = tokio::time::interval(Duration::from_secs(MODELS_REFRESH_SECS));
      ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
      ticker.tick().await;
      loop {
        ticker.tick().await;
        if let Err(e) = models_rt.refresh_access_token_if_needed().await {
          warn!(error = %e, "models refresh: token refresh failed");
          continue;
        }
        if let Err(e) = models_rt
          .models
          .fetch_and_store(models_rt.provider.as_ref())
          .await
        {
          warn!(error = %e, "background models refresh failed");
        }
      }
    });
  }
}
