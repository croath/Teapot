//! Grok CLI provider (`grok-cli`) — local `grok agent stdio` ACP JSON-RPC.

mod compact;
mod execute;
mod models;
mod stdio;

use std::collections::HashMap;

use crate::auth::AuthMethod;
use crate::error::{AppError, AppResult};
use crate::providers::ProviderKind;
use crate::providers::traits::{PromptRequest, Provider, SpawnSpec, StdoutCodec, resolve_binary};

pub use models::{GrokCliModel, parse_models_output};

/// Grok Build CLI via `grok agent stdio` (no Teapot-stored credentials).
///
/// Each execute spawns a `grok agent --always-approve stdio` process, speaks
/// ACP JSON-RPC on stdin/stdout (stream in and stream out), and reads
/// `session/update` chunks. Auth lives in the local Grok login (`grok login`).
#[derive(Debug, Clone, Default)]
pub struct GrokCliProvider;

impl GrokCliProvider {
  pub fn new() -> Self {
    Self
  }

  /// Confirm the `grok` binary is on PATH (called at session bootstrap).
  pub async fn ensure_session(&self) -> AppResult<()> {
    let _ = require_grok_binary()?;
    Ok(())
  }

  /// True when the local CLI is missing (no long-lived process to keep alive).
  pub async fn session_needs_refresh(&self) -> bool {
    resolve_binary("grok").is_none()
  }
}

fn require_grok_binary() -> AppResult<String> {
  resolve_binary("grok").ok_or_else(|| {
    AppError::ProviderBinaryMissing(
      "grok: install Grok Build CLI and ensure `grok` is on PATH".into(),
    )
  })
}

impl Provider for GrokCliProvider {
  fn kind(&self) -> ProviderKind {
    ProviderKind::GrokCli
  }

  fn description(&self) -> &str {
    "Grok Build CLI (local ACP stdio; uses `grok login`)"
  }

  fn command(&self) -> &str {
    "grok"
  }

  fn list_models_args(&self) -> Vec<String> {
    vec!["models".into()]
  }

  fn spawn_spec(&self, req: &PromptRequest) -> SpawnSpec {
    let mut args = vec![
      "agent".into(),
      "--always-approve".into(),
      "--no-leader".into(),
    ];
    if let Some(model) = req.model.as_deref().filter(|s| !s.is_empty()) {
      args.push("-m".into());
      args.push(model.to_string());
    }
    args.push("stdio".into());
    SpawnSpec {
      program: "grok".into(),
      args,
      stdin: None,
      cwd: None,
      env: HashMap::new(),
      timeout_secs: self.timeout_secs(),
      stdout_codec: StdoutCodec::GrokStreamingJson,
    }
  }

  fn auth_method(&self) -> AuthMethod {
    AuthMethod::None
  }
}
