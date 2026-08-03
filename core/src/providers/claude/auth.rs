//! Claude OAuth (browser + PKCE); credentials in `auth/claude.json` as native [`StoredAuth`].

use std::path::PathBuf;
use std::time::Duration;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::auth::{AuthStore, LoginOptions, generate_pkce, generate_state, open_url};
use crate::error::{AppError, AppResult};
use crate::providers::{AuthEntry, ProviderKind};

use super::ClaudeProvider;

const AUTH_URL: &str = "https://claude.ai/oauth/authorize";
const TOKEN_URL: &str = "https://platform.claude.com/v1/oauth/token";
const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const REDIRECT_URI: &str = "http://localhost:54545/callback";
const CALLBACK_PORT: u16 = 54545;
const CALLBACK_PATH: &str = "/callback";
const SCOPE: &str =
  "user:profile user:inference user:sessions:claude_code user:mcp_servers user:file_upload";

/// Claude auth record (serialized as-is into `auth/claude.json`).
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
  pub account_uuid: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub organization_uuid: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub organization_name: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub expired: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub last_refresh: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub redirect_uri: Option<String>,
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub device_ids: Vec<String>,
}

fn default_auth_kind() -> String {
  "oauth".into()
}

impl StoredAuth {
  pub fn account_key(&self) -> String {
    self
      .email
      .as_deref()
      .or(self.account_uuid.as_deref())
      .unwrap_or("default")
      .to_string()
  }

  pub fn require_access_token(&self) -> AppResult<&str> {
    self
      .access_token
      .as_deref()
      .filter(|s| !s.is_empty())
      .ok_or_else(|| AppError::Unauthorized("claude: missing access_token".into()))
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
    if let Some(acc) = &token.account {
      if let Some(email) = acc.email_address.as_ref().filter(|s| !s.is_empty()) {
        self.email = Some(email.clone());
      }
      if let Some(uuid) = acc.uuid.as_ref().filter(|s| !s.is_empty()) {
        self.account_uuid = Some(uuid.clone());
      }
    }
    if let Some(org) = &token.organization {
      if let Some(uuid) = org.uuid.as_ref().filter(|s| !s.is_empty()) {
        self.organization_uuid = Some(uuid.clone());
      }
      if let Some(name) = org.name.as_ref().filter(|s| !s.is_empty()) {
        self.organization_name = Some(name.clone());
      }
    }
    if let Some(secs) = token.expires_in {
      self.set_expiry_from_secs(secs);
    } else {
      self.expired = Some((Utc::now() + chrono::Duration::hours(8)).to_rfc3339());
    }
  }
}

#[derive(Debug, Serialize)]
struct ClaudeRefreshBody {
  grant_type: &'static str,
  refresh_token: String,
  client_id: &'static str,
}

#[derive(Debug, Serialize)]
struct ClaudeCodeExchangeBody {
  grant_type: &'static str,
  code: String,
  redirect_uri: String,
  client_id: &'static str,
  code_verifier: String,
  state: String,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
  access_token: String,
  #[serde(default)]
  refresh_token: Option<String>,
  #[serde(default)]
  expires_in: Option<i64>,
  #[serde(default)]
  organization: Option<Org>,
  #[serde(default)]
  account: Option<Account>,
}

#[derive(Debug, Deserialize)]
struct Org {
  #[serde(default)]
  uuid: Option<String>,
  #[serde(default)]
  name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Account {
  #[serde(default)]
  uuid: Option<String>,
  #[serde(default)]
  email_address: Option<String>,
}

impl ClaudeProvider {
  pub(super) fn load_all(&self, store: &AuthStore) -> AppResult<Vec<AuthEntry>> {
    let mut out = Vec::new();
    for (_account, stored) in store.load_all::<StoredAuth>(ProviderKind::Claude)? {
      out.push(AuthEntry::Claude(stored));
    }
    Ok(out)
  }

  pub(super) fn save(&self, store: &AuthStore, entry: &AuthEntry) -> AppResult<PathBuf> {
    let stored = entry.as_claude()?;
    store.save_account(ProviderKind::Claude, &stored.account_key(), stored)
  }

  fn load_stored(&self, store: &AuthStore, entry: &AuthEntry) -> AppResult<StoredAuth> {
    store.load_account(ProviderKind::Claude, &entry.account_key())
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
        q.append_pair("code", "true");
        q.append_pair("client_id", CLIENT_ID);
        q.append_pair("response_type", "code");
        q.append_pair("redirect_uri", &redirect_uri);
        q.append_pair("scope", SCOPE);
        q.append_pair("code_challenge", &pkce.code_challenge);
        q.append_pair("code_challenge_method", "S256");
        q.append_pair("state", &state);
      }
      url.to_string()
    };

    info!("starting Claude OAuth login");
    if opts.no_browser || !open_url(&auth_url) {
      println!("Visit the following URL to continue authentication:\n{auth_url}\n");
    } else {
      println!("Opening browser for Claude authentication…");
      println!("If the browser does not open, visit:\n{auth_url}\n");
    }
    println!("Waiting for OAuth callback on http://localhost:{port}{CALLBACK_PATH} …");

