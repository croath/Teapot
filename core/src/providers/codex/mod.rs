//! OpenAI Codex provider (`codex`) — CLI spawn + OAuth auth.

mod auth;
mod callback;
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

use super::traits::{PromptRequest, Provider, SpawnSpec, StdoutCodec, stdin_prompt};

pub use auth::StoredAuth;
pub use models::{CodexModel, CodexModelsResponse};

/// OpenAI Codex CLI backend.
///
/// In-memory session is the native [`StoredAuth`] (not a shared credential bag).
#[derive(Debug, Clone)]
pub struct CodexProvider {
  /// Reused HTTP client for OAuth token exchange / refresh.
  http: reqwest::Client,
  /// Live credentials for this provider only.
  session: Arc<RwLock<Option<StoredAuth>>>,
}

impl CodexProvider {
  pub fn new() -> Self {
    Self {
      http: build_http_client(),
      session: Arc::new(RwLock::new(None)),
    }
  }

  /// Install native auth into memory (called after ensure/refresh).
  pub async fn set_session(&self, auth: StoredAuth) {
    let mut guard = self.session.write().await;
    *guard = Some(auth);
  }

  /// Snapshot of this provider's in-memory credentials.
  pub async fn session(&self) -> AppResult<StoredAuth> {
    self.session.read().await.clone().ok_or_else(|| {
      AppError::Unauthorized("codex: no session in memory; restart server after auth login".into())
    })
  }

  /// Whether the in-memory session is missing or near expiry.
  pub async fn session_needs_refresh(&self) -> bool {
    match self.session.read().await.as_ref() {
      None => true,
      Some(s) => s.session_needs_refresh(chrono::Duration::minutes(5)),
    }
  }
}

impl Default for CodexProvider {
  fn default() -> Self {
    Self::new()
  }
}

fn build_http_client() -> reqwest::Client {
  reqwest::Client::builder()
    .timeout(Duration::from_secs(600))
    .build()
    .unwrap_or_else(|e| {
      tracing::warn!(error = %e, "codex: falling back to default HTTP client");
      reqwest::Client::new()
    })
}

impl Provider for CodexProvider {
  fn kind(&self) -> ProviderKind {
    ProviderKind::Codex
  }

  fn description(&self) -> &str {
    "OpenAI Codex CLI (OAuth)"
  }

  fn command(&self) -> &str {
    "codex"
  }

  fn list_models_args(&self) -> Vec<String> {
    vec!["debug".into(), "models".into()]
  }

  fn spawn_spec(&self, req: &PromptRequest) -> SpawnSpec {
    SpawnSpec {
      program: "codex".into(),
      args: vec![
        "exec".into(),
        "--skip-git-repo-check".into(),
        "--json".into(),
        "-".into(),
      ],
      stdin: Some(stdin_prompt(req)),
      cwd: None,
      env: HashMap::new(),
      timeout_secs: self.timeout_secs(),
      stdout_codec: StdoutCodec::CodexJsonl,
    }
  }

  fn auth_method(&self) -> AuthMethod {
    AuthMethod::BrowserOAuth
  }

  fn load_auth(&self, store: &AuthStore) -> AppResult<Vec<AuthEntry>> {
    self.load_all(store)
  }

  fn save_auth(&self, store: &AuthStore, entry: &AuthEntry) -> AppResult<PathBuf> {
    self.save(store, entry)
  }

  async fn login(&self, store: &AuthStore, opts: LoginOptions) -> AppResult<AuthEntry> {
    self.login_oauth(store, opts).await
  }

  async fn refresh_auth(&self, store: &AuthStore, entry: &AuthEntry) -> AppResult<AuthEntry> {
    self.refresh(store, entry).await
  }
}
