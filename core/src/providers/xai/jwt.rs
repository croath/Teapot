//! Decode xAI id_token claims (no signature verification).

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE, URL_SAFE_NO_PAD};
use serde::Deserialize;

use crate::error::{AppError, AppResult};

/// Identity fields taken from the xAI id_token.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct IdTokenClaims {
  #[serde(default)]
  pub email: Option<String>,
  #[serde(default)]
  pub sub: Option<String>,
}

impl IdTokenClaims {
  pub fn user_email(&self) -> Option<String> {
    self
      .email
      .as_ref()
      .map(|s| s.trim())
      .filter(|s| !s.is_empty())
      .map(|s| s.to_string())
  }

  pub fn subject(&self) -> Option<String> {
    self
      .sub
      .as_ref()
      .map(|s| s.trim())
      .filter(|s| !s.is_empty())
      .map(|s| s.to_string())
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
