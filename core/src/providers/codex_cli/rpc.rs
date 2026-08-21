//! JSON-RPC 2.0 client for `codex app-server` over stdio (JSONL, no `jsonrpc` field).

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
pub struct RpcError {
  pub code: i64,
  pub message: String,
}

impl RpcError {
  fn into_app(self) -> AppError {
    AppError::ProviderFailed(format!(
      "codex-cli: JSON-RPC error {}: {}",
      self.code, self.message
    ))
  }
}

#[derive(Debug, Clone)]
pub struct Notification {
  pub method: String,
  pub params: Value,
}

/// Live `codex app-server --stdio` child plus JSON-RPC mux.
pub struct AppServerSession {
  child: Child,
  stdin: Arc<Mutex<ChildStdin>>,
  next_id: AtomicU64,
  pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, RpcError>>>>>,
  notifications: mpsc::UnboundedReceiver<Notification>,
}

impl AppServerSession {
  pub async fn spawn() -> AppResult<Self> {
    let program = resolve_binary("codex").ok_or_else(|| {
      AppError::ProviderBinaryMissing(
        "codex: install the Codex CLI and ensure `codex` is on PATH".into(),
      )
    })?;

    let mut cmd = Command::new(&program);
    cmd
      .args(["app-server", "--stdio"])
      .env("PATH", augmented_path())
      .stdin(Stdio::piped())
      .stdout(Stdio::piped())
      .stderr(Stdio::piped())
      .kill_on_drop(true);

    let mut child = cmd.spawn().map_err(|e| {
      AppError::ProviderFailed(format!(
        "codex-cli: spawn `{program} app-server --stdio`: {e}"
      ))
    })?;

    let stdin = child
      .stdin
      .take()
      .ok_or_else(|| AppError::Internal("codex-cli: child stdin missing after spawn".into()))?;
    let stdout = child
      .stdout
      .take()
      .ok_or_else(|| AppError::Internal("codex-cli: child stdout missing after spawn".into()))?;
    let stderr = child.stderr.take();

    if let Some(stderr) = stderr {
      tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
          if !line.is_empty() {
            debug!(target: "codex_cli", "{line}");
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

  pub async fn initialize(&self) -> AppResult<Value> {
    let params = json!({
      "clientInfo": {
        "name": CLIENT_NAME,
        "title": CLIENT_TITLE,
        "version": env!("CARGO_PKG_VERSION"),
      }
    });
    let result = self.request("initialize", params).await?;
    self.notify("initialized", Value::Null).await?;
    Ok(result)
  }

  pub async fn request(&self, method: &str, params: Value) -> AppResult<Value> {
    let id = self.next_id.fetch_add(1, Ordering::Relaxed);
    let (tx, rx) = oneshot::channel();
    {
      let mut pending = self.pending.lock().await;
      pending.insert(id, tx);
    }
    let msg = json!({
      "method": method,
      "id": id,
      "params": params,
    });
    if let Err(e) = write_line(&self.stdin, &msg).await {
      let mut pending = self.pending.lock().await;
      pending.remove(&id);
      return Err(e);
    }
    match rx.await {
      Ok(Ok(value)) => Ok(value),
      Ok(Err(err)) => Err(err.into_app()),
      Err(_) => Err(AppError::ProviderFailed(
        "codex-cli: app-server closed before JSON-RPC reply".into(),
      )),
    }
  }

  async fn notify(&self, method: &str, params: Value) -> AppResult<()> {
    let msg = if params.is_null() {
      json!({ "method": method })
    } else {
      json!({ "method": method, "params": params })
    };
    write_line(&self.stdin, &msg).await
  }

  pub async fn recv_notification(&mut self) -> Option<Notification> {
    self.notifications.recv().await
  }

  /// True while the child is still running.
  pub fn is_alive(&mut self) -> bool {
    match self.child.try_wait() {
      Ok(None) => true,
      Ok(Some(status)) => {
        debug!(?status, "codex-cli: app-server process exited");
        false
      }
      Err(error) => {
        debug!(%error, "codex-cli: app-server wait failed");
        false
      }
    }
  }
}

impl Drop for AppServerSession {
  fn drop(&mut self) {
    let _ = self.child.start_kill();
  }
}

async fn write_line(stdin: &Arc<Mutex<ChildStdin>>, msg: &Value) -> AppResult<()> {
  let mut line = serde_json::to_vec(msg)
    .map_err(|e| AppError::Internal(format!("codex-cli: encode JSON-RPC: {e}")))?;
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
        warn!(error = %e, raw = %truncate(line, 240), "codex-cli: skip malformed JSON-RPC line");
        continue;
      }
    };
    match classify(incoming) {
      Classified::Response { id, result } => {
        if let Some(tx) = pending.lock().await.remove(&id) {
          let _ = tx.send(result);
        } else {
          debug!(id, "codex-cli: response for unknown id");
        }
      }
      Classified::Notification { method, params } => {
        if notes.send(Notification { method, params }).is_err() {
          break;
        }
      }
      Classified::ServerRequest { id, method, params } => {
        let reply = auto_reply(&method, &params);
        debug!(method, "codex-cli: auto-replying to server request");
        let msg = json!({ "id": id, "result": reply });
        if let Err(e) = write_line(&stdin, &msg).await {
          warn!(error = %e, "codex-cli: failed to reply to server request");
        }
      }
      Classified::Ignored => {}
    }
  }

