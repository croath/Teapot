//! Core traits for provider CLI backends, process execution, and auth.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::{Stream, StreamExt};

use crate::auth::{AuthMethod, AuthStore, LoginOptions};
use crate::error::{AppError, AppResult};
use crate::providers::AuthEntry;
use crate::providers::ProviderKind;

// ---------------------------------------------------------------------------
// Events & session
// ---------------------------------------------------------------------------

/// Events emitted while a provider CLI is running.
#[derive(Debug, Clone)]
pub enum ProviderEvent {
  /// A chunk of stdout text (treated as model tokens / content delta).
  Token(String),
  /// Intermediate reasoning summary.
  Reasoning(String),
  /// Short progress / status line (tool/command lifecycle).
  Status(String),
  /// A line from stderr (diagnostic; not sent as model content).
  Stderr(String),
  /// Process finished successfully.
  Done { exit_code: i32 },
  /// Process failed or timed out.
  Failed(String),
}

/// Live session streaming provider events as a [`tokio_stream::Stream`].
pub struct ProviderSession {
  pub provider_name: String,
  events: ReceiverStream<ProviderEvent>,
}

impl ProviderSession {
  pub fn new(provider_name: String, receiver: mpsc::Receiver<ProviderEvent>) -> Self {
    Self {
      provider_name,
      events: ReceiverStream::new(receiver),
    }
  }

  pub async fn next_event(&mut self) -> Option<ProviderEvent> {
    self.events.next().await
  }

  pub fn provider_name(&self) -> &str {
    &self.provider_name
  }

  pub fn into_stream(self) -> ReceiverStream<ProviderEvent> {
    self.events
  }
}

impl Stream for ProviderSession {
  type Item = ProviderEvent;

  fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
    Pin::new(&mut self.events).poll_next(cx)
  }
}

// ---------------------------------------------------------------------------
// Request / spawn
// ---------------------------------------------------------------------------

/// Input for a single provider invocation.
#[derive(Debug, Clone)]
pub struct PromptRequest {
  pub system: Option<String>,
  pub prompt: String,
  /// Optional model id from the client (providers may forward it).
  pub model: Option<String>,
}

impl PromptRequest {
  pub fn new(prompt: impl Into<String>) -> Self {
    Self {
      system: None,
      prompt: prompt.into(),
      model: None,
    }
  }

  pub fn with_system(mut self, system: Option<String>) -> Self {
    self.system = system;
    self
  }

  pub fn with_model(mut self, model: Option<String>) -> Self {
    self.model = model;
    self
  }

  pub fn system_str(&self) -> &str {
    self.system.as_deref().unwrap_or("")
  }
}

/// How to interpret process stdout as model content.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum StdoutCodec {
  /// Forward raw stdout bytes as tokens (chunk-based).
  #[default]
  Raw,
  /// Codex `exec --json` JSONL: content + optional progress.
  CodexJsonl,
  /// Claude Code `--output-format stream-json` NDJSON: emit text deltas.
  ClaudeStreamJson,
}

/// How to spawn a provider process (produced by [`Provider::spawn_spec`]).
#[derive(Debug, Clone)]
pub struct SpawnSpec {
  pub program: String,
  pub args: Vec<String>,
  /// When set, written to stdin then closed.
  pub stdin: Option<String>,
  pub cwd: Option<PathBuf>,
  pub env: HashMap<String, String>,
  /// 0 = no timeout.
  pub timeout_secs: u64,
  /// How to turn stdout into [`ProviderEvent::Token`]s.
  pub stdout_codec: StdoutCodec,
}

// ---------------------------------------------------------------------------
// Traits
// ---------------------------------------------------------------------------

/// A provider CLI backend that can turn a prompt into a process spawn plan
/// and (optionally) authenticate against the upstream service.
///
/// Implementations hard-code command, argv, and codecs. Runtime: build
/// [`SpawnSpec`] → spawn → stream stdout. Each provider saves its own
/// [`AuthEntry`] / storage struct into `auth/{provider}.json` via [`Provider::save_auth`].
pub trait Provider: Send + Sync {
  /// Typed provider identity.
  fn kind(&self) -> ProviderKind;

  /// Canonical string id (same as [`ProviderKind::as_str`]).
  fn id(&self) -> &str {
    self.kind().as_str()
  }

  /// Human-readable description for listings.
  fn description(&self) -> &str;

  /// Executable name or path (looked up on `PATH` when relative).
  fn command(&self) -> &str;

  /// Process timeout in seconds (`0` = no timeout).
  fn timeout_secs(&self) -> u64 {
    900
  }

  /// Optional working directory.
  fn cwd(&self) -> Option<&Path> {
    None
  }

  /// Extra environment variables for the process.
  fn env(&self) -> HashMap<String, String> {
    HashMap::new()
  }

  /// Argv used to list models from the CLI (stdout is parsed). Empty = skip probe.
  fn list_models_args(&self) -> Vec<String> {
    Vec::new()
  }

  /// Whether the provider binary is available on this machine.
  fn is_installed(&self) -> bool {
    resolve_binary(self.command()).is_some()
  }

  /// Resolved absolute path to the binary, if found.
  fn resolve_binary(&self) -> Option<String> {
    resolve_binary(self.command())
  }

  /// Build the process specification for a prompt.
  fn spawn_spec(&self, req: &PromptRequest) -> SpawnSpec;

  // -----------------------------------------------------------------------
  // Auth (each provider's own struct inside AuthEntry → auth/{provider}.json)
  // -----------------------------------------------------------------------

  /// How this provider authenticates. Default: no auth.
  fn auth_method(&self) -> AuthMethod {
    AuthMethod::None
  }

