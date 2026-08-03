//! Antigravity Google OAuth + project onboarding; credentials in `auth/antigravity.json`.

use std::path::PathBuf;
use std::time::Duration;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::auth::{AuthStore, LoginOptions, generate_state, open_url};
use crate::error::{AppError, AppResult};
use crate::providers::{AuthEntry, ProviderKind};

use super::AntigravityProvider;

const CLIENT_ID: &str = env!("ANTIGRAVITY_CLIENT_ID");
const CLIENT_SECRET: &str = env!("ANTIGRAVITY_CLIENT_SECRET");
const CALLBACK_PORT: u16 = 51121;
const CALLBACK_PATH: &str = "/oauth-callback";
const TOKEN_ENDPOINT: &str = "https://oauth2.googleapis.com/token";
const AUTH_ENDPOINT: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const USERINFO_ENDPOINT: &str = "https://www.googleapis.com/oauth2/v2/userinfo?alt=json";
const API_ENDPOINT: &str = "https://cloudcode-pa.googleapis.com";
const DAILY_API_ENDPOINT: &str = "https://daily-cloudcode-pa.googleapis.com";
const API_VERSION: &str = "v1internal";

const SCOPES: &str = "https://www.googleapis.com/auth/cloud-platform \
https://www.googleapis.com/auth/userinfo.email \
https://www.googleapis.com/auth/userinfo.profile \
https://www.googleapis.com/auth/cclog \
https://www.googleapis.com/auth/experimentsandconfigs";

/// Short runtime UA used by userinfo / loadCodeAssist.
const REQUEST_USER_AGENT: &str = "antigravity/hub/2.2.1 darwin/arm64";
/// Long control-plane UA used by onboardUser.
const ONBOARD_USER_AGENT: &str =
  "antigravity/hub/2.2.1 darwin/arm64 google-api-nodejs-client/10.3.0";
const GOOG_API_CLIENT: &str = "gl-node/22.21.1";

/// Antigravity auth record (serialized as-is into `auth/antigravity.json`).
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
  pub token_type: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub project_id: Option<String>,
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
    self.email.as_deref().unwrap_or("default").to_string()
  }

  pub fn require_access_token(&self) -> AppResult<&str> {
    self
      .access_token
      .as_deref()
      .filter(|s| !s.is_empty())
      .ok_or_else(|| AppError::Unauthorized("antigravity: missing access_token".into()))
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
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
  access_token: String,
  #[serde(default)]
  refresh_token: String,
  #[serde(default)]
  expires_in: i64,
  #[serde(default)]
  token_type: String,
}

#[derive(Debug, Deserialize)]
struct UserInfo {
  #[serde(default)]
  email: String,
}

impl AntigravityProvider {
  pub(super) fn load_all(&self, store: &AuthStore) -> AppResult<Vec<AuthEntry>> {
    let mut out = Vec::new();
    for (_account, stored) in store.load_all::<StoredAuth>(ProviderKind::Antigravity)? {
      out.push(AuthEntry::Antigravity(stored));
    }
    Ok(out)
  }

  pub(super) fn save(&self, store: &AuthStore, entry: &AuthEntry) -> AppResult<PathBuf> {
    let stored = entry.as_antigravity()?;
    store.save_account(ProviderKind::Antigravity, &stored.account_key(), stored)
  }

  fn load_stored(&self, store: &AuthStore, entry: &AuthEntry) -> AppResult<StoredAuth> {
    store.load_account(ProviderKind::Antigravity, &entry.account_key())
  }

