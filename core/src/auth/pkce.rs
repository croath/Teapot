//! PKCE (RFC 7636) helpers for OAuth authorization-code flows.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::RngCore;
use sha2::{Digest, Sha256};

use crate::error::AppResult;

/// PKCE verifier + S256 challenge pair.
#[derive(Debug, Clone)]
pub struct PkceCodes {
  pub code_verifier: String,
  pub code_challenge: String,
}

/// Generate a high-entropy PKCE pair (S256).
pub fn generate_pkce() -> AppResult<PkceCodes> {
  let mut bytes = [0u8; 96];
  rand::rng().fill_bytes(&mut bytes);
  let code_verifier = URL_SAFE_NO_PAD.encode(bytes);
  let hash = Sha256::digest(code_verifier.as_bytes());
  let code_challenge = URL_SAFE_NO_PAD.encode(hash);
  Ok(PkceCodes {
    code_verifier,
    code_challenge,
  })
}

/// Cryptographically random URL-safe state string (OAuth CSRF protection).
pub fn generate_state() -> AppResult<String> {
  let mut bytes = [0u8; 32];
  rand::rng().fill_bytes(&mut bytes);
  Ok(URL_SAFE_NO_PAD.encode(bytes))
}
