//! Grok Build `agent stdio` ACP client over JSON-RPC 2.0 (NDJSON).
//!
//! Spawn:
//! ```text
//! grok agent --always-approve --no-leader -m <id> stdio
//! ```
//!
//! Stdin = JSON-RPC requests (`initialize`, `authenticate`, `session/new`,
//! `session/prompt`). Stdout = JSON-RPC responses plus `session/update`
//! notifications. Auth is owned by the local CLI (`grok login`); Teapot
//! stores nothing.

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{Mutex, mpsc, oneshot};
use tracing::{debug, warn};

use crate::error::{AppError, AppResult};
use crate::providers::traits::{augmented_path, resolve_binary};

const CLIENT_NAME: &str = "teapot";
const CLIENT_TITLE: &str = "Teapot";

#[derive(Debug, Clone)]
struct RpcError {
  code: i64,
  message: String,
}

impl RpcError {
  fn into_app(self) -> AppError {
    AppError::ProviderFailed(format!(
      "grok-cli: JSON-RPC error {}: {}",
      self.code, self.message
    ))
  }
}

#[derive(Debug, Clone)]
pub struct Notification {
  pub method: String,
  pub params: Value,
}

/// One parsed `session/update` (or equivalent) from Grok ACP.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamEvent {
  TextDelta(String),
  ThinkingDelta(String),
  Usage {
    prompt_tokens: u32,
    completion_tokens: u32,
  },
  Ignored,
}

/// Live `grok agent stdio` child speaking ACP JSON-RPC on stdio.
pub struct AcpSession {
  child: Child,
  stdin: Arc<Mutex<ChildStdin>>,
  next_id: AtomicU64,
  pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, RpcError>>>>>,
  notifications: mpsc::UnboundedReceiver<Notification>,
}

