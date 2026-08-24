//! Grok CLI provider (`grok-cli`) — local `grok agent stdio` ACP JSON-RPC.

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
use crate::providers::traits::{PromptRequest, Provider, SpawnSpec, StdoutCodec};

use stdio::AcpSession;

pub use models::{GrokCliModel, parse_models_output};

/// Grok Build CLI via `grok agent stdio` (no Teapot-stored credentials).
///
/// One `grok agent --always-approve --no-leader stdio` process is started with
/// teapotx and reused. Each HTTP execute opens a fresh ACP session
/// (`session/new` → `session/prompt`) so requests do not share conversation
/// history. Auth lives in the local Grok login (`grok login`).
#[derive(Clone)]
pub struct GrokCliProvider {
  session: Arc<Mutex<Option<AcpSession>>>,
}

impl fmt::Debug for GrokCliProvider {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    f.debug_struct("GrokCliProvider").finish_non_exhaustive()
  }
}

impl GrokCliProvider {
  pub fn new() -> Self {
    Self {
      session: Arc::new(Mutex::new(None)),
    }
  }

  /// Start `grok agent stdio` if it is not running (or restart if it died).
  pub async fn ensure_session(&self) -> AppResult<()> {
    let mut guard = self.lock_session().await?;
    let _ = guard.as_mut();
    Ok(())
  }

  /// True when the long-lived agent is missing or has exited.
  pub async fn session_needs_refresh(&self) -> bool {
    let mut guard = self.session.lock().await;
    match guard.as_mut() {
      Some(session) => !session.is_alive(),
      None => true,
    }
  }

  pub(super) async fn lock_session(
    &self,
  ) -> AppResult<tokio::sync::MutexGuard<'_, Option<AcpSession>>> {
    let mut guard = self.session.lock().await;
    let restart = match guard.as_mut() {
      None => true,
      Some(session) => !session.is_alive(),
    };
    if restart {
      *guard = None;
      let session = AcpSession::spawn().await?;
      session.handshake().await?;
      info!("grok-cli: agent started (stdio ACP, reused for this process)");
      *guard = Some(session);
    }
    Ok(guard)
  }

  pub(super) fn session_mut<'a>(
    guard: &'a mut tokio::sync::MutexGuard<'_, Option<AcpSession>>,
  ) -> AppResult<&'a mut AcpSession> {
    guard
      .as_mut()
      .ok_or_else(|| AppError::Internal("grok-cli: agent session missing after connect".into()))
  }
}

impl Default for GrokCliProvider {
  fn default() -> Self {
    Self::new()
  }
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