  /// Browser OAuth login + project onboarding.
  pub(super) async fn login_oauth(
    &self,
    store: &AuthStore,
    opts: LoginOptions,
  ) -> AppResult<AuthEntry> {
    let state = generate_state()?;
    let port = opts.callback_port.unwrap_or(CALLBACK_PORT);
    let redirect_uri = format!("http://localhost:{port}{CALLBACK_PATH}");

    let auth_url = {
      let mut url = url::Url::parse(AUTH_ENDPOINT)
        .map_err(|e| AppError::Internal(format!("parse auth url: {e}")))?;
      {
        let mut q = url.query_pairs_mut();
        q.append_pair("access_type", "offline");
        q.append_pair("client_id", CLIENT_ID);
        q.append_pair("prompt", "consent");
        q.append_pair("redirect_uri", &redirect_uri);
        q.append_pair("response_type", "code");
        q.append_pair("scope", SCOPES);
        q.append_pair("state", &state);
      }
      url.to_string()
    };

    info!("starting Antigravity OAuth login");
    if opts.no_browser || !open_url(&auth_url) {
      println!("Visit the following URL to continue authentication:\n{auth_url}\n");
    } else {
      println!("Opening browser for Antigravity authentication…");
      println!("If the browser does not open, visit:\n{auth_url}\n");
    }
    println!("Waiting for OAuth callback on {redirect_uri} …");

    let cb = self
      .wait_for_callback(
        port,
        CALLBACK_PATH,
        Some(&state),
        Duration::from_secs(5 * 60),
      )
      .await?;

    let token = self.exchange_code(&cb.code, &redirect_uri).await?;
    let email = self
      .fetch_user_email(&token.access_token)
      .await
      .unwrap_or_else(|e| {
        warn!(error = %e, "antigravity: userinfo failed");
        String::new()
      });
    let project_id = match self.fetch_project_id(&token.access_token).await {
      Ok(p) => Some(p),
      Err(e) => {
        warn!(error = %e, "antigravity: project id lookup failed");
        None
      }
    };

    let mut stored = StoredAuth {
      auth_kind: default_auth_kind(),
      email: if email.is_empty() { None } else { Some(email) },
      access_token: Some(token.access_token),
      refresh_token: if token.refresh_token.is_empty() {
        None
      } else {
        Some(token.refresh_token)
      },
      token_type: if token.token_type.is_empty() {
        None
      } else {
        Some(token.token_type)
      },
      project_id,
      expired: None,
      last_refresh: None,
      redirect_uri: Some(redirect_uri),
    };
    stored.set_expiry_from_secs(token.expires_in);
    stored.touch_refresh();
    store.save_account(ProviderKind::Antigravity, &stored.account_key(), &stored)?;
    Ok(AuthEntry::Antigravity(stored))
  }

  pub(super) async fn refresh(&self, store: &AuthStore, entry: &AuthEntry) -> AppResult<AuthEntry> {
    let mut stored = self.load_stored(store, entry)?;
    let refresh = stored
      .refresh_token
      .as_deref()
      .filter(|s| !s.is_empty())
      .ok_or_else(|| AppError::Unauthorized("antigravity: missing refresh_token".into()))?
      .to_string();

    let (status, body) = self
      .post_token_form(&[
        ("client_id", CLIENT_ID),
        ("client_secret", CLIENT_SECRET),
        ("refresh_token", &refresh),
        ("grant_type", "refresh_token"),
      ])
      .await?;
    if !status.is_success() {
      return Err(AppError::Unauthorized(format!(
        "antigravity token refresh failed ({status}): {body}"
      )));
    }
    let token: TokenResponse = serde_json::from_str(&body)
      .map_err(|e| AppError::Internal(format!("antigravity token refresh parse: {e}")))?;

    stored.access_token = Some(token.access_token);
    if !token.refresh_token.is_empty() {
      stored.refresh_token = Some(token.refresh_token);
    }
    if !token.token_type.is_empty() {
      stored.token_type = Some(token.token_type);
    }
    stored.set_expiry_from_secs(token.expires_in);
    stored.touch_refresh();
    store.save_account(ProviderKind::Antigravity, &stored.account_key(), &stored)?;
    Ok(AuthEntry::Antigravity(stored))
  }

  async fn exchange_code(&self, code: &str, redirect_uri: &str) -> AppResult<TokenResponse> {
    let (status, body) = self
      .post_token_form(&[
        ("code", code),
        ("client_id", CLIENT_ID),
        ("client_secret", CLIENT_SECRET),
        ("redirect_uri", redirect_uri),
        ("grant_type", "authorization_code"),
      ])
      .await?;
    if !status.is_success() {
      return Err(AppError::Unauthorized(format!(
        "antigravity token exchange failed ({status}): {body}"
      )));
    }
    serde_json::from_str(&body)
      .map_err(|e| AppError::Internal(format!("antigravity token exchange parse: {e}")))
  }

  async fn fetch_user_email(&self, access_token: &str) -> AppResult<String> {
    // Authorization + User-Agent only.
    let resp = self
      .http
      .get(USERINFO_ENDPOINT)
      .header(
        reqwest::header::AUTHORIZATION,
        format!("Bearer {access_token}"),
      )
      .header(reqwest::header::USER_AGENT, REQUEST_USER_AGENT)
      .send()
      .await
      .map_err(|e| AppError::Internal(format!("antigravity userinfo: {e}")))?;
    let status = resp.status();
    let body = resp
      .text()
      .await
      .map_err(|e| AppError::Internal(format!("antigravity userinfo body: {e}")))?;
    if !status.is_success() {
      return Err(AppError::Internal(format!(
        "antigravity userinfo failed ({status}): {body}"
      )));
    }
    let info: UserInfo = serde_json::from_str(&body)
      .map_err(|e| AppError::Internal(format!("antigravity userinfo parse: {e}")))?;
    if info.email.trim().is_empty() {
      return Err(AppError::Internal(
        "antigravity userinfo missing email".into(),
      ));
    }
    Ok(info.email)
  }