impl AcpSession {
  pub async fn spawn(model: &str) -> AppResult<Self> {
    let program = resolve_binary("grok").ok_or_else(|| {
      AppError::ProviderBinaryMissing(
        "grok: install Grok Build CLI and ensure `grok` is on PATH".into(),
      )
    })?;

    let mut args: Vec<String> = vec![
      "agent".into(),
      "--always-approve".into(),
      "--no-leader".into(),
    ];
    if !model.trim().is_empty() {
      args.push("-m".into());
      args.push(model.to_string());
    }
    args.push("stdio".into());

    let mut cmd = Command::new(&program);
    cmd
      .args(&args)
      .env("PATH", augmented_path())
      .env("GROK_DISABLE_AUTOUPDATER", "1")
      .stdin(Stdio::piped())
      .stdout(Stdio::piped())
      .stderr(Stdio::piped())
      .kill_on_drop(true);
    if let Ok(cwd) = std::env::current_dir() {
      cmd.current_dir(cwd);
    }

    let mut child = cmd.spawn().map_err(|e| {
      AppError::ProviderFailed(format!(
        "grok-cli: spawn `{program} agent --always-approve stdio`: {e}"
      ))
    })?;

    let stdin = child
      .stdin
      .take()
      .ok_or_else(|| AppError::Internal("grok-cli: child stdin missing after spawn".into()))?;
    let stdout = child
      .stdout
      .take()
      .ok_or_else(|| AppError::Internal("grok-cli: child stdout missing after spawn".into()))?;
    let stderr = child.stderr.take();

    if let Some(stderr) = stderr {
      tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
          if !line.is_empty() {
            debug!(target: "grok_cli", "{line}");
          }
        }
      });
    }

    let pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, RpcError>>>>> =
      Arc::new(Mutex::new(HashMap::new()));
    let stdin = Arc::new(Mutex::new(stdin));
    let (note_tx, notifications) = mpsc::unbounded_channel();

    let pending_r = Arc::clone(&pending);
    let stdin_r = Arc::clone(&stdin);
    tokio::spawn(async move {
      read_loop(stdout, pending_r, stdin_r, note_tx).await;
    });

    Ok(Self {
      child,
      stdin,
      next_id: AtomicU64::new(1),
      pending,
      notifications,
    })
  }

  /// Handshake: initialize + authenticate with the local Grok login.
  pub async fn handshake(&self) -> AppResult<Value> {
    let init = self
      .request(
        "initialize",
        json!({
          "protocolVersion": 1,
          "clientInfo": {
            "name": CLIENT_NAME,
            "title": CLIENT_TITLE,
            "version": env!("CARGO_PKG_VERSION"),
          },
          "clientCapabilities": {
            "fs": { "readTextFile": false, "writeTextFile": false },
            "terminal": false
          }
        }),
      )
      .await?;
    self.authenticate(&init).await?;
    Ok(init)
  }

  async fn authenticate(&self, init: &Value) -> AppResult<()> {
    let methods: Vec<String> = init
      .get("authMethods")
      .and_then(Value::as_array)
      .map(|rows| {
        rows
          .iter()
          .filter_map(|row| row.get("id").and_then(Value::as_str).map(str::to_string))
          .collect()
      })
      .unwrap_or_default();
    if methods.is_empty() {
      return Ok(());
    }
    let has_api_key = std::env::var("XAI_API_KEY")
      .map(|v| !v.trim().is_empty())
      .unwrap_or(false);
    let method_id = if has_api_key && methods.iter().any(|m| m == "xai.api_key") {
      "xai.api_key"
    } else if methods.iter().any(|m| m == "cached_token") {
      "cached_token"
    } else {
      methods.first().map(String::as_str).unwrap_or("")
    };
    if method_id.is_empty() {
      return Err(AppError::Unauthorized(
        "grok-cli: run `grok login` first, or set XAI_API_KEY".into(),
      ));
    }
    self
      .request(
        "authenticate",
        json!({
          "methodId": method_id,
          "_meta": { "headless": true }
        }),
      )
      .await
      .map_err(|e| {
        AppError::Unauthorized(format!(
          "{e}; run `grok login` (or set XAI_API_KEY) and retry"
        ))
      })?;
    Ok(())
  }

  pub async fn new_session(&self, system: Option<&str>) -> AppResult<String> {
    let cwd = std::env::current_dir()
      .ok()
      .and_then(|p| p.to_str().map(str::to_string))
      .unwrap_or_else(|| ".".into());
    let mut meta = json!({ "yoloMode": true });
    if let Some(sys) = system.map(str::trim).filter(|s| !s.is_empty()) {
      meta["rules"] = json!(sys);
    }
    let result = self
      .request(
        "session/new",
        json!({
          "cwd": cwd,
          "mcpServers": [],
          "_meta": meta
        }),
      )
      .await?;
    result
      .get("sessionId")
      .and_then(Value::as_str)
      .filter(|s| !s.is_empty())
      .map(str::to_string)
      .ok_or_else(|| AppError::ProviderFailed("grok-cli: session/new missing sessionId".into()))
  }

  pub async fn start_prompt(&self, session_id: &str, prompt: &str) -> AppResult<PromptWaiter> {
    let rx = self
      .start_request(
        "session/prompt",
        json!({
          "sessionId": session_id,
          "prompt": [{ "type": "text", "text": prompt }]
        }),
      )
      .await?;
    Ok(PromptWaiter { rx })
  }

  pub async fn recv_notification(&mut self) -> Option<Notification> {
    self.notifications.recv().await
  }

  pub async fn request(&self, method: &str, params: Value) -> AppResult<Value> {
    let rx = self.start_request(method, params).await?;
    match rx.await {
      Ok(Ok(value)) => Ok(value),
      Ok(Err(err)) => Err(err.into_app()),
      Err(_) => Err(AppError::ProviderFailed(
        "grok-cli: agent closed before JSON-RPC reply (run `grok login` if needed)".into(),
      )),
    }
  }

  async fn start_request(
    &self,
    method: &str,
    params: Value,
  ) -> AppResult<oneshot::Receiver<Result<Value, RpcError>>> {
    let id = self.next_id.fetch_add(1, Ordering::Relaxed);
    let (tx, rx) = oneshot::channel();
    {
      let mut pending = self.pending.lock().await;
      pending.insert(id, tx);
    }
    let msg = json!({
      "jsonrpc": "2.0",
      "id": id,
      "method": method,
      "params": params,
    });
    if let Err(e) = write_line(&self.stdin, &msg).await {
      let mut pending = self.pending.lock().await;
      pending.remove(&id);
      return Err(e);
    }
    Ok(rx)
  }
}

