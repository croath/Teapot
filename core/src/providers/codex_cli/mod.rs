//! Codex CLI provider (`codex-cli`) — local `codex app-server` JSON-RPC.

mod compact;
mod execute;
mod models;
mod rpc;

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use tokio::sync::Mutex;
use tracing::info;

use crate::auth::AuthMethod;
use crate::error::{AppError, AppResult};
use crate::providers::ProviderKind;
use crate::providers::traits::{PromptRequest, Provider, SpawnSpec, StdoutCodec, stdin_prompt};

use rpc::AppServerSession;

pub use models::CodexCliModel;

/// Codex CLI via `codex app-server` (no Teapot-stored credentials).
///
/// One app-server process is started with teapotx and reused for models and
/// chat. Auth lives in the local Codex home (`codex login`).
#[derive(Clone)]
pub struct CodexCliProvider {
  session: Arc<Mutex<Option<AppServerSession>>>,
}

impl fmt::Debug for CodexCliProvider {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    f.debug_struct("CodexCliProvider").finish_non_exhaustive()
  }
}

impl CodexCliProvider {
  pub fn new() -> Self {
    Self {
      session: Arc::new(Mutex::new(None)),
    }
  }

  /// Start `codex app-server` if it is not running (or restart if it died).
  pub async fn ensure_session(&self) -> AppResult<()> {
    let mut guard = self.lock_session().await?;
    let _ = guard.as_mut();
    Ok(())
  }

  /// True when the long-lived app-server is missing or has exited.
  pub async fn session_needs_refresh(&self) -> bool {
    let mut guard = self.session.lock().await;
    match guard.as_mut() {
      Some(session) => !session.is_alive(),
      None => true,
    }
  }

  pub(super) async fn lock_session(
    &self,
  ) -> AppResult<tokio::sync::MutexGuard<'_, Option<AppServerSession>>> {
    let mut guard = self.session.lock().await;
    let restart = match guard.as_mut() {
      None => true,
      Some(session) => !session.is_alive(),
    };
    if restart {
      *guard = None;
      let session = AppServerSession::spawn().await?;
      session.initialize().await?;
      info!("codex-cli: app-server started (stdio JSON-RPC, reused for this process)");
      *guard = Some(session);
    }
    Ok(guard)
  }

  pub(super) fn session_mut<'a>(
    guard: &'a mut tokio::sync::MutexGuard<'_, Option<AppServerSession>>,
  ) -> AppResult<&'a mut AppServerSession> {
    guard.as_mut().ok_or_else(|| {
      AppError::Internal("codex-cli: app-server session missing after connect".into())
    })
  }
}

impl Default for CodexCliProvider {
  fn default() -> Self {
    Self::new()
  }
}

impl Provider for CodexCliProvider {
  fn kind(&self) -> ProviderKind {
    ProviderKind::CodexCli
  }

  fn description(&self) -> &str {
    "OpenAI Codex CLI (local app-server; uses `codex login`)"
  }

  fn command(&self) -> &str {
    "codex"
  }

  fn list_models_args(&self) -> Vec<String> {
    Vec::new()
  }

  fn spawn_spec(&self, req: &PromptRequest) -> SpawnSpec {
    SpawnSpec {
      program: "codex".into(),
      args: vec![
        "exec".into(),
        "--skip-git-repo-check".into(),
        "--json".into(),
        "-".into(),
      ],
      stdin: Some(stdin_prompt(req)),
      cwd: None,
      env: HashMap::new(),
      timeout_secs: self.timeout_secs(),
      stdout_codec: StdoutCodec::CodexJsonl,
    }
  }

  fn auth_method(&self) -> AuthMethod {
    AuthMethod::None
  }
}