  let mut pending = pending.lock().await;
  for (_, tx) in pending.drain() {
    let _ = tx.send(Err(RpcError {
      code: -1,
      message: "app-server stdout closed".into(),
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

fn auto_reply(method: &str, _params: &Value) -> Value {
  if method.contains("requestApproval") {
    json!({ "decision": "accept" })
  } else if method.ends_with("requestUserInput") {
    json!({ "action": "cancel", "content": Value::Null })
  } else {
    json!({})
  }
}

fn truncate(s: &str, max: usize) -> String {
  if s.len() <= max {
    s.to_string()
  } else {
    format!("{}…", &s[..max])
  }
}

/// Thread id on a notification, when the payload includes one.
pub fn notification_thread_id(note: &Notification) -> Option<&str> {
  let p = &note.params;
  p.get("threadId")
    .and_then(Value::as_str)
    .or_else(|| {
      p.get("thread")
        .and_then(|t| t.get("id"))
        .and_then(Value::as_str)
    })
    .or_else(|| {
      p.get("turn")
        .and_then(|t| t.get("threadId"))
        .and_then(Value::as_str)
    })
}

/// Pull streamed assistant text out of an `item/agentMessage/delta` params object.
pub fn extract_text_delta(params: &Value) -> Option<String> {
  let delta = params.get("delta")?;
  if let Some(s) = delta.as_str() {
    if s.is_empty() {
      return None;
    }
    return Some(s.to_string());
  }
  if let Some(s) = delta.get("text").and_then(Value::as_str) {
    if s.is_empty() {
      return None;
    }
    return Some(s.to_string());
  }
  None
}

/// Full agent message text from `item/completed` (or `item/started`) params.
pub fn extract_agent_message_text(params: &Value) -> Option<String> {
  let item = params.get("item")?;
  let kind = item
    .get("type")
    .or_else(|| item.get("itemType"))
    .and_then(Value::as_str)
    .unwrap_or("");
  if !kind.is_empty() && kind != "agentMessage" && kind != "agent_message" {
    return None;
  }
  item
    .get("text")
    .and_then(Value::as_str)
    .filter(|s| !s.is_empty())
    .map(str::to_string)
}

pub fn turn_status(params: &Value) -> Option<&str> {
  params
    .get("turn")
    .and_then(|t| t.get("status"))
    .and_then(Value::as_str)
}

pub fn turn_error_message(params: &Value) -> Option<String> {
  let err = params.get("turn").and_then(|t| t.get("error"))?;
  if err.is_null() {
    return None;
  }
  err
    .get("message")
    .and_then(Value::as_str)
    .filter(|s| !s.is_empty())
    .map(str::to_string)
    .or_else(|| Some(err.to_string()))
}

pub fn token_usage(params: &Value) -> Option<(u32, u32)> {
  let usage = params
    .get("tokenUsage")
    .or_else(|| params.get("usage"))
    .or_else(|| {
      params
        .get("turn")
        .and_then(|t| t.get("tokenUsage").or_else(|| t.get("usage")))
    })?;
  let prompt = usage
    .get("inputTokens")
    .or_else(|| usage.get("input_tokens"))
    .or_else(|| usage.get("promptTokens"))
    .and_then(Value::as_u64)
    .unwrap_or(0) as u32;
  let completion = usage
    .get("outputTokens")
    .or_else(|| usage.get("output_tokens"))
    .or_else(|| usage.get("completionTokens"))
    .and_then(Value::as_u64)
    .unwrap_or(0) as u32;
  Some((prompt, completion))
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn classify_response_and_notification() {
    let resp: Incoming = serde_json::from_str(r#"{"id":3,"result":{"ok":true}}"#).unwrap();
    match classify(resp) {
      Classified::Response { id, result } => {
        assert_eq!(id, 3);
        assert!(result.is_ok());
      }
      other => panic!("unexpected {other:?}"),
    }

    let note: Incoming = serde_json::from_str(
      r#"{"method":"turn/completed","params":{"turn":{"status":"completed"}}}"#,
    )
    .unwrap();
    match classify(note) {
      Classified::Notification { method, .. } => assert_eq!(method, "turn/completed"),
      other => panic!("unexpected {other:?}"),
    }
  }

  #[test]
  fn classify_server_request() {
    let req: Incoming = serde_json::from_str(
      r#"{"method":"item/commandExecution/requestApproval","id":9,"params":{}}"#,
    )
    .unwrap();
    match classify(req) {
      Classified::ServerRequest { method, .. } => {
        assert!(method.contains("requestApproval"));
        assert_eq!(
          auto_reply(&method, &Value::Null),
          json!({"decision":"accept"})
        );
      }
      other => panic!("unexpected {other:?}"),
    }
  }

  #[test]
  fn extract_delta_string_or_object() {
    let a = json!({"delta":"hi"});
    assert_eq!(extract_text_delta(&a).as_deref(), Some("hi"));
    let b = json!({"delta":{"text":"there"}});
    assert_eq!(extract_text_delta(&b).as_deref(), Some("there"));
  }

  #[test]
  fn extract_completed_agent_text() {
    let params = json!({"item":{"type":"agentMessage","text":"done"}});
    assert_eq!(extract_agent_message_text(&params).as_deref(), Some("done"));
  }
}