    let cb = self
      .wait_for_callback(
        port,
        CALLBACK_PATH,
        // State may arrive as a `code#state` fragment; we normalize below.
        None,
        Duration::from_secs(5 * 60),
      )
      .await?;

    // Claude sometimes returns `authorization_code#oauth_state` in the code param.
    let (code, state_from_code) = split_code_and_state(&cb.code);
    let returned_state = if !state_from_code.is_empty() {
      state_from_code
    } else {
      cb.state
    };
    if returned_state != state {
      return Err(AppError::Unauthorized(
        "claude: OAuth state mismatch (possible CSRF)".into(),
      ));
    }

    let mut stored = self
      .exchange_code(&code, &returned_state, &redirect_uri, &pkce.code_verifier)
      .await?;
    stored.auth_kind = default_auth_kind();
    stored.redirect_uri = Some(redirect_uri);
    stored.device_ids = vec![uuid::Uuid::new_v4().to_string()];
    stored.touch_refresh();
    if stored.email.is_none() {
      warn!("claude: token response missing email");
    }
    store.save_account(ProviderKind::Claude, &stored.account_key(), &stored)?;
    Ok(AuthEntry::Claude(stored))
  }

  pub(super) async fn refresh(&self, store: &AuthStore, entry: &AuthEntry) -> AppResult<AuthEntry> {
    let mut stored = self.load_stored(store, entry)?;
    let refresh = stored
      .refresh_token
      .as_deref()
      .filter(|s| !s.is_empty())
      .ok_or_else(|| AppError::Unauthorized("claude: missing refresh_token".into()))?
      .to_string();

    let body = ClaudeRefreshBody {
      grant_type: "refresh_token",
      refresh_token: refresh,
      client_id: CLIENT_ID,
    };
    let (status, text) = self.post_oauth_json(&body).await?;
    if !status.is_success() {
      return Err(AppError::Unauthorized(format!(
        "claude token refresh failed ({status}): {text}"
      )));
    }
    let token: TokenResponse = serde_json::from_str(&text)
      .map_err(|e| AppError::Internal(format!("claude token refresh parse: {e}")))?;

    stored.apply_token_response(&token);
    stored.touch_refresh();
    store.save_account(ProviderKind::Claude, &stored.account_key(), &stored)?;
    Ok(AuthEntry::Claude(stored))
  }

  async fn exchange_code(
    &self,
    code: &str,
    state: &str,
    redirect_uri: &str,
    code_verifier: &str,
  ) -> AppResult<StoredAuth> {
    let body = ClaudeCodeExchangeBody {
      grant_type: "authorization_code",
      code: code.to_string(),
      redirect_uri: redirect_uri.to_string(),
      client_id: CLIENT_ID,
      code_verifier: code_verifier.to_string(),
      state: state.to_string(),
    };
    let (status, text) = self.post_oauth_json(&body).await?;
    if !status.is_success() {
      return Err(AppError::Unauthorized(format!(
        "claude token exchange failed ({status}): {text}"
      )));
    }
    let token: TokenResponse = serde_json::from_str(&text)
      .map_err(|e| AppError::Internal(format!("claude token exchange parse: {e}")))?;

    let mut stored = StoredAuth {
      auth_kind: default_auth_kind(),
      email: None,
      access_token: None,
      refresh_token: None,
      account_uuid: None,
      organization_uuid: None,
      organization_name: None,
      expired: None,
      last_refresh: None,
      redirect_uri: None,
      device_ids: Vec::new(),
    };
    stored.apply_token_response(&token);
    Ok(stored)
  }

  /// Claude OAuth control-plane POST (axios-shaped headers).
  async fn post_oauth_json<T: Serialize>(
    &self,
    body: &T,
  ) -> AppResult<(reqwest::StatusCode, String)> {
    let resp = self
      .http
      .post(TOKEN_URL)
      .header(reqwest::header::ACCEPT, "application/json, text/plain, */*")
      .header(reqwest::header::CONTENT_TYPE, "application/json")
      .header(reqwest::header::USER_AGENT, "axios/1.15.2")
      .header(
        reqwest::header::ACCEPT_ENCODING,
        "gzip, compress, deflate, br",
      )
      .header(reqwest::header::CONNECTION, "close")
      .json(body)
      .send()
      .await
      .map_err(|e| AppError::Internal(format!("claude oauth request: {e}")))?;
    let status = resp.status();
    let text = resp
      .text()
      .await
      .map_err(|e| AppError::Internal(format!("claude oauth body: {e}")))?;
    Ok((status, text))
  }
}

/// Split Claude's `code#state` callback form into code and optional state.
fn split_code_and_state(code: &str) -> (String, String) {
  if let Some((c, s)) = code.split_once('#') {
    (c.to_string(), s.to_string())
  } else {
    (code.to_string(), String::new())
  }
}