  async fn fetch_project_id(&self, access_token: &str) -> AppResult<String> {
    let endpoint = format!("{API_ENDPOINT}/{API_VERSION}:loadCodeAssist");
    let body = LoadCodeAssistRequest {
      metadata: LoadCodeAssistMetadata {
        ide_type: "ANTIGRAVITY",
      },
    };
    let resp = self
      .http
      .post(&endpoint)
      .header(
        reqwest::header::AUTHORIZATION,
        format!("Bearer {access_token}"),
      )
      .header(reqwest::header::ACCEPT, "*/*")
      .header(reqwest::header::CONTENT_TYPE, "application/json")
      .header(reqwest::header::USER_AGENT, REQUEST_USER_AGENT)
      .json(&body)
      .send()
      .await
      .map_err(|e| AppError::Internal(format!("antigravity loadCodeAssist: {e}")))?;
    let status = resp.status();
    let text = resp
      .text()
      .await
      .map_err(|e| AppError::Internal(format!("antigravity loadCodeAssist body: {e}")))?;
    if !status.is_success() {
      return Err(AppError::Internal(format!(
        "antigravity loadCodeAssist failed ({status}): {text}"
      )));
    }
    let data: LoadCodeAssistResponse = serde_json::from_str(&text)
      .map_err(|e| AppError::Internal(format!("antigravity loadCodeAssist parse: {e}")))?;
    if let Some(pid) = data.project_id() {
      return Ok(pid);
    }
    self
      .onboard_user(access_token, data.default_tier_id())
      .await
  }

  async fn onboard_user(&self, access_token: &str, tier_id: String) -> AppResult<String> {
    info!(%tier_id, "antigravity: onboarding user");
    let endpoint = format!("{DAILY_API_ENDPOINT}/{API_VERSION}:onboardUser");
    let body = OnboardUserRequest {
      tier_id,
      metadata: OnboardUserMetadata {
        ide_type: "ANTIGRAVITY",
        ide_version: "2.2.1",
        ide_name: "antigravity",
      },
    };
    for attempt in 1..=5 {
      let resp = self
        .http
        .post(&endpoint)
        .header(
          reqwest::header::AUTHORIZATION,
          format!("Bearer {access_token}"),
        )
        .header(reqwest::header::ACCEPT, "*/*")
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .header(reqwest::header::USER_AGENT, ONBOARD_USER_AGENT)
        .header("X-Goog-Api-Client", GOOG_API_CLIENT)
        .json(&body)
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("antigravity onboardUser: {e}")))?;
      let status = resp.status();
      let text = resp
        .text()
        .await
        .map_err(|e| AppError::Internal(format!("antigravity onboardUser body: {e}")))?;
      if status.is_success() {
        let data: OnboardUserResponse = serde_json::from_str(&text)
          .map_err(|e| AppError::Internal(format!("antigravity onboardUser parse: {e}")))?;
        if data.done == Some(true) {
          if let Some(pid) = data
            .response
            .as_ref()
            .and_then(|r| r.project_id())
            .or_else(|| data.project_id())
          {
            return Ok(pid);
          }
          return Err(AppError::Internal(
            "antigravity onboardUser: no project_id in response".into(),
          ));
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
        continue;
      }
      return Err(AppError::Internal(format!(
        "antigravity onboardUser attempt {attempt} failed ({status}): {text}"
      )));
    }
    Err(AppError::Internal(
      "antigravity onboardUser did not complete".into(),
    ))
  }

  /// Google token endpoint form POST (`Content-Type: application/x-www-form-urlencoded` only).
  async fn post_token_form(
    &self,
    fields: &[(&str, &str)],
  ) -> AppResult<(reqwest::StatusCode, String)> {
    let body = url::form_urlencoded::Serializer::new(String::new())
      .extend_pairs(fields.iter().copied())
      .finish();
    let resp = self
      .http
      .post(TOKEN_ENDPOINT)
      .header(
        reqwest::header::CONTENT_TYPE,
        "application/x-www-form-urlencoded",
      )
      .body(body)
      .send()
      .await
      .map_err(|e| AppError::Internal(format!("antigravity token request: {e}")))?;
    let status = resp.status();
    let text = resp
      .text()
      .await
      .map_err(|e| AppError::Internal(format!("antigravity token body: {e}")))?;
    Ok((status, text))
  }
}