  /// Whether interactive / import auth is supported.
  fn supports_auth(&self) -> bool {
    !matches!(self.auth_method(), AuthMethod::None)
  }

  /// Load this provider's stored auth records (original structs, wrapped).
  ///
  /// Default: empty (providers that support auth override this).
  fn load_auth(&self, _store: &AuthStore) -> AppResult<Vec<AuthEntry>> {
    Ok(Vec::new())
  }

  /// Persist a provider-owned auth record into `auth/{provider}.json`.
  ///
  /// Default: unsupported.
  fn save_auth(&self, _store: &AuthStore, _entry: &AuthEntry) -> AppResult<PathBuf> {
    Err(AppError::BadRequest(format!(
      "provider `{}` does not implement save_auth",
      self.kind()
    )))
  }

  /// Remove this provider's stored credentials. When `account` is set, only that key.
  fn clear_auth(&self, store: &AuthStore, account: Option<&str>) -> AppResult<usize> {
    store.remove(self.kind(), account)
  }

  /// Interactive login / credential import. Default: unsupported.
  fn login(
    &self,
    _store: &AuthStore,
    _opts: LoginOptions,
  ) -> impl std::future::Future<Output = AppResult<AuthEntry>> + Send {
    let kind = self.kind();
    async move {
      Err(AppError::BadRequest(format!(
        "provider `{kind}` does not support auth login"
      )))
    }
  }

  /// Refresh an expired (or near-expiry) credential and re-save.
  /// Default: return the entry unchanged.
  fn refresh_auth(
    &self,
    _store: &AuthStore,
    entry: &AuthEntry,
  ) -> impl std::future::Future<Output = AppResult<AuthEntry>> + Send {
    let entry = entry.clone();
    async move { Ok(entry) }
  }

  /// Load credentials and refresh the first one if it needs it.
  fn ensure_auth(
    &self,
    store: &AuthStore,
  ) -> impl std::future::Future<Output = AppResult<AuthEntry>> + Send {
    async move {
      let list = self.load_auth(store)?;
      let Some(entry) = list.into_iter().next() else {
        return Err(AppError::Unauthorized(format!(
          "no credentials for provider `{}`; run `teapotx auth login {}`",
          self.kind(),
          self.kind()
        )));
      };
      if entry.needs_refresh(chrono::Duration::minutes(5)) {
        let refreshed = self.refresh_auth(store, &entry).await?;
        let _ = self.save_auth(store, &refreshed)?;
        Ok(refreshed)
      } else {
        Ok(entry)
      }
    }
  }
}

/// Executes a [`SpawnSpec`] and streams [`ProviderEvent`]s.
pub trait ProviderExecutor: Send + Sync {
  fn execute(
    &self,
    provider_name: String,
    spec: SpawnSpec,
  ) -> impl std::future::Future<Output = AppResult<ProviderSession>> + Send;
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Resolve the provider binary path if the command is available.
pub fn resolve_binary(command: &str) -> Option<String> {
  let path = Path::new(command);
  if path.is_absolute() || command.contains('/') || command.contains('\\') {
    if path.is_file() {
      return Some(path.display().to_string());
    }
    return None;
  }
  which::which(command).ok().map(|p| p.display().to_string())
}

/// Apply `{prompt}`, `{system}`, and `{model}` placeholders in argv templates.
pub fn expand_args(args: &[&str], req: &PromptRequest) -> Vec<String> {
  let system = req.system_str();
  let model = req.model.as_deref().unwrap_or("");
  args
    .iter()
    .map(|a| {
      a.replace("{prompt}", &req.prompt)
        .replace("{system}", system)
        .replace("{model}", model)
    })
    .collect()
}

/// Build stdin payload: optional system block + prompt.
pub fn stdin_prompt(req: &PromptRequest) -> String {
  let mut payload = String::new();
  if let Some(sys) = req.system.as_deref() {
    if !sys.is_empty() {
      payload.push_str(sys);
      payload.push_str("\n\n");
    }
  }
  payload.push_str(&req.prompt);
  payload
}

/// Build a single prompt string from chat-style messages.
///
/// Single-turn chats pass the user text through as-is.
/// Multi-turn history uses a compact dialogue transcript.
pub fn flatten_messages(
  system: Option<&str>,
  messages: &[(String, String)],
) -> (Option<String>, String) {
  let mut system_parts: Vec<String> = Vec::new();
  if let Some(s) = system {
    if !s.is_empty() {
      system_parts.push(s.to_string());
    }
  }

  let mut turns: Vec<(String, String)> = Vec::new();
  for (role, content) in messages {
    match role.as_str() {
      "system" => {
        if !content.is_empty() {
          system_parts.push(content.clone());
        }
      }
      "user" | "human" | "assistant" | "model" | _ => {
        if !content.is_empty() {
          turns.push((role.clone(), content.clone()));
        }
      }
    }
  }

  let body = if turns.len() == 1 && matches!(turns[0].0.as_str(), "user" | "human") {
    turns[0].1.clone()
  } else {
    let mut body = String::new();
    for (role, content) in &turns {
      if !body.is_empty() {
        body.push_str("\n\n");
      }
      match role.as_str() {
        "user" | "human" => {
          body.push_str("User: ");
          body.push_str(content);
        }
        "assistant" | "model" => {
          body.push_str("Assistant: ");
          body.push_str(content);
        }
        other => {
          body.push_str(other);
          body.push_str(": ");
          body.push_str(content);
        }
      }
    }
    if !body.is_empty() && !body.ends_with("Assistant: ") {
      body.push_str("\n\nAssistant:");
    }
    body
  };

  let system = if system_parts.is_empty() {
    None
  } else {
    Some(system_parts.join("\n\n"))
  };
  (system, body)
}
