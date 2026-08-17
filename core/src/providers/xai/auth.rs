//! xAI device-code OAuth; credentials in `auth/xai.json` as native [`StoredAuth`].

use std::path::PathBuf;
use std::time::{Duration, Instant};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::auth::{AuthStore, LoginOptions, open_url};
use crate::error::{AppError, AppResult};
use crate::providers::{AuthEntry, ProviderKind};

use super::XaiProvider;
use super::jwt::parse_id_token;

const DISCOVERY_URL: &str = "https://auth.x.ai/.well-known/openid-configuration";
const CLIENT_ID: &str = "b1a00492-073a-47ea-816f-4c329264a828";
const SCOPE: &str = "openid profile email offline_access grok-cli:access api:access";
const DEVICE_CODE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";
const DEFAULT_API_BASE: &str = "https://api.x.ai/v1";
const DEFAULT_POLL: Duration = Duration::from_secs(5);
const MAX_POLL: Duration = Duration::from_secs(30 * 60);

/// xAI auth record (serialized as-is into `auth/xai.json`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredAuth {
  #[serde(default = "default_auth_kind")]
  pub auth_kind: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub email: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub subject: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub access_token: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub refresh_token: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub id_token: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub token_type: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub expired: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub last_refresh: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub base_url: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub token_endpoint: Option<String>,
}

fn default_auth_kind() -> String {
  "oauth".into()
}

impl StoredAuth {
  pub fn account_key(&self) -> String {
    self
      .email
      .as_deref()
      .or(self.subject.as_deref())
      .unwrap_or("default")
      .to_string()
  }

