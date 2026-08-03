//! Type-safe auth records: each variant holds the provider's own storage struct.

use chrono::{DateTime, Utc};

use crate::error::{AppError, AppResult};
use crate::providers::ProviderKind;
use crate::providers::antigravity::StoredAuth as AntigravityAuth;
use crate::providers::claude::StoredAuth as ClaudeAuth;
use crate::providers::codex::StoredAuth as CodexAuth;
use crate::providers::vertex::StoredAuth as VertexAuth;
use crate::providers::xai::StoredAuth as XaiAuth;

/// One saved credential: the original provider-owned struct (not a flattened bag).
#[derive(Debug, Clone)]
pub enum AuthEntry {
  Codex(CodexAuth),
  Claude(ClaudeAuth),
  Xai(XaiAuth),
  Antigravity(AntigravityAuth),
  Vertex(VertexAuth),
}

impl AuthEntry {
  pub fn provider(&self) -> ProviderKind {
    match self {
      Self::Codex(_) => ProviderKind::Codex,
      Self::Claude(_) => ProviderKind::Claude,
      Self::Xai(_) => ProviderKind::Xai,
      Self::Antigravity(_) => ProviderKind::Antigravity,
      Self::Vertex(_) => ProviderKind::Vertex,
    }
  }

  /// Account key used under `auth/{provider}.json` → `{account}`.
  pub fn account_key(&self) -> String {
    match self {
      Self::Codex(a) => a.account_key(),
      Self::Claude(a) => a.account_key(),
      Self::Xai(a) => a.account_key(),
      Self::Antigravity(a) => a.account_key(),
      Self::Vertex(a) => a.account_key(),
    }
  }

  pub fn email(&self) -> Option<&str> {
    match self {
      Self::Codex(a) => a.email.as_deref(),
      Self::Claude(a) => a.email.as_deref(),
      Self::Xai(a) => a.email.as_deref(),
      Self::Antigravity(a) => a.email.as_deref(),
      Self::Vertex(a) => a.email.as_deref(),
    }
  }

  pub fn auth_kind(&self) -> &str {
    match self {
      Self::Codex(a) => a.auth_kind.as_str(),
      Self::Claude(a) => a.auth_kind.as_str(),
      Self::Xai(a) => a.auth_kind.as_str(),
      Self::Antigravity(a) => a.auth_kind.as_str(),
      Self::Vertex(a) => a.auth_kind.as_str(),
    }
  }

  pub fn expired(&self) -> Option<&str> {
    match self {
      Self::Codex(a) => a.expired.as_deref(),
      Self::Claude(a) => a.expired.as_deref(),
      Self::Xai(a) => a.expired.as_deref(),
      Self::Antigravity(a) => a.expired.as_deref(),
      Self::Vertex(_) => None,
    }
  }

  pub fn last_refresh(&self) -> Option<&str> {
    match self {
      Self::Codex(a) => a.last_refresh.as_deref(),
      Self::Claude(a) => a.last_refresh.as_deref(),
      Self::Xai(a) => a.last_refresh.as_deref(),
      Self::Antigravity(a) => a.last_refresh.as_deref(),
      Self::Vertex(a) => a.last_refresh.as_deref(),
    }
  }

  pub fn expires_at(&self) -> Option<DateTime<Utc>> {
    let raw = self.expired()?;
    DateTime::parse_from_rfc3339(raw)
      .ok()
      .map(|dt| dt.with_timezone(&Utc))
  }

  /// Whether this credential should be refreshed (provider-specific rules).
  pub fn needs_refresh(&self, lead: chrono::Duration) -> bool {
    match self {
      Self::Vertex(_) => false,
      Self::Codex(a) => oauth_needs_refresh(a.access_token.as_deref(), a.expired.as_deref(), lead),
      Self::Claude(a) => oauth_needs_refresh(a.access_token.as_deref(), a.expired.as_deref(), lead),
      Self::Xai(a) => oauth_needs_refresh(a.access_token.as_deref(), a.expired.as_deref(), lead),
      Self::Antigravity(a) => {
        oauth_needs_refresh(a.access_token.as_deref(), a.expired.as_deref(), lead)
      }
    }
  }

  /// Expect a Codex record (used by provider code).
  pub fn into_codex(self) -> AppResult<CodexAuth> {
    match self {
      Self::Codex(a) => Ok(a),
      other => Err(AppError::BadRequest(format!(
        "expected codex auth, got {}",
        other.provider()
      ))),
    }
  }

  pub fn into_claude(self) -> AppResult<ClaudeAuth> {
    match self {
      Self::Claude(a) => Ok(a),
      other => Err(AppError::BadRequest(format!(
        "expected claude auth, got {}",
        other.provider()
      ))),
    }
  }

  pub fn into_xai(self) -> AppResult<XaiAuth> {
    match self {
      Self::Xai(a) => Ok(a),
      other => Err(AppError::BadRequest(format!(
        "expected xai auth, got {}",
        other.provider()
      ))),
    }
  }

  pub fn into_antigravity(self) -> AppResult<AntigravityAuth> {
    match self {
      Self::Antigravity(a) => Ok(a),
      other => Err(AppError::BadRequest(format!(
        "expected antigravity auth, got {}",
        other.provider()
      ))),
    }
  }

  pub fn into_vertex(self) -> AppResult<VertexAuth> {
    match self {
      Self::Vertex(a) => Ok(a),
      other => Err(AppError::BadRequest(format!(
        "expected vertex auth, got {}",
        other.provider()
      ))),
    }
  }

  pub fn as_codex(&self) -> AppResult<&CodexAuth> {
    match self {
      Self::Codex(a) => Ok(a),
      other => Err(AppError::BadRequest(format!(
        "expected codex auth, got {}",
        other.provider()
      ))),
    }
  }

  pub fn as_claude(&self) -> AppResult<&ClaudeAuth> {
    match self {
      Self::Claude(a) => Ok(a),
      other => Err(AppError::BadRequest(format!(
        "expected claude auth, got {}",
        other.provider()
      ))),
    }
  }

  pub fn as_xai(&self) -> AppResult<&XaiAuth> {
    match self {
      Self::Xai(a) => Ok(a),
      other => Err(AppError::BadRequest(format!(
        "expected xai auth, got {}",
        other.provider()
      ))),
    }
  }

  pub fn as_antigravity(&self) -> AppResult<&AntigravityAuth> {
    match self {
      Self::Antigravity(a) => Ok(a),
      other => Err(AppError::BadRequest(format!(
        "expected antigravity auth, got {}",
        other.provider()
      ))),
    }
  }

  pub fn as_vertex(&self) -> AppResult<&VertexAuth> {
    match self {
      Self::Vertex(a) => Ok(a),
      other => Err(AppError::BadRequest(format!(
        "expected vertex auth, got {}",
        other.provider()
      ))),
    }
  }
}

fn oauth_needs_refresh(
  access_token: Option<&str>,
  expired: Option<&str>,
  lead: chrono::Duration,
) -> bool {
  if access_token.unwrap_or("").is_empty() {
    return true;
  }
  match expired.and_then(|raw| DateTime::parse_from_rfc3339(raw).ok()) {
    Some(exp) => Utc::now() + lead >= exp.with_timezone(&Utc),
    None => false,
  }
}
