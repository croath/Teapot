//! Claude CLI provider (`claude-cli`) — local `claude -p` stream-json stdio.

mod compact;
mod execute;
mod models;
mod stdio;

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use tokio::sync::Mutex;
use tracing::info;

use crate::auth::AuthMethod;
use crate::error::{AppError, AppResult};
use crate::providers::ProviderKind;
use crate::providers::traits::{
  PromptRequest, Provider, SpawnSpec, StdoutCodec, resolve_binary, stdin_prompt,
};

use stdio::StreamJsonSession;

pub use models::{ClaudeCliModel, builtin_models};

/// Claude Code via `claude -p` stream-json (no Teapot-stored credentials).
///
/// One `claude -p` process is reused for HTTP execute. Each request is a fresh
/// conversation (`/clear` between turns) so Chat Completions history is not
/// stacked onto the previous call. Model / system changes respawn the child.
/// Auth lives in the local Claude Code login (`claude auth login`).
#[derive(Clone)]
pub struct ClaudeCliProvider {
  session: Arc<Mutex<Option<StreamJsonSession>>>,
}

impl fmt::Debug for ClaudeCliProvider {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    f.debug_struct("ClaudeCliProvider").finish_non_exhaustive()
  }
}

impl ClaudeCliProvider {
  pub fn new() -> Self {
    Self {
      session: Arc::new(Mutex::new(None)),
    }
  }

  /// Confirm the `claude` binary is on PATH (called at session bootstrap).
  pub async fn ensure_session(&self) -> AppResult<()> {
    let _ = require_claude_binary()?;
    Ok(())
  }

  /// True when a live child has exited and should be replaced on next execute.
  pub async fn session_needs_refresh(&self) -> bool {
    let mut guard = self.session.lock().await;
    match guard.as_mut() {
      Some(session) => !session.is_alive(),
      None => false,
    }
  }

  pub(super) async fn lock_session(
    &self,
    model: &str,
    system: Option<&str>,
  ) -> AppResult<tokio::sync::MutexGuard<'_, Option<StreamJsonSession>>> {
    let mut guard = self.session.lock().await;
    let restart = match guard.as_mut() {
      None => true,
      Some(session) => {
        if !session.is_alive() || !session.matches(model, system) {
          true
        } else if session.needs_clear() {
          match session.reset_conversation().await {
            Ok(()) => false,
            Err(e) => {
              tracing::debug!(error = %e, "claude-cli: /clear failed; restarting process");
              true
            }
          }
        } else {
          false
        }
      }
    };
    if restart {
      *guard = None;
      let session = StreamJsonSession::spawn(model, system).await?;
      info!("claude-cli: stream-json started (reused for this process)");
      *guard = Some(session);
    }
    Ok(guard)
  }

  pub(super) fn session_mut<'a>(
    guard: &'a mut tokio::sync::MutexGuard<'_, Option<StreamJsonSession>>,
  ) -> AppResult<&'a mut StreamJsonSession> {
    guard.as_mut().ok_or_else(|| {
      AppError::Internal("claude-cli: stream-json session missing after connect".into())
    })
  }
}

impl Default for ClaudeCliProvider {
  fn default() -> Self {
    Self::new()
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
