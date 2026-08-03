//! Vertex service-account import; credentials in `auth/vertex.json` as native [`StoredAuth`].

use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::auth::{AuthStore, LoginOptions};
use crate::error::{AppError, AppResult};
use crate::providers::{AuthEntry, ProviderKind};

use super::VertexProvider;

/// Vertex-only import options (not shared LoginOptions).
#[derive(Debug, Clone)]
pub struct ImportOptions {
  pub credential_path: PathBuf,
  pub location: Option<String>,
  pub prefix: Option<String>,
}

/// Google service-account key fields used for token minting.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceAccount {
  pub client_email: String,
  pub private_key: String,
  #[serde(default)]
  pub project_id: Option<String>,
  #[serde(default)]
  pub token_uri: Option<String>,
  #[serde(default)]
  pub private_key_id: Option<String>,
  #[serde(default)]
  pub client_id: Option<String>,
  #[serde(default)]
  pub auth_uri: Option<String>,
  #[serde(default)]
  pub auth_provider_x509_cert_url: Option<String>,
  #[serde(default)]
  pub client_x509_cert_url: Option<String>,
  #[serde(default)]
  pub universe_domain: Option<String>,
  #[serde(default, rename = "type")]
  pub account_type: Option<String>,
}

/// Vertex auth record (serialized as-is into `auth/vertex.json`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredAuth {
  #[serde(default = "default_auth_kind")]
  pub auth_kind: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub email: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub project_id: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub location: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub prefix: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub last_refresh: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub service_account: Option<ServiceAccount>,
}

fn default_auth_kind() -> String {
  "service_account".into()
}

impl StoredAuth {
  pub fn account_key(&self) -> String {
    self
      .email
      .as_deref()
      .or(self.project_id.as_deref())
      .unwrap_or("default")
      .to_string()
  }

  fn touch_refresh(&mut self) {
    self.last_refresh = Some(Utc::now().to_rfc3339());
  }
}

impl VertexProvider {
  pub(super) fn load_all(&self, store: &AuthStore) -> AppResult<Vec<AuthEntry>> {
    let mut out = Vec::new();
    for (_account, stored) in store.load_all::<StoredAuth>(ProviderKind::Vertex)? {
      out.push(AuthEntry::Vertex(stored));
    }
    Ok(out)
  }

  pub(super) fn save(&self, store: &AuthStore, entry: &AuthEntry) -> AppResult<PathBuf> {
    let stored = entry.as_vertex()?;
    store.save_account(ProviderKind::Vertex, &stored.account_key(), stored)
  }

  fn load_stored(&self, store: &AuthStore, entry: &AuthEntry) -> AppResult<StoredAuth> {
    store.load_account(ProviderKind::Vertex, &entry.account_key())
  }

  pub(super) async fn import(
    &self,
    store: &AuthStore,
    opts: ImportOptions,
  ) -> AppResult<AuthEntry> {
    let path = &opts.credential_path;
    if !path.is_file() {
      return Err(AppError::BadRequest(format!(
        "vertex credential file not found: {}",
        path.display()
      )));
    }

    let raw = fs::read_to_string(path)?;
    let sa: ServiceAccount = serde_json::from_str(&raw)
      .map_err(|e| AppError::BadRequest(format!("invalid service account JSON: {e}")))?;

    if sa.client_email.trim().is_empty() {
      return Err(AppError::BadRequest(
        "service account missing client_email".into(),
      ));
    }
    if sa.private_key.trim().is_empty() {
      return Err(AppError::BadRequest(
        "service account missing private_key".into(),
      ));
    }
    let project_id = sa
      .project_id
      .as_deref()
      .map(str::trim)
      .filter(|s| !s.is_empty())
      .ok_or_else(|| AppError::BadRequest("service account missing project_id".into()))?
      .to_string();

    let email = {
      let e = sa.client_email.trim();
      if e.is_empty() {
        None
      } else {
        Some(e.to_string())
      }
    };

    let mut stored = StoredAuth {
      auth_kind: default_auth_kind(),
      email,
      project_id: Some(project_id),
      location: opts.location.or_else(|| Some("us-central1".into())),
      prefix: opts.prefix,
      last_refresh: None,
      service_account: Some(sa),
    };
    stored.touch_refresh();
    store.save_account(ProviderKind::Vertex, &stored.account_key(), &stored)?;
    info!(path = %path.display(), "imported Vertex service account");
    Ok(AuthEntry::Vertex(stored))
  }

  pub(super) async fn login_import(
    &self,
    _store: &AuthStore,
    _opts: LoginOptions,
  ) -> AppResult<AuthEntry> {
    Err(AppError::BadRequest(
      "vertex login requires import options; use `teapotx auth login vertex -c <sa.json>` \
       (calls import_service_account)"
        .into(),
    ))
  }

  pub(super) async fn refresh(&self, store: &AuthStore, entry: &AuthEntry) -> AppResult<AuthEntry> {
    let stored = self.load_stored(store, entry)?;
    if stored.service_account.is_none() {
      return Err(AppError::Unauthorized(
        "vertex: service_account payload missing".into(),
      ));
    }
    Ok(AuthEntry::Vertex(stored))
  }

  pub async fn import_service_account(
    &self,
    store: &AuthStore,
    path: &Path,
    location: Option<String>,
    prefix: Option<String>,
  ) -> AppResult<AuthEntry> {
    self
      .import(
        store,
        ImportOptions {
          credential_path: path.to_path_buf(),
          location,
          prefix,
        },
      )
      .await
  }
}
