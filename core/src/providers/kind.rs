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
  #[serde(rename = "codex-cli")]
  CodexCli,
  Claude,
  #[serde(rename = "claude-cli")]
  ClaudeCli,
  #[serde(rename = "grok-cli")]
  GrokCli,
  Xai,
  Antigravity,
  Vertex,
}

impl ProviderKind {
  /// All compiled first-party providers (includes backends hidden from CLI/UI).
  pub const ALL: &'static [ProviderKind] = &[
    ProviderKind::Codex,
    ProviderKind::CodexCli,
    ProviderKind::Claude,
    ProviderKind::ClaudeCli,
    ProviderKind::GrokCli,
    ProviderKind::Xai,
    ProviderKind::Antigravity,
    ProviderKind::Vertex,
  ];

  /// Providers listed in CLI pickers and the desktop UI.
  ///
  /// `codex`, `claude`, and `xai` stay compiled (`ALL` / `PinnedProvider`) but
  /// are not offered as selectable backends.
  pub const OFFERED: &'static [ProviderKind] = &[
    ProviderKind::CodexCli,
    ProviderKind::ClaudeCli,
    ProviderKind::GrokCli,
    ProviderKind::Antigravity,
    ProviderKind::Vertex,
  ];

  /// Default offered provider when none is configured.
  pub const DEFAULT: ProviderKind = ProviderKind::CodexCli;

  /// Canonical JSON / config key (`"codex"`, `"claude"`, …).
  pub const fn as_str(self) -> &'static str {
    match self {
      Self::Codex => "codex",
      Self::CodexCli => "codex-cli",
      Self::Claude => "claude",
      Self::ClaudeCli => "claude-cli",
      Self::GrokCli => "grok-cli",
      Self::Xai => "xai",
      Self::Antigravity => "antigravity",
      Self::Vertex => "vertex",
    }
  }

  /// Short label for pickers.
  pub const fn display_name(self) -> &'static str {
    match self {
      Self::Codex => "Codex",
      Self::CodexCli => "Codex CLI",
      Self::Claude => "Claude",
      Self::ClaudeCli => "Claude CLI",
      Self::GrokCli => "Grok CLI",
      Self::Xai => "xAI",
      Self::Antigravity => "Antigravity",
      Self::Vertex => "Vertex",
    }
  }

  /// True when serve/execute needs a local CLI binary on PATH (`codex`, …).
  pub const fn requires_local_cli(self) -> bool {
    matches!(self, Self::CodexCli | Self::ClaudeCli | Self::GrokCli)
  }

  /// How to install the local CLI, if this provider needs one.
  pub const fn install_hint(self) -> Option<&'static str> {
    match self {
      Self::CodexCli => Some(
        "Install Codex CLI, then restart Teapot. macOS: `brew install --cask codex` · or `npm install -g @openai/codex` · or `curl -fsSL https://chatgpt.com/codex/install.sh | sh`",
      ),
      Self::ClaudeCli => Some(
        "Install Claude Code, then restart Teapot. `curl -fsSL https://claude.ai/install.sh | bash` · or `npm install -g @anthropic-ai/claude-code`",
      ),
      Self::GrokCli => Some(
        "Install Grok Build CLI, then restart Teapot. `curl -fsSL https://x.ai/cli/install.sh | bash` · then `grok login`",
      ),
      _ => None,
    }
  }

  /// Parse a canonical id or known alias.
  pub fn parse(name: &str) -> AppResult<Self> {
    name.parse()
  }

  /// True when this kind is listed in CLI and the desktop UI.
  pub const fn is_offered(self) -> bool {
    !matches!(self, Self::Codex | Self::Claude | Self::Xai)
  }

  /// Reject kinds that are compiled but not offered in CLI/UI.
  pub fn require_offered(self) -> AppResult<Self> {
    if self.is_offered() {
      Ok(self)
    } else {
      Err(AppError::BadRequest(format!(
        "provider `{}` is not available; choose from: {}",
        self.as_str(),
        Self::offered_names()
      )))
    }
  }

  /// Comma-separated offered ids for help / error text.
  pub fn offered_names() -> String {
    Self::OFFERED
      .iter()
      .map(|kind| kind.as_str())
      .collect::<Vec<_>>()
      .join(", ")
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
      "codex-cli" | "codex-app-server" => Ok(Self::CodexCli),
      "claude" => Ok(Self::Claude),
      "claude-cli" | "claude-code" => Ok(Self::ClaudeCli),
      "grok-cli" | "grok" | "grok-build" | "grok-build-cli" => Ok(Self::GrokCli),
      "xai" => Ok(Self::Xai),
      "antigravity" | "agy" | "antigravity-cli" => Ok(Self::Antigravity),
      "vertex" | "vertex-ai" | "gemini-vertex" => Ok(Self::Vertex),
      other => Err(AppError::BadRequest(format!("unknown provider `{other}`"))),
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn parse_codex_cli() {
    assert_eq!(
      ProviderKind::parse("codex-cli").unwrap(),
      ProviderKind::CodexCli
    );
    assert_eq!(ProviderKind::CodexCli.as_str(), "codex-cli");
    assert_ne!(
      ProviderKind::parse("codex").unwrap(),
      ProviderKind::CodexCli
    );
  }

  #[test]
  fn offered_hides_codex_claude_and_xai() {
    assert!(ProviderKind::CodexCli.is_offered());
    assert!(ProviderKind::ClaudeCli.is_offered());
    assert!(ProviderKind::GrokCli.is_offered());
    assert!(!ProviderKind::Xai.is_offered());
    assert!(!ProviderKind::Codex.is_offered());
    assert!(!ProviderKind::Claude.is_offered());
    assert!(ProviderKind::Codex.require_offered().is_err());
    assert!(ProviderKind::Claude.require_offered().is_err());
    assert!(ProviderKind::Xai.require_offered().is_err());
    assert_eq!(ProviderKind::DEFAULT, ProviderKind::CodexCli);
    assert_eq!(
      ProviderKind::OFFERED,
      &[
        ProviderKind::CodexCli,
        ProviderKind::ClaudeCli,
        ProviderKind::GrokCli,
        ProviderKind::Antigravity,
        ProviderKind::Vertex,
      ]
    );
  }

  #[test]
  fn parse_grok_cli() {
    assert_eq!(
      ProviderKind::parse("grok-cli").unwrap(),
      ProviderKind::GrokCli
    );
    assert_eq!(ProviderKind::parse("grok").unwrap(), ProviderKind::GrokCli);
    assert_eq!(
      ProviderKind::parse("grok-build").unwrap(),
      ProviderKind::GrokCli
    );
    assert_eq!(ProviderKind::parse("xai").unwrap(), ProviderKind::Xai);
    assert_eq!(ProviderKind::GrokCli.as_str(), "grok-cli");
  }

  #[test]
  fn parse_claude_cli() {
    assert_eq!(
      ProviderKind::parse("claude-cli").unwrap(),
      ProviderKind::ClaudeCli
    );
    assert_eq!(
      ProviderKind::parse("claude-code").unwrap(),
      ProviderKind::ClaudeCli
    );
    assert_eq!(ProviderKind::ClaudeCli.as_str(), "claude-cli");
    assert_ne!(
      ProviderKind::parse("claude").unwrap(),
      ProviderKind::ClaudeCli
    );
  }
}