  pub fn require_access_token(&self) -> AppResult<&str> {
    self
      .access_token
      .as_deref()
      .filter(|s| !s.is_empty())
      .ok_or_else(|| AppError::Unauthorized("xai: missing access_token".into()))
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

  /// API base URL from stored auth, with default.
  pub fn api_base(&self) -> &str {
    self
      .base_url
      .as_deref()
      .map(str::trim)
      .filter(|s| !s.is_empty())
      .unwrap_or("https://api.x.ai/v1")
  }

  fn touch_refresh(&mut self) {
    self.last_refresh = Some(Utc::now().to_rfc3339());
  }

  fn set_expiry_from_secs(&mut self, expires_in: i64) {
    if expires_in > 0 {
      self.expired = Some((Utc::now() + chrono::Duration::seconds(expires_in)).to_rfc3339());
    }
  }

  fn apply_token(&mut self, token: &TokenPayload) {
    if let Some(at) = token.access_token.as_ref().filter(|s| !s.is_empty()) {
      self.access_token = Some(at.clone());
    }
    if let Some(rt) = token.refresh_token.as_ref().filter(|s| !s.is_empty()) {
      self.refresh_token = Some(rt.clone());
    }
    if let Some(tt) = token.token_type.as_ref().filter(|s| !s.is_empty()) {
      self.token_type = Some(tt.clone());
    }
    if let Some(id) = token.id_token.as_ref().filter(|s| !s.is_empty()) {
      self.id_token = Some(id.clone());
      match parse_id_token(id) {
        Ok(claims) => {
          if let Some(email) = claims.user_email() {
            self.email = Some(email);
          }
          if let Some(sub) = claims.subject() {
            self.subject = Some(sub);
          }
        }
        Err(e) => warn!(error = %e, "xai: failed to parse id_token"),
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
struct Discovery {
  device_authorization_endpoint: String,
  token_endpoint: String,
}

#[derive(Debug, Deserialize)]
struct DeviceCodeResponse {
  device_code: String,
  #[serde(default)]
  user_code: Option<String>,
  #[serde(default)]
  verification_uri: Option<String>,
  #[serde(default)]
  verification_uri_complete: Option<String>,
  #[serde(default)]
  expires_in: Option<u64>,
  #[serde(default)]
  interval: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct TokenPayload {
  #[serde(default)]
  error: Option<String>,
  #[serde(default)]
  error_description: Option<String>,
  #[serde(default)]
  access_token: Option<String>,
  #[serde(default)]
  refresh_token: Option<String>,
  #[serde(default)]
  id_token: Option<String>,
  #[serde(default)]
  token_type: Option<String>,
  #[serde(default)]
  expires_in: Option<i64>,
}

impl XaiProvider {
  pub(super) fn load_all(&self, store: &AuthStore) -> AppResult<Vec<AuthEntry>> {
    let mut out = Vec::new();
    for (_account, stored) in store.load_all::<StoredAuth>(ProviderKind::Xai)? {
      out.push(AuthEntry::Xai(stored));
    }
    Ok(out)
  }

  pub(super) fn save(&self, store: &AuthStore, entry: &AuthEntry) -> AppResult<PathBuf> {
    let stored = entry.as_xai()?;
    store.save_account(ProviderKind::Xai, &stored.account_key(), stored)
  }

  fn load_stored(&self, store: &AuthStore, entry: &AuthEntry) -> AppResult<StoredAuth> {
    store.load_account(ProviderKind::Xai, &entry.account_key())
  }

  /// Device-code OAuth login.
  pub(super) async fn login_device(
    &self,
    store: &AuthStore,
    opts: LoginOptions,
  ) -> AppResult<AuthEntry> {
    let discovery = self.discover().await?;
    let device = self
      .request_device_code(&discovery.device_authorization_endpoint)
      .await?;

    let verify = device
      .verification_uri_complete
      .as_deref()
      .or(device.verification_uri.as_deref())
      .unwrap_or("https://auth.x.ai");
    println!("xAI device login");
    if opts.no_browser || !open_url(verify) {
      println!("  Visit: {verify}");
      if let Some(code) = &device.user_code {
        println!("  Code:  {code}");
      }
    } else {
      println!("Opening browser for xAI authentication…");
      println!("If the browser does not open, visit:\n{verify}");
      if let Some(code) = &device.user_code {
        println!("  Code:  {code}");
      }
    }
    println!("Waiting for authorization…");

    let token = self
      .poll_for_token(
        &discovery.token_endpoint,
        &device.device_code,
        device.interval,
        device.expires_in,
      )
      .await?;

    let mut stored = StoredAuth {
      auth_kind: default_auth_kind(),
      email: None,
      subject: None,
      access_token: None,
      refresh_token: None,
      id_token: None,
      token_type: None,
      expired: None,
      last_refresh: None,
      base_url: Some(DEFAULT_API_BASE.into()),
      token_endpoint: Some(discovery.token_endpoint),
    };
    stored.apply_token(&token);
    stored.touch_refresh();
    store.save_account(ProviderKind::Xai, &stored.account_key(), &stored)?;
    info!(email = ?stored.email, "xAI login complete");
    Ok(AuthEntry::Xai(stored))
  }

  pub(super) async fn refresh(&self, store: &AuthStore, entry: &AuthEntry) -> AppResult<AuthEntry> {
    let mut stored = self.load_stored(store, entry)?;
    let refresh = stored
      .refresh_token
      .as_deref()
      .filter(|s| !s.is_empty())
      .ok_or_else(|| AppError::Unauthorized("xai: missing refresh_token".into()))?
      .to_string();

    let token_endpoint = match stored.token_endpoint.as_deref().filter(|s| !s.is_empty()) {
      Some(ep) => ep.to_string(),
      None => self.discover().await?.token_endpoint,
    };

    let (status, body) = self
      .post_form_urlencoded(
        &token_endpoint,
        &[
          ("grant_type", "refresh_token"),
          ("client_id", CLIENT_ID),
          ("refresh_token", &refresh),
        ],
      )
      .await?;
    if !status.is_success() {
      return Err(AppError::Unauthorized(format!(
        "xai token refresh failed ({status}): {body}"
      )));
    }
    let token: TokenPayload = serde_json::from_str(&body)
      .map_err(|e| AppError::Internal(format!("xai token refresh parse: {e}")))?;

    stored.apply_token(&token);
    stored.token_endpoint = Some(token_endpoint);
    stored.touch_refresh();
    store.save_account(ProviderKind::Xai, &stored.account_key(), &stored)?;
    Ok(AuthEntry::Xai(stored))
  }

  async fn discover(&self) -> AppResult<Discovery> {
    let resp = self
      .http
      .get(DISCOVERY_URL)
      .header(reqwest::header::ACCEPT, "application/json")
      .send()
      .await
      .map_err(|e| AppError::Internal(format!("xai discovery: {e}")))?;
    let status = resp.status();
    let body = resp
      .text()
      .await
      .map_err(|e| AppError::Internal(format!("xai discovery body: {e}")))?;
    if !status.is_success() {
      return Err(AppError::Internal(format!(
        "xai discovery failed ({status}): {body}"
      )));
    }
    serde_json::from_str(&body).map_err(|e| AppError::Internal(format!("xai discovery parse: {e}")))
  }

  async fn request_device_code(&self, endpoint: &str) -> AppResult<DeviceCodeResponse> {
    let (status, body) = self
      .post_form_urlencoded(endpoint, &[("client_id", CLIENT_ID), ("scope", SCOPE)])
      .await?;
    if !status.is_success() {
      return Err(AppError::Internal(format!(
        "xai device code failed ({status}): {body}"
      )));
    }
    let device: DeviceCodeResponse = serde_json::from_str(&body)
      .map_err(|e| AppError::Internal(format!("xai device code parse: {e}")))?;
    if device.device_code.trim().is_empty() {
      return Err(AppError::Internal(
        "xai device code response missing device_code".into(),
      ));
    }
    Ok(device)
  }

  async fn poll_for_token(
    &self,
    token_endpoint: &str,
    device_code: &str,
    interval_secs: Option<u64>,
    expires_in: Option<u64>,
  ) -> AppResult<TokenPayload> {
    let mut interval = interval_secs
      .map(Duration::from_secs)
      .unwrap_or(DEFAULT_POLL);
    if interval < DEFAULT_POLL {
      interval = DEFAULT_POLL;
    }
    let deadline = Instant::now()
      + expires_in
        .map(Duration::from_secs)
        .unwrap_or(MAX_POLL)
        .min(MAX_POLL);

    loop {
      if Instant::now() > deadline {
        return Err(AppError::Unauthorized("xai device code expired".into()));
      }

      let (status, body) = self
        .post_form_urlencoded(
          token_endpoint,
          &[
            ("grant_type", DEVICE_CODE_GRANT),
            ("device_code", device_code),
            ("client_id", CLIENT_ID),
          ],
        )
        .await?;
      let payload: TokenPayload = serde_json::from_str(&body).unwrap_or(TokenPayload {
        error: None,
        error_description: None,
        access_token: None,
        refresh_token: None,
        id_token: None,
        token_type: None,
        expires_in: None,
      });

      if let Some(err) = payload.error.as_deref() {
        match err {
          "authorization_pending" => {
            tokio::time::sleep(interval).await;
            continue;
          }
          "slow_down" => {
            interval += DEFAULT_POLL;
            tokio::time::sleep(interval).await;
            continue;
          }
          "expired_token" => {
            return Err(AppError::Unauthorized("xai device code expired".into()));
          }
          "access_denied" => {
            return Err(AppError::Unauthorized(
              "xai device authorization denied".into(),
            ));
          }
          other => {
            let desc = payload.error_description.unwrap_or_default();
            return Err(AppError::Unauthorized(format!(
              "xai device token error: {other} {desc}"
            )));
          }
        }
      }

      if !status.is_success() {
        return Err(AppError::Unauthorized(format!(
          "xai device token failed ({status}): {body}"
        )));
      }
      if payload
        .access_token
        .as_deref()
        .unwrap_or("")
        .trim()
        .is_empty()
      {
        return Err(AppError::Unauthorized(
          "xai device token response missing access_token".into(),
        ));
      }
      return Ok(payload);
    }
  }

  /// xAI OAuth form POST (`application/x-www-form-urlencoded` + JSON Accept).
  async fn post_form_urlencoded(
    &self,
    url: &str,
    fields: &[(&str, &str)],
  ) -> AppResult<(reqwest::StatusCode, String)> {
    let body = url::form_urlencoded::Serializer::new(String::new())
      .extend_pairs(fields.iter().copied())
      .finish();
    let resp = self
      .http
      .post(url)
      .header(
        reqwest::header::CONTENT_TYPE,
        "application/x-www-form-urlencoded",
      )
      .header(reqwest::header::ACCEPT, "application/json")
      .body(body)
      .send()
      .await
      .map_err(|e| AppError::Internal(format!("xai form post {url}: {e}")))?;
    let status = resp.status();
    let text = resp
      .text()
      .await
      .map_err(|e| AppError::Internal(format!("xai form body {url}: {e}")))?;
    Ok((status, text))
  }
}
