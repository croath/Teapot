//! Codex OAuth (browser + PKCE); credentials in `auth/codex.json` as native [`StoredAuth`].

use std::path::PathBuf;
use std::time::Duration;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::auth::{AuthStore, LoginOptions, generate_pkce, generate_state, open_url};
use crate::error::{AppError, AppResult};
use crate::providers::{AuthEntry, ProviderKind};

use super::CodexProvider;
use super::jwt::parse_id_token;

const AUTH_URL: &str = "https://auth.openai.com/oauth/authorize";
const TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const REDIRECT_URI: &str = "http://localhost:1455/auth/callback";
const CALLBACK_PORT: u16 = 1455;
const CALLBACK_PATH: &str = "/auth/callback";

// ---------------------------------------------------------------------------
// Provider-owned file: `auth/codex.json` → `{ "<account>": { ...StoredAuth } }`
// ---------------------------------------------------------------------------

/// Codex auth record (serialized as-is into `auth/codex.json`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredAuth {
  #[serde(default = "default_auth_kind")]
  pub auth_kind: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub email: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub access_token: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub refresh_token: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub id_token: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub account_id: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub expired: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub last_refresh: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub redirect_uri: Option<String>,
}

fn default_auth_kind() -> String {
  "oauth".into()
}

impl StoredAuth {
  pub fn account_key(&self) -> String {
    self
      .email
      .as_deref()
      .or(self.account_id.as_deref())
      .unwrap_or("default")
      .to_string()
  }

  /// Access token required for upstream HTTP (native field, not a shared bag).
  pub fn require_access_token(&self) -> AppResult<&str> {
    self
      .access_token
      .as_deref()
      .filter(|s| !s.is_empty())
      .ok_or_else(|| AppError::Unauthorized("codex: missing access_token".into()))
  }

  pub fn token_expires_at(&self) -> Option<chrono::DateTime<Utc>> {
    self
      .expired
      .as_deref()
      .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
      .map(|dt| dt.with_timezone(&Utc))
  }

  pub fn session_needs_refresh(&self, lead: chrono::Duration) -> bool {
    if self.access_token.as_deref().unwrap_or("").is_empty() {
      return true;
    }
    match self.token_expires_at() {
      Some(exp) => Utc::now() + lead >= exp,
      None => false,
    }
  }

  fn touch_refresh(&mut self) {
    self.last_refresh = Some(Utc::now().to_rfc3339());
  }

  fn set_expiry_from_secs(&mut self, expires_in: i64) {
    if expires_in > 0 {
      self.expired = Some((Utc::now() + chrono::Duration::seconds(expires_in)).to_rfc3339());
    }
  }

  fn apply_token_response(&mut self, token: &TokenResponse) {
    self.access_token = Some(token.access_token.clone());
    if let Some(r) = token.refresh_token.as_ref().filter(|s| !s.is_empty()) {
      self.refresh_token = Some(r.clone());
    }
    if let Some(id) = token.id_token.as_ref().filter(|s| !s.is_empty()) {
      self.id_token = Some(id.clone());
      match parse_id_token(id) {
        Ok(claims) => {
          if let Some(email) = claims.user_email() {
            self.email = Some(email);
          }
          if let Some(aid) = claims.account_id() {
            self.account_id = Some(aid);
          }
        }
        Err(e) => warn!(error = %e, "codex: failed to parse id_token"),
      }
    }
    if let Some(secs) = token.expires_in {
      self.set_expiry_from_secs(secs);
    } else {
      self.expired = Some((Utc::now() + chrono::Duration::hours(1)).to_rfc3339());
    }
  }
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
  access_token: String,
  #[serde(default)]
  refresh_token: Option<String>,
  #[serde(default)]
  id_token: Option<String>,
  #[serde(default)]
  expires_in: Option<i64>,
}

impl CodexProvider {
  /// Load all Codex accounts from `auth/codex.json` as native [`StoredAuth`].
  pub(super) fn load_all(&self, store: &AuthStore) -> AppResult<Vec<AuthEntry>> {
    let mut out = Vec::new();
    for (_account, stored) in store.load_all::<StoredAuth>(ProviderKind::Codex)? {
      out.push(AuthEntry::Codex(stored));
    }
    Ok(out)
  }

  /// Persist a Codex entry (must be [`AuthEntry::Codex`]).
  pub(super) fn save(&self, store: &AuthStore, entry: &AuthEntry) -> AppResult<PathBuf> {
    let stored = entry.as_codex()?;
    store.save_account(ProviderKind::Codex, &stored.account_key(), stored)
  }

  fn load_stored(&self, store: &AuthStore, entry: &AuthEntry) -> AppResult<StoredAuth> {
    store.load_account(ProviderKind::Codex, &entry.account_key())
  }