/// Outstanding `session/prompt` JSON-RPC reply.
pub struct PromptWaiter {
  rx: oneshot::Receiver<Result<Value, RpcError>>,
}

impl PromptWaiter {
  pub async fn wait(self) -> AppResult<Value> {
    match self.rx.await {
      Ok(Ok(value)) => Ok(value),
      Ok(Err(err)) => Err(err.into_app()),
      Err(_) => Err(AppError::ProviderFailed(
        "grok-cli: agent closed before session/prompt reply (run `grok login` if needed)".into(),
      )),
    }
  }
}

impl Drop for AcpSession {
  fn drop(&mut self) {
    let _ = self.child.start_kill();
  }
}

async fn write_line(stdin: &Arc<Mutex<ChildStdin>>, msg: &Value) -> AppResult<()> {
  let mut line = serde_json::to_vec(msg)
    .map_err(|e| AppError::Internal(format!("grok-cli: encode JSON-RPC: {e}")))?;
  line.push(b'\n');
  let mut guard = stdin.lock().await;
  guard.write_all(&line).await?;
  guard.flush().await?;
  Ok(())
}

async fn read_loop(
  stdout: tokio::process::ChildStdout,
  pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, RpcError>>>>>,
  stdin: Arc<Mutex<ChildStdin>>,
  notes: mpsc::UnboundedSender<Notification>,
) {
  let mut lines = BufReader::new(stdout).lines();
  while let Ok(Some(line)) = lines.next_line().await {
    let line = line.trim();
    if line.is_empty() {
      continue;
    }
    let incoming = match serde_json::from_str::<Incoming>(line) {
      Ok(v) => v,
      Err(e) => {
        warn!(error = %e, raw = %truncate(line, 240), "grok-cli: skip malformed JSON-RPC line");
        continue;
      }
    };
    match classify(incoming) {
      Classified::Response { id, result } => {
        if let Some(tx) = pending.lock().await.remove(&id) {
          let _ = tx.send(result);
        } else {
          debug!(id, "grok-cli: response for unknown id");
        }
      }
      Classified::Notification { method, params } => {
        if notes.send(Notification { method, params }).is_err() {
          break;
        }
      }
      Classified::ServerRequest { id, method, params } => {
        let reply = auto_reply(&method, &params);
        debug!(method, "grok-cli: auto-replying to server request");
        let msg = json!({
          "jsonrpc": "2.0",
          "id": id,
          "result": reply
        });
        if let Err(e) = write_line(&stdin, &msg).await {
          warn!(error = %e, "grok-cli: failed to reply to server request");
        }
      }
      Classified::Ignored => {}
    }
  }

  let mut pending = pending.lock().await;
  for (_, tx) in pending.drain() {
    let _ = tx.send(Err(RpcError {
      code: -1,
      message: "agent stdout closed".into(),
    }));
  }
}

#[derive(Debug, Deserialize)]
struct Incoming {
  #[serde(default)]
  id: Option<Value>,
  #[serde(default)]
  method: Option<String>,
  #[serde(default)]
  params: Option<Value>,
  #[serde(default)]
  result: Option<Value>,
  #[serde(default)]
  error: Option<IncomingError>,
}

#[derive(Debug, Deserialize)]
struct IncomingError {
  #[serde(default)]
  code: i64,
  #[serde(default)]
  message: String,
}

