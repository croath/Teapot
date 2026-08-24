//! Claude CLI provider (`claude-cli`) — local `claude -p` stream-json stdio.

mod compact;
mod execute;
mod models;
mod stdio;

use std::collections::HashMap;

use crate::auth::AuthMethod;
use crate::error::{AppError, AppResult};
use crate::providers::ProviderKind;
use crate::providers::traits::{
  PromptRequest, Provider, SpawnSpec, StdoutCodec, resolve_binary, stdin_prompt,
};

pub use models::{ClaudeCliModel, builtin_models};

/// Claude Code via `claude -p` stream-json (no Teapot-stored credentials).
///
/// Each execute spawns a `claude` process, writes a user message on stdin,
/// and reads NDJSON events from stdout. Auth lives in the local Claude Code
/// login (`claude auth login`).
#[derive(Debug, Clone, Default)]
pub struct ClaudeCliProvider;

impl ClaudeCliProvider {
  pub fn new() -> Self {
    Self
  }

  /// Confirm the `claude` binary is on PATH (called at session bootstrap).
  pub async fn ensure_session(&self) -> AppResult<()> {
    let _ = require_claude_binary()?;
    Ok(())
  }

  /// True when the local CLI is missing (no long-lived process to keep alive).
  pub async fn session_needs_refresh(&self) -> bool {
    resolve_binary("claude").is_none()
  }
}

fn require_claude_binary() -> AppResult<String> {
  resolve_binary("claude").ok_or_else(|| {
    AppError::ProviderBinaryMissing(
      "claude: install Claude Code and ensure `claude` is on PATH".into(),
    )
  })
}

impl Provider for ClaudeCliProvider {
  fn kind(&self) -> ProviderKind {
    ProviderKind::ClaudeCli
  }

  fn description(&self) -> &str {
    "Anthropic Claude Code CLI (local stream-json; uses `claude auth login`)"
  }

  fn command(&self) -> &str {
    "claude"
  }

  fn list_models_args(&self) -> Vec<String> {
    Vec::new()
  }

  fn spawn_spec(&self, req: &PromptRequest) -> SpawnSpec {
    let mut args = vec![
      "-p".into(),
      "--output-format".into(),
      "stream-json".into(),
      "--input-format".into(),
      "stream-json".into(),
      "--verbose".into(),
      "--include-partial-messages".into(),
      "--dangerously-skip-permissions".into(),
    ];
    if let Some(model) = req.model.as_deref().filter(|s| !s.is_empty()) {
      args.push("--model".into());
      args.push(model.to_string());
    }
    if let Some(system) = req.system.as_deref().filter(|s| !s.is_empty()) {
      args.push("--append-system-prompt".into());
      args.push(system.to_string());
    }
    SpawnSpec {
      program: "claude".into(),
      args,
      stdin: Some(stream_json_user_line(&stdin_prompt(req))),
      cwd: None,
      env: HashMap::new(),
      timeout_secs: self.timeout_secs(),
      stdout_codec: StdoutCodec::ClaudeStreamJson,
    }
  }

  fn auth_method(&self) -> AuthMethod {
    AuthMethod::None
  }
}

fn stream_json_user_line(text: &str) -> String {
  let msg = serde_json::json!({
    "type": "user",
    "message": {
      "role": "user",
      "content": [{ "type": "text", "text": text }],
    }
  });
  format!("{msg}\n")
}