  /// Browser OAuth login with PKCE.
  pub(super) async fn login_oauth(
    &self,
    store: &AuthStore,
    opts: LoginOptions,
  ) -> AppResult<AuthEntry> {
    let pkce = generate_pkce()?;
    let state = generate_state()?;
    let port = opts.callback_port.unwrap_or(CALLBACK_PORT);
    let redirect_uri = if port == CALLBACK_PORT {
      REDIRECT_URI.to_string()
    } else {
      format!("http://localhost:{port}{CALLBACK_PATH}")
    };

    let auth_url = {
      let mut url = url::Url::parse(AUTH_URL)
        .map_err(|e| AppError::Internal(format!("parse auth url: {e}")))?;
      {
        let mut q = url.query_pairs_mut();
        q.append_pair("client_id", CLIENT_ID);
        q.append_pair("response_type", "code");
        q.append_pair("redirect_uri", &redirect_uri);
        q.append_pair("scope", "openid email profile offline_access");
        q.append_pair("state", &state);
        q.append_pair("code_challenge", &pkce.code_challenge);
        q.append_pair("code_challenge_method", "S256");
        q.append_pair("prompt", "login");
        q.append_pair("id_token_add_organizations", "true");
        q.append_pair("codex_cli_simplified_flow", "true");
      }
      url.to_string()
    };

    info!("starting Codex OAuth login");
    if opts.no_browser || !open_url(&auth_url) {
      println!("Visit the following URL to continue authentication:\n{auth_url}\n");
    } else {
      println!("Opening browser for Codex authentication…");
      println!("If the browser does not open, visit:\n{auth_url}\n");
    }
    println!("Waiting for OAuth callback on http://localhost:{port}{CALLBACK_PATH} …");

    let cb = self
      .wait_for_callback(
        port,
        CALLBACK_PATH,
        Some(&state),
        Duration::from_secs(5 * 60),
      )
      .await?;

    let mut stored = self
      .exchange_code(&cb.code, &redirect_uri, &pkce.code_verifier)
      .await?;
    stored.auth_kind = default_auth_kind();
    stored.redirect_uri = Some(redirect_uri);
    stored.touch_refresh();
    store.save_account(ProviderKind::Codex, &stored.account_key(), &stored)?;
    Ok(AuthEntry::Codex(stored))
  }

  /// Refresh access token using the stored refresh_token.
  pub(super) async fn refresh(&self, store: &AuthStore, entry: &AuthEntry) -> AppResult<AuthEntry> {
    let mut stored = self.load_stored(store, entry)?;
    let refresh = stored
      .refresh_token
      .as_deref()
      .filter(|s| !s.is_empty())
      .ok_or_else(|| AppError::Unauthorized("codex: missing refresh_token".into()))?
      .to_string();

    let (status, body) = self
      .post_token_form(&[
        ("client_id", CLIENT_ID),
        ("grant_type", "refresh_token"),
        ("refresh_token", &refresh),
        ("scope", "openid profile email"),
      ])
      .await?;
    if !status.is_success() {
      return Err(AppError::Unauthorized(format!(
        "codex token refresh failed ({status}): {body}"
      )));
    }

    let token: TokenResponse = serde_json::from_str(&body)
      .map_err(|e| AppError::Internal(format!("codex token refresh parse: {e}")))?;

    stored.apply_token_response(&token);
    stored.touch_refresh();
    store.save_account(ProviderKind::Codex, &stored.account_key(), &stored)?;
    Ok(AuthEntry::Codex(stored))
  }

  async fn exchange_code(
    &self,
    code: &str,
    redirect_uri: &str,
    code_verifier: &str,
  ) -> AppResult<StoredAuth> {
    let (status, body) = self
      .post_token_form(&[
        ("grant_type", "authorization_code"),
        ("client_id", CLIENT_ID),
        ("code", code),
        ("redirect_uri", redirect_uri),
        ("code_verifier", code_verifier),
      ])
      .await?;
    if !status.is_success() {
      return Err(AppError::Unauthorized(format!(
        "codex token exchange failed ({status}): {body}"
      )));
    }
    let token: TokenResponse = serde_json::from_str(&body)
      .map_err(|e| AppError::Internal(format!("codex token exchange parse: {e}")))?;

    let mut stored = StoredAuth {
      auth_kind: default_auth_kind(),
      email: None,
      access_token: None,
      refresh_token: None,
      id_token: None,
      account_id: None,
      expired: None,
      last_refresh: None,
      redirect_uri: None,
    };
    stored.apply_token_response(&token);
    Ok(stored)
  }

  /// Codex token endpoint POST (`application/x-www-form-urlencoded`).
  async fn post_token_form(
    &self,
    fields: &[(&str, &str)],
  ) -> AppResult<(reqwest::StatusCode, String)> {
    let body = url::form_urlencoded::Serializer::new(String::new())
      .extend_pairs(fields.iter().copied())
      .finish();
    let resp = self
      .http
      .post(TOKEN_URL)
      .header(
        reqwest::header::CONTENT_TYPE,
        "application/x-www-form-urlencoded",
      )
      .header(reqwest::header::ACCEPT, "application/json")
      .body(body)
      .send()
      .await
      .map_err(|e| AppError::Internal(format!("codex token request: {e}")))?;
    let status = resp.status();
    let text = resp
      .text()
      .await
      .map_err(|e| AppError::Internal(format!("codex token body: {e}")))?;
    Ok((status, text))
  }
}
