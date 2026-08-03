//! Decode OpenAI id_token claims for Codex (no signature verification).

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE, URL_SAFE_NO_PAD};
use serde::Deserialize;

use crate::error::{AppError, AppResult};

/// Nested OpenAI auth claim object inside the id_token.
#[derive(Debug, Clone, Default, Deserialize)]
struct OpenAiAuthClaims {
  #[serde(default)]
  chatgpt_account_id: Option<String>,
  #[serde(default)]
  account_id: Option<String>,
}

/// Claims used to fill email / account_id after OAuth.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct IdTokenClaims {
  #[serde(default)]
  pub email: Option<String>,
  #[serde(default)]
  pub sub: Option<String>,
  #[serde(default)]
  pub preferred_username: Option<String>,
  #[serde(default)]
  pub user_email: Option<String>,
  #[serde(default, rename = "https://api.openai.com/auth")]
  api_openai_auth: Option<OpenAiAuthClaims>,
  #[serde(default, rename = "https://auth.openai.com/auth")]
  auth_openai_auth: Option<OpenAiAuthClaims>,
  #[serde(default)]
  chatgpt_account_id: Option<String>,
}

impl IdTokenClaims {
  pub fn user_email(&self) -> Option<String> {
    for candidate in [
      self.email.as_deref(),
      self.user_email.as_deref(),
      self.preferred_username.as_deref(),
    ] {
      if let Some(e) = candidate.map(str::trim).filter(|s| !s.is_empty()) {
        return Some(e.to_string());
      }
    }
    None
  }

  /// OpenAI account id from nested auth claims when present.
  pub fn account_id(&self) -> Option<String> {
    for auth in [&self.api_openai_auth, &self.auth_openai_auth] {
      if let Some(obj) = auth {
        for candidate in [obj.chatgpt_account_id.as_deref(), obj.account_id.as_deref()] {
          if let Some(id) = candidate.map(str::trim).filter(|s| !s.is_empty()) {
            return Some(id.to_string());
          }
        }
      }
    }
    if let Some(id) = self
      .chatgpt_account_id
      .as_deref()
      .map(str::trim)
      .filter(|s| !s.is_empty())
    {
      return Some(id.to_string());
    }
    self.sub.clone()
  }
}

/// Decode JWT payload without verifying the signature.
pub fn parse_id_token(token: &str) -> AppResult<IdTokenClaims> {
  let bytes = decode_jwt_payload(token)?;
  serde_json::from_slice(&bytes)
    .map_err(|e| AppError::BadRequest(format!("invalid JWT payload: {e}")))
}

fn decode_jwt_payload(token: &str) -> AppResult<Vec<u8>> {
  let token = token.trim();
  if token.is_empty() {
    return Err(AppError::BadRequest("empty JWT".into()));
  }
  let mut parts = token.split('.');
  let _header = parts
    .next()
    .ok_or_else(|| AppError::BadRequest("invalid JWT: missing header".into()))?;
  let payload = parts
    .next()
    .ok_or_else(|| AppError::BadRequest("invalid JWT: missing payload".into()))?;
  decode_b64url(payload)
}

fn decode_b64url(input: &str) -> AppResult<Vec<u8>> {
  if let Ok(b) = URL_SAFE_NO_PAD.decode(input) {
    return Ok(b);
  }
  if let Ok(b) = URL_SAFE.decode(input) {
    return Ok(b);
  }
  STANDARD
    .decode(input)
    .map_err(|e| AppError::BadRequest(format!("JWT base64 decode: {e}")))
}
