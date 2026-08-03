//! Anthropic Claude Code CLI (OAuth)

mod auth;
mod callback;
mod compact;
mod execute;
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
pub use models::{ClaudeModel, ClaudeModelsResponse};

/// Anthropic Claude Code CLI (OAuth).
///
/// In-memory session is the native [`StoredAuth`].
#[derive(Debug, Clone)]
pub struct ClaudeProvider {
  /// Reused HTTP client for OAuth token exchange / refresh.
  http: reqwest::Client,
  session: Arc<RwLock<Option<StoredAuth>>>,
}

impl ClaudeProvider {
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
      AppError::Unauthorized("claude: no session in memory; restart server after auth login".into())
    })
  }

  pub async fn session_needs_refresh(&self) -> bool {
    match self.session.read().await.as_ref() {
      None => true,
      Some(s) => s.session_needs_refresh(chrono::Duration::minutes(5)),
    }
  }
}

impl Default for ClaudeProvider {
  fn default() -> Self {
    Self::new()
  }
}

fn build_http_client() -> reqwest::Client {
  reqwest::Client::builder()
    .timeout(Duration::from_secs(600))
    .build()
    .unwrap_or_else(|e| {
      tracing::warn!(error = %e, "claude: falling back to default HTTP client");
      reqwest::Client::new()
    })
}

impl Provider for ClaudeProvider {
  fn kind(&self) -> ProviderKind {
    ProviderKind::Claude
  }

  fn description(&self) -> &str {
    "Anthropic Claude Code CLI (OAuth)"
  }

  fn command(&self) -> &str {
    "claude"
  }

  fn list_models_args(&self) -> Vec<String> {
    vec!["models".into()]
  }

  fn spawn_spec(&self, req: &PromptRequest) -> SpawnSpec {
    SpawnSpec {
      program: "claude".into(),
      args: expand_args(
        &[
          "-p",
          "--output-format",
          "stream-json",
          "--verbose",
          "--include-partial-messages",
          "{prompt}",
        ],
        req,
      ),
      stdin: None,
      cwd: None,
      env: HashMap::new(),
      timeout_secs: self.timeout_secs(),
      stdout_codec: StdoutCodec::ClaudeStreamJson,
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
