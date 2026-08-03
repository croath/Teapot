//! Google Vertex AI (service-account import + HTTP execute)

mod auth;
mod compact;
mod execute;
mod models;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use tokio::sync::RwLock;

use crate::auth::{AuthMethod, AuthStore, LoginOptions};
use crate::error::{AppError, AppResult};
use crate::providers::{AuthEntry, ProviderKind};

use super::traits::{PromptRequest, Provider, SpawnSpec, StdoutCodec};

pub use auth::{ImportOptions, ServiceAccount, StoredAuth};
pub use models::{VertexModel, VertexModelsResponse};

/// In-memory Vertex session (minted access token + native project fields).
///
/// Not the same as [`StoredAuth`] (disk); includes a short-lived access token.
#[derive(Debug, Clone)]
pub struct VertexSession {
  pub access_token: String,
  pub expires_at: DateTime<Utc>,
  pub project_id: String,
  pub location: Option<String>,
  pub service_account: ServiceAccount,
}

impl VertexSession {
  pub fn require_access_token(&self) -> AppResult<&str> {
    if self.access_token.is_empty() {
      return Err(AppError::Unauthorized(
        "vertex: missing access_token".into(),
      ));
    }
    Ok(self.access_token.as_str())
  }

  pub fn session_needs_refresh(&self, lead: chrono::Duration) -> bool {
    if self.access_token.is_empty() {
      return true;
    }
    Utc::now() + lead >= self.expires_at
  }

  pub fn project_id(&self) -> &str {
    self.project_id.as_str()
  }

  pub fn location(&self) -> &str {
    self
      .location
      .as_deref()
      .map(str::trim)
      .filter(|s| !s.is_empty())
      .unwrap_or("us-central1")
  }
}

/// Google Vertex AI (service-account import + generateContent).
///
/// In-memory credentials are [`VertexSession`] (provider-owned).
#[derive(Debug, Clone)]
pub struct VertexProvider {
  /// HTTP client for token exchange and generateContent.
  http: reqwest::Client,
  session: Arc<RwLock<Option<VertexSession>>>,
}

impl VertexProvider {
  pub fn new() -> Self {
    Self {
      http: build_http_client(),
      session: Arc::new(RwLock::new(None)),
    }
  }

  pub async fn set_session(&self, session: VertexSession) {
    let mut guard = self.session.write().await;
    *guard = Some(session);
  }

  pub async fn session(&self) -> AppResult<VertexSession> {
    self.session.read().await.clone().ok_or_else(|| {
      AppError::Unauthorized("vertex: no session in memory; restart server after auth login".into())
    })
  }

  pub async fn session_needs_refresh(&self) -> bool {
    match self.session.read().await.as_ref() {
      None => true,
      Some(s) => s.session_needs_refresh(chrono::Duration::minutes(5)),
    }
  }

  /// Build a live session from stored auth (mints access token).
  pub async fn session_from_stored(&self, stored: &StoredAuth) -> AppResult<VertexSession> {
    let sa = stored
      .service_account
      .clone()
      .ok_or_else(|| AppError::Unauthorized("vertex: service_account payload missing".into()))?;
    let project_id = stored
      .project_id
      .clone()
      .filter(|s| !s.is_empty())
      .or_else(|| sa.project_id.clone())
      .filter(|s| !s.is_empty())
      .ok_or_else(|| AppError::Unauthorized("vertex: missing project_id".into()))?;
    let token = self.fetch_access_token(&sa).await?;
    Ok(VertexSession {
      access_token: token,
      expires_at: Utc::now() + chrono::Duration::minutes(50),
      project_id,
      location: stored.location.clone(),
      service_account: sa,
    })
  }
}

impl Default for VertexProvider {
  fn default() -> Self {
    Self::new()
  }
}

fn build_http_client() -> reqwest::Client {
  reqwest::Client::builder()
    .timeout(Duration::from_secs(600))
    .build()
    .unwrap_or_else(|e| {
      tracing::warn!(error = %e, "vertex: falling back to default HTTP client");
      reqwest::Client::new()
    })
}

impl Provider for VertexProvider {
  fn kind(&self) -> ProviderKind {
    ProviderKind::Vertex
  }

  fn description(&self) -> &str {
    "Google Vertex AI (service-account import)"
  }

  fn command(&self) -> &str {
    "gcloud"
  }

  fn is_installed(&self) -> bool {
    true
  }

  fn list_models_args(&self) -> Vec<String> {
    Vec::new()
  }

  fn spawn_spec(&self, req: &PromptRequest) -> SpawnSpec {
    SpawnSpec {
      program: "gcloud".into(),
      args: vec![
        "ai".into(),
        "models".into(),
        "list".into(),
        "--format=json".into(),
      ],
      stdin: None,
      cwd: None,
      env: {
        let mut e = HashMap::new();
        e.insert("TEAPOT_PROMPT".into(), req.prompt.clone());
        e
      },
      timeout_secs: self.timeout_secs(),
      stdout_codec: StdoutCodec::Raw,
    }
  }

  fn auth_method(&self) -> AuthMethod {
    AuthMethod::CredentialImport
  }

  fn load_auth(&self, store: &AuthStore) -> AppResult<Vec<AuthEntry>> {
    self.load_all(store)
  }

  fn save_auth(&self, store: &AuthStore, entry: &AuthEntry) -> AppResult<PathBuf> {
    self.save(store, entry)
  }

  async fn login(&self, store: &AuthStore, opts: LoginOptions) -> AppResult<AuthEntry> {
    self.login_import(store, opts).await
  }

  async fn refresh_auth(&self, store: &AuthStore, entry: &AuthEntry) -> AppResult<AuthEntry> {
    self.refresh(store, entry).await
  }
}

/// Import a Vertex service-account JSON (Vertex-owned options).
pub async fn import_service_account(
  store: &AuthStore,
  path: &Path,
  location: Option<String>,
  prefix: Option<String>,
) -> AppResult<AuthEntry> {
  VertexProvider::new()
    .import_service_account(store, path, location, prefix)
    .await
}