#[derive(Debug)]
enum Classified {
  Response {
    id: u64,
    result: Result<Value, RpcError>,
  },
  Notification {
    method: String,
    params: Value,
  },
  ServerRequest {
    id: Value,
    method: String,
    params: Value,
  },
  Ignored,
}

fn classify(msg: Incoming) -> Classified {
  let method = msg.method.filter(|s| !s.is_empty());
  match (method, msg.id) {
    (Some(method), Some(id)) => Classified::ServerRequest {
      id,
      method,
      params: msg.params.unwrap_or(Value::Null),
    },
    (Some(method), None) => Classified::Notification {
      method,
      params: msg.params.unwrap_or(Value::Null),
    },
    (None, Some(id)) => {
      let Some(num) = json_id_u64(&id) else {
        return Classified::Ignored;
      };
      let result = if let Some(err) = msg.error {
        Err(RpcError {
          code: err.code,
          message: err.message,
        })
      } else {
        Ok(msg.result.unwrap_or(Value::Null))
      };
      Classified::Response { id: num, result }
    }
    (None, None) => Classified::Ignored,
  }
}

fn json_id_u64(id: &Value) -> Option<u64> {
  match id {
    Value::Number(n) => n.as_u64().or_else(|| n.as_i64().map(|v| v as u64)),
    Value::String(s) => s.parse().ok(),
    _ => None,
  }
}

fn auto_reply(method: &str, params: &Value) -> Value {
  if method == "session/request_permission" || method.ends_with("request_permission") {
    let option_id = pick_allow_option(params);
    json!({
      "outcome": {
        "outcome": "selected",
        "optionId": option_id
      }
    })
  } else {
    json!({})
  }
}

fn pick_allow_option(params: &Value) -> String {
  let options = params
    .get("options")
    .and_then(Value::as_array)
    .cloned()
    .unwrap_or_default();
  let prefer = |kind: &str| {
    options.iter().find_map(|opt| {
      let k = opt.get("kind").and_then(Value::as_str).unwrap_or("");
      if k == kind {
        opt
          .get("optionId")
          .and_then(Value::as_str)
          .map(str::to_string)
      } else {
        None
      }
    })
  };
  prefer("allow_always")
    .or_else(|| prefer("allow_once"))
    .or_else(|| {
      options
        .first()
        .and_then(|opt| opt.get("optionId").and_then(Value::as_str))
        .map(str::to_string)
    })
    .unwrap_or_else(|| "allow-once".into())
}

fn truncate(s: &str, max: usize) -> String {
  if s.len() <= max {
    s.to_string()
  } else {
    format!("{}…", &s[..max])
  }
}

/// Classify a `session/update` notification into a stream event.
pub fn parse_session_update(params: &Value) -> StreamEvent {
  let update = params.get("update").unwrap_or(params);
  let kind = update
    .get("sessionUpdate")
    .or_else(|| update.get("session_update"))
    .and_then(Value::as_str)
    .unwrap_or("");
  match kind {
    "agent_message_chunk" => match content_text(update.get("content")) {
      Some(text) => StreamEvent::TextDelta(text),
      None => StreamEvent::Ignored,
    },
    "agent_thought_chunk" => match content_text(update.get("content")) {
      Some(text) => StreamEvent::ThinkingDelta(text),
      None => StreamEvent::Ignored,
    },
    "usage_update" => {
      let (prompt, completion) = usage_tokens(update);
      if prompt == 0 && completion == 0 {
        StreamEvent::Ignored
      } else {
        StreamEvent::Usage {
          prompt_tokens: prompt,
          completion_tokens: completion,
        }
      }
    }
    _ => StreamEvent::Ignored,
  }
}

/// True when this notification carries prompt-turn stream data.
pub fn is_session_update(method: &str) -> bool {
  matches!(
    method,
    "session/update" | "x.ai/session/update" | "session_update"
  )
}

