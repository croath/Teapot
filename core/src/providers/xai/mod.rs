//! xAI Grok CLI (device-code OAuth)

mod auth;
mod compact;
mod execute;
mod jwt;
mod models;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::RwLock;

use crate::auth::{AuthMethod, AuthStore, LoginOptions};
use crate::error::{AppError, AppResult};
use crate::providers::{AuthEntry, ProviderKind};

use super::traits::{PromptRequest, Provider, SpawnSpec, StdoutCodec, expand_args};

pub use auth::StoredAuth;
pub use models::{XaiModel, XaiModelsResponse};

/// xAI Grok CLI (device-code OAuth).
///
/// In-memory session is the native [`StoredAuth`].
#[derive(Debug, Clone)]
pub struct XaiProvider {
  /// Reused HTTP client for discovery, device code, and token requests.
  http: reqwest::Client,
  session: Arc<RwLock<Option<StoredAuth>>>,
}

impl XaiProvider {
  pub fn new() -> Self {
    Self {
      http: build_http_client(),
      session: Arc::new(RwLock::new(None)),
    }
  }

  pub async fn set_session(&self, auth: StoredAuth) {
    let mut guard = self.session.write().await;
    *guard = Some(auth);
  }

  pub async fn session(&self) -> AppResult<StoredAuth> {
    self.session.read().await.clone().ok_or_else(|| {
      AppError::Unauthorized("xai: no session in memory; restart server after auth login".into())
    })
  }

  pub async fn session_needs_refresh(&self) -> bool {
    match self.session.read().await.as_ref() {
      None => true,
      Some(s) => s.session_needs_refresh(chrono::Duration::minutes(5)),
    }
  }
}

impl Default for XaiProvider {
  fn default() -> Self {
    Self::new()
  }
}

fn build_http_client() -> reqwest::Client {
  reqwest::Client::builder()
    .timeout(Duration::from_secs(600))
    .build()
    .unwrap_or_else(|e| {
      tracing::warn!(error = %e, "xai: falling back to default HTTP client");
      reqwest::Client::new()
    })
}

impl Provider for XaiProvider {
  fn kind(&self) -> ProviderKind {
    ProviderKind::Xai
  }

  fn description(&self) -> &str {
    "xAI Grok CLI (device-code OAuth)"
  }

  fn command(&self) -> &str {
    "grok"
  }

  fn list_models_args(&self) -> Vec<String> {
    vec!["models".into()]
  }

  fn spawn_spec(&self, req: &PromptRequest) -> SpawnSpec {
    SpawnSpec {
      program: "grok".into(),
      args: expand_args(&["{prompt}"], req),
      stdin: None,
      cwd: None,
      env: HashMap::new(),
      timeout_secs: self.timeout_secs(),
      stdout_codec: StdoutCodec::Raw,
    }
  }

  fn auth_method(&self) -> AuthMethod {
    AuthMethod::DeviceCode
  }

  fn load_auth(&self, store: &AuthStore) -> AppResult<Vec<AuthEntry>> {
    self.load_all(store)
  }

  fn save_auth(&self, store: &AuthStore, entry: &AuthEntry) -> AppResult<PathBuf> {
    self.save(store, entry)
  }

  async fn login(&self, store: &AuthStore, opts: LoginOptions) -> AppResult<AuthEntry> {
    self.login_device(store, opts).await
  }

  async fn refresh_auth(&self, store: &AuthStore, entry: &AuthEntry) -> AppResult<AuthEntry> {
    self.refresh(store, entry).await
  }
}