#[derive(Debug, Serialize)]
struct LoadCodeAssistRequest {
  metadata: LoadCodeAssistMetadata,
}

#[derive(Debug, Serialize)]
struct LoadCodeAssistMetadata {
  #[serde(rename = "ideType")]
  ide_type: &'static str,
}

#[derive(Debug, Deserialize)]
struct LoadCodeAssistResponse {
  #[serde(default, rename = "cloudaicompanionProject")]
  cloud_ai_companion_project: Option<ProjectRef>,
  #[serde(default, rename = "projectId")]
  project_id_field: Option<String>,
  #[serde(default)]
  project: Option<ProjectRef>,
  #[serde(default, rename = "allowedTiers")]
  allowed_tiers: Vec<TierInfo>,
  #[serde(default, rename = "currentTier")]
  current_tier: Option<TierInfo>,
}

impl LoadCodeAssistResponse {
  fn project_id(&self) -> Option<String> {
    if let Some(s) = self
      .project_id_field
      .as_deref()
      .map(str::trim)
      .filter(|s| !s.is_empty())
    {
      return Some(s.to_string());
    }
    self
      .cloud_ai_companion_project
      .as_ref()
      .and_then(|p| p.id())
      .or_else(|| self.project.as_ref().and_then(|p| p.id()))
  }

  fn default_tier_id(&self) -> String {
    for tier in &self.allowed_tiers {
      if tier.is_default == Some(true) {
        if let Some(id) = tier.id.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
          return id.to_string();
        }
      }
    }
    if let Some(id) = self
      .current_tier
      .as_ref()
      .and_then(|t| t.id.as_deref())
      .map(str::trim)
      .filter(|s| !s.is_empty())
    {
      return id.to_string();
    }
    "free-tier".into()
  }
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ProjectRef {
  Id(String),
  Object { id: Option<String> },
}

impl ProjectRef {
  fn id(&self) -> Option<String> {
    match self {
      Self::Id(s) => {
        let t = s.trim();
        if t.is_empty() {
          None
        } else {
          Some(t.to_string())
        }
      }
      Self::Object { id } => id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string()),
    }
  }
}

#[derive(Debug, Deserialize)]
struct TierInfo {
  #[serde(default)]
  id: Option<String>,
  #[serde(default, rename = "isDefault")]
  is_default: Option<bool>,
}

#[derive(Debug, Serialize)]
struct OnboardUserRequest {
  tier_id: String,
  metadata: OnboardUserMetadata,
}

#[derive(Debug, Serialize)]
struct OnboardUserMetadata {
  ide_type: &'static str,
  ide_version: &'static str,
  ide_name: &'static str,
}

#[derive(Debug, Deserialize)]
struct OnboardUserResponse {
  #[serde(default)]
  done: Option<bool>,
  #[serde(default)]
  response: Option<OnboardUserInner>,
  #[serde(default, rename = "cloudaicompanionProject")]
  cloud_ai_companion_project: Option<ProjectRef>,
  #[serde(default, rename = "projectId")]
  project_id_field: Option<String>,
  #[serde(default)]
  project: Option<ProjectRef>,
}

impl OnboardUserResponse {
  fn project_id(&self) -> Option<String> {
    if let Some(s) = self
      .project_id_field
      .as_deref()
      .map(str::trim)
      .filter(|s| !s.is_empty())
    {
      return Some(s.to_string());
    }
    self
      .cloud_ai_companion_project
      .as_ref()
      .and_then(|p| p.id())
      .or_else(|| self.project.as_ref().and_then(|p| p.id()))
  }
}

#[derive(Debug, Deserialize)]
struct OnboardUserInner {
  #[serde(default, rename = "cloudaicompanionProject")]
  cloud_ai_companion_project: Option<ProjectRef>,
  #[serde(default, rename = "projectId")]
  project_id_field: Option<String>,
  #[serde(default)]
  project: Option<ProjectRef>,
}

impl OnboardUserInner {
  fn project_id(&self) -> Option<String> {
    if let Some(s) = self
      .project_id_field
      .as_deref()
      .map(str::trim)
      .filter(|s| !s.is_empty())
    {
      return Some(s.to_string());
    }
    self
      .cloud_ai_companion_project
      .as_ref()
      .and_then(|p| p.id())
      .or_else(|| self.project.as_ref().and_then(|p| p.id()))
  }
}