pub fn content_text(content: Option<&Value>) -> Option<String> {
  let content = content?;
  if let Some(s) = content.as_str() {
    return nonempty(s);
  }
  if let Some(s) = content.get("text").and_then(Value::as_str) {
    return nonempty(s);
  }
  if let Some(arr) = content.as_array() {
    let mut out = String::new();
    for block in arr {
      if let Some(t) = block
        .as_str()
        .or_else(|| block.get("text").and_then(Value::as_str))
      {
        out.push_str(t);
      }
    }
    return nonempty(&out);
  }
  None
}

pub fn usage_tokens(value: &Value) -> (u32, u32) {
  let usage = value.get("usage").unwrap_or(value);
  let prompt = u32_field(
    usage,
    &[
      "input_tokens",
      "inputTokens",
      "prompt_tokens",
      "promptTokens",
    ],
  )
  .or_else(|| u32_field(value, &["used"]))
  .unwrap_or(0);
  let completion = u32_field(
    usage,
    &[
      "output_tokens",
      "outputTokens",
      "completion_tokens",
      "completionTokens",
    ],
  )
  .unwrap_or(0);
  (prompt, completion)
}

pub fn result_text(result: &Value) -> Option<String> {
  result
    .get("text")
    .and_then(Value::as_str)
    .and_then(nonempty)
}

pub fn result_stop_reason(result: &Value) -> Option<&str> {
  result
    .get("stopReason")
    .or_else(|| result.get("stop_reason"))
    .and_then(Value::as_str)
}

fn nonempty(s: &str) -> Option<String> {
  if s.is_empty() {
    None
  } else {
    Some(s.to_string())
  }
}

fn u32_field(value: &Value, keys: &[&str]) -> Option<u32> {
  for key in keys {
    if let Some(n) = value.get(*key).and_then(Value::as_u64) {
      return Some(n as u32);
    }
  }
  None
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn classify_response_and_notification() {
    let resp: Incoming =
      serde_json::from_str(r#"{"jsonrpc":"2.0","id":3,"result":{"sessionId":"s"}}"#).unwrap();
    match classify(resp) {
      Classified::Response { id, result } => {
        assert_eq!(id, 3);
        assert!(result.is_ok());
      }
      other => panic!("unexpected {other:?}"),
    }

    let note: Incoming = serde_json::from_str(
      r#"{"jsonrpc":"2.0","method":"session/update","params":{"update":{"sessionUpdate":"agent_message_chunk"}}}"#,
    )
    .unwrap();
    match classify(note) {
      Classified::Notification { method, .. } => assert_eq!(method, "session/update"),
      other => panic!("unexpected {other:?}"),
    }
  }

  #[test]
  fn parse_text_and_thought_chunks() {
    let text = json!({
      "sessionId": "s",
      "update": {
        "sessionUpdate": "agent_message_chunk",
        "content": { "type": "text", "text": "Hello" }
      }
    });
    assert_eq!(
      parse_session_update(&text),
      StreamEvent::TextDelta("Hello".into())
    );

    let think = json!({
      "update": {
        "sessionUpdate": "agent_thought_chunk",
        "content": { "text": "hmm" }
      }
    });
    assert_eq!(
      parse_session_update(&think),
      StreamEvent::ThinkingDelta("hmm".into())
    );
  }

  #[test]
  fn pick_allow_always_then_once() {
    let params = json!({
      "options": [
        { "optionId": "reject-once", "kind": "reject_once" },
        { "optionId": "allow-once", "kind": "allow_once" },
        { "optionId": "allow-always", "kind": "allow_always" }
      ]
    });
    assert_eq!(pick_allow_option(&params), "allow-always");

    let once = json!({
      "options": [
        { "optionId": "allow-once", "kind": "allow_once" }
      ]
    });
    assert_eq!(pick_allow_option(&once), "allow-once");
  }

  #[test]
  fn permission_auto_reply_shape() {
    let params = json!({
      "options": [{ "optionId": "allow-once", "kind": "allow_once" }]
    });
    assert_eq!(
      auto_reply("session/request_permission", &params),
      json!({
        "outcome": { "outcome": "selected", "optionId": "allow-once" }
      })
    );
  }
}
