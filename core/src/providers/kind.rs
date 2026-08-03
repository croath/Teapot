//! First-party provider identity as an enum (no free-form strings in APIs).

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

/// Built-in Teapot provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
  Codex,
  Claude,
  Xai,
  Antigravity,
  Vertex,
}

impl ProviderKind {
  /// All first-party providers in display order.
  pub const ALL: &'static [ProviderKind] = &[
    ProviderKind::Codex,
    ProviderKind::Claude,
    ProviderKind::Xai,
    ProviderKind::Antigravity,
    ProviderKind::Vertex,
  ];

  /// Canonical JSON / config key (`"codex"`, `"claude"`, …).
  pub const fn as_str(self) -> &'static str {
    match self {
      Self::Codex => "codex",
      Self::Claude => "claude",
      Self::Xai => "xai",
      Self::Antigravity => "antigravity",
      Self::Vertex => "vertex",
    }
  }

  /// Parse a canonical id or known alias.
  pub fn parse(name: &str) -> AppResult<Self> {
    name.parse()
  }
}

impl fmt::Display for ProviderKind {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    f.write_str(self.as_str())
  }
}

impl FromStr for ProviderKind {
  type Err = AppError;

  fn from_str(s: &str) -> Result<Self, Self::Err> {
    match s {
      "codex" | "codex-ci" => Ok(Self::Codex),
      "claude" | "claude-cli" => Ok(Self::Claude),
      "xai" | "grok" | "grok-build" | "grok-build-cli" => Ok(Self::Xai),
      "antigravity" | "agy" | "antigravity-cli" => Ok(Self::Antigravity),
      "vertex" | "vertex-ai" | "gemini-vertex" => Ok(Self::Vertex),
      other => Err(AppError::BadRequest(format!("unknown provider `{other}`"))),
    }
  }
}
