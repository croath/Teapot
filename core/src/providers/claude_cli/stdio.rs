//! Claude Code `-p` stream-json client over stdio (NDJSON).
//!
//! Spawn:
//! ```text
//! claude -p --output-format stream-json --input-format stream-json
//!        --verbose --include-partial-messages --dangerously-skip-permissions
//!        --model <id>
//! ```
//!
//! Stdin = JSONL user messages (and control replies). Stdout = NDJSON events.
//! Auth is owned by the local CLI (`claude auth login`); Teapot stores nothing.

use std::process::Stdio;
use std::sync::Arc;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::Mutex;
use tracing::{debug, warn};

use crate::error::{AppError, AppResult};
use crate::providers::traits::{augmented_path, resolve_binary};

/// One parsed stdout line from Claude Code stream-json.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamEvent {
  Init {
    session_id: Option<String>,
    model: Option<String>,
  },
  TextDelta(String),
  ThinkingDelta(String),
  AssistantText(String),
  Result {
    content: String,
    is_error: bool,
    error: Option<String>,
    prompt_tokens: u32,
    completion_tokens: u32,
  },
  ControlRequest {
    request_id: String,
  },
  Ignored,
}

/// Live `claude -p` child speaking stream-json on stdio.
pub struct StreamJsonSession {
  child: Child,
  stdin: Arc<Mutex<Option<ChildStdin>>>,
  stdout: BufReader<tokio::process::ChildStdout>,
}

impl StreamJsonSession {
  pub async fn spawn(model: &str, system: Option<&str>) -> AppResult<Self> {
    let program = resolve_binary("claude").ok_or_else(|| {
      AppError::ProviderBinaryMissing(
        "claude: install Claude Code and ensure `claude` is on PATH".into(),
      )
    })?;

    let mut args: Vec<String> = vec![
      "-p".into(),
      "--output-format".into(),
      "stream-json".into(),
      "--input-format".into(),
      "stream-json".into(),
      "--verbose".into(),
      "--include-partial-messages".into(),
      "--dangerously-skip-permissions".into(),
    ];
    if !model.trim().is_empty() {
      args.push("--model".into());
      args.push(model.to_string());
    }
    if let Some(sys) = system.map(str::trim).filter(|s| !s.is_empty()) {
      args.push("--append-system-prompt".into());
      args.push(sys.to_string());
    }

    let mut cmd = Command::new(&program);
    cmd
      .args(&args)
      .env("PATH", augmented_path())
      .env("CLAUDE_CODE_ENTRYPOINT", "teapot")
      .stdin(Stdio::piped())
      .stdout(Stdio::piped())
      .stderr(Stdio::piped())
      .kill_on_drop(true);
    if let Ok(cwd) = std::env::current_dir() {
      cmd.current_dir(cwd);
    }

    let mut child = cmd.spawn().map_err(|e| {
      AppError::ProviderFailed(format!(
        "claude-cli: spawn `{program} -p --output-format stream-json`: {e}"
      ))
    })?;

    let stdin = child
      .stdin
      .take()
      .ok_or_else(|| AppError::Internal("claude-cli: child stdin missing after spawn".into()))?;
    let stdout = child
      .stdout
      .take()
      .ok_or_else(|| AppError::Internal("claude-cli: child stdout missing after spawn".into()))?;
    let stderr = child.stderr.take();

    if let Some(stderr) = stderr {
      tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
          if !line.is_empty() {
            debug!(target: "claude_cli", "{line}");
          }
        }
      });
    }

    Ok(Self {
      child,
      stdin: Arc::new(Mutex::new(Some(stdin))),
      stdout: BufReader::new(stdout),
    })
  }

  pub async fn send_user(&self, text: &str) -> AppResult<()> {
    let msg = json!({
      "type": "user",
      "message": {
        "role": "user",
        "content": [{ "type": "text", "text": text }],
      }
    });
    write_line(&self.stdin, &msg).await
  }

  async fn reply_control(&self, request_id: &str) -> AppResult<()> {
    let msg = json!({
      "type": "control_response",
      "response": {
        "subtype": "success",
        "request_id": request_id,
        "response": { "behavior": "allow" },
      }
    });
    write_line(&self.stdin, &msg).await
  }

  pub async fn close_stdin(&self) -> AppResult<()> {
    let mut guard = self.stdin.lock().await;
    if let Some(mut stdin) = guard.take() {
      stdin.shutdown().await?;
    }
    Ok(())
  }

  pub async fn recv_raw(&mut self) -> Option<String> {
    let mut line = String::new();
    match self.stdout.read_line(&mut line).await {
      Ok(0) => None,
      Ok(_) => Some(line),
      Err(e) => {
        debug!(error = %e, "claude-cli: stdout read failed");
        None
      }
    }
  }

  /// Read the next classified event, auto-replying to control requests.
  pub async fn recv_event(&mut self) -> Option<StreamEvent> {
    loop {
      let raw = self.recv_raw().await?;
      let line = raw.trim();
      if line.is_empty() {
        continue;
      }
      let value = match serde_json::from_str::<Value>(line) {
        Ok(v) => v,
        Err(e) => {
          warn!(
            error = %e,
            raw = %truncate(line, 240),
            "claude-cli: skip malformed stream-json line"
          );
          continue;
        }
      };
      let event = parse_stream_event(&value);
      match event {
        StreamEvent::ControlRequest { request_id } => {
          debug!(request_id, "claude-cli: auto-allowing control request");
          if let Err(e) = self.reply_control(&request_id).await {
            warn!(error = %e, "claude-cli: failed to reply to control request");
          }
        }
        other => return Some(other),
      }
    }
  }
}

impl Drop for StreamJsonSession {
  fn drop(&mut self) {
    let _ = self.child.start_kill();
  }
}

async fn write_line(stdin: &Arc<Mutex<Option<ChildStdin>>>, msg: &Value) -> AppResult<()> {
  let mut line = serde_json::to_vec(msg)
    .map_err(|e| AppError::Internal(format!("claude-cli: encode stream-json: {e}")))?;
  line.push(b'\n');
  let mut guard = stdin.lock().await;
  let stdin = guard
    .as_mut()
    .ok_or_else(|| AppError::ProviderFailed("claude-cli: stdin already closed".into()))?;
  stdin.write_all(&line).await?;
  stdin.flush().await?;
  Ok(())
}

fn truncate(s: &str, max: usize) -> String {
  if s.len() <= max {
    s.to_string()
  } else {
    format!("{}…", &s[..max])
  }
}

/// Classify one stream-json object.
pub fn parse_stream_event(value: &Value) -> StreamEvent {
  let kind = value.get("type").and_then(Value::as_str).unwrap_or("");
  match kind {
    "system" if value.get("subtype").and_then(Value::as_str) == Some("init") => StreamEvent::Init {
      session_id: string_field(value, &["session_id", "sessionId"]),
      model: string_field(value, &["model"]),
    },
    "stream_event" => parse_nested_stream_event(value.get("event").unwrap_or(&Value::Null)),
    "assistant" => {
      if let Some(text) = assistant_text(value) {
        StreamEvent::AssistantText(text)
      } else {
        StreamEvent::Ignored
      }
    }
    "result" => parse_result(value),
    "control_request" | "sdk_control_request" => {
      if let Some(id) = control_request_id(value) {
        StreamEvent::ControlRequest { request_id: id }
      } else {
        StreamEvent::Ignored
      }
    }
    _ => StreamEvent::Ignored,
  }
}

fn parse_nested_stream_event(event: &Value) -> StreamEvent {
  let event_type = event.get("type").and_then(Value::as_str).unwrap_or("");
  if event_type != "content_block_delta" && event.get("delta").is_none() {
    return StreamEvent::Ignored;
  }
  let delta = event.get("delta").unwrap_or(&Value::Null);
  let delta_type = delta.get("type").and_then(Value::as_str).unwrap_or("");
  match delta_type {
    "text_delta" => {
      let text = delta.get("text").and_then(Value::as_str).unwrap_or("");
      if text.is_empty() {
        StreamEvent::Ignored
      } else {
        StreamEvent::TextDelta(text.to_string())
      }
    }
    "thinking_delta" => {
      let text = delta
        .get("thinking")
        .or_else(|| delta.get("text"))
        .and_then(Value::as_str)
        .unwrap_or("");
      if text.is_empty() {
        StreamEvent::Ignored
      } else {
        StreamEvent::ThinkingDelta(text.to_string())
      }
    }
    _ => StreamEvent::Ignored,
  }
}

fn parse_result(value: &Value) -> StreamEvent {
  let is_error = value
    .get("is_error")
    .and_then(Value::as_bool)
    .unwrap_or(false)
    || matches!(
      value.get("subtype").and_then(Value::as_str),
      Some("error" | "error_during_execution")
    );
  let content = value
    .get("result")
    .and_then(Value::as_str)
    .unwrap_or("")
    .to_string();
  let error = if is_error {
    Some(
      value
        .get("errors")
        .and_then(Value::as_str)
        .or_else(|| value.get("error").and_then(Value::as_str))
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| {
          if content.is_empty() {
            Some("claude-cli: result error".into())
          } else {
            Some(content.clone())
          }
        })
        .unwrap_or_else(|| "claude-cli: result error".into()),
    )
  } else {
    None
  };
  let (prompt_tokens, completion_tokens) = usage_tokens(value);
  StreamEvent::Result {
    content,
    is_error,
    error,
    prompt_tokens,
    completion_tokens,
  }
}

fn assistant_text(value: &Value) -> Option<String> {
  let content = value
    .get("message")
    .and_then(|m| m.get("content"))
    .or_else(|| value.get("content"))?;
  let mut out = String::new();
  match content {
    Value::String(s) => out.push_str(s),
    Value::Array(blocks) => {
      for block in blocks {
        let kind = block.get("type").and_then(Value::as_str).unwrap_or("text");
        if kind != "text" {
          continue;
        }
        if let Some(t) = block.get("text").and_then(Value::as_str) {
          out.push_str(t);
        }
      }
    }
    _ => {}
  }
  if out.is_empty() { None } else { Some(out) }
}

fn usage_tokens(value: &Value) -> (u32, u32) {
  let usage = value.get("usage");
  let prompt = u32_field(value, &["total_input_tokens"])
    .or_else(|| usage.and_then(|u| u32_field(u, &["input_tokens", "prompt_tokens"])))
    .unwrap_or(0);
  let completion = u32_field(value, &["total_output_tokens"])
    .or_else(|| usage.and_then(|u| u32_field(u, &["output_tokens", "completion_tokens"])))
    .unwrap_or(0);
  (prompt, completion)
}

fn control_request_id(value: &Value) -> Option<String> {
  string_field(value, &["request_id"])
    .or_else(|| {
      value
        .get("request")
        .and_then(|r| string_field(r, &["request_id", "id"]))
    })
    .or_else(|| string_field(value, &["id"]))
}

fn string_field(value: &Value, keys: &[&str]) -> Option<String> {
  for key in keys {
    if let Some(s) = value.get(*key).and_then(Value::as_str) {
      if !s.is_empty() {
        return Some(s.to_string());
      }
    }
  }
  None
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
  fn parse_init_and_text_delta() {
    let init = json!({
      "type": "system",
      "subtype": "init",
      "session_id": "abc",
      "model": "sonnet"
    });
    assert_eq!(
      parse_stream_event(&init),
      StreamEvent::Init {
        session_id: Some("abc".into()),
        model: Some("sonnet".into()),
      }
    );

    let delta = json!({
      "type": "stream_event",
      "event": {
        "type": "content_block_delta",
        "delta": { "type": "text_delta", "text": "Hi" }
      }
    });
    assert_eq!(
      parse_stream_event(&delta),
      StreamEvent::TextDelta("Hi".into())
    );
  }

  #[test]
  fn parse_thinking_and_assistant() {
    let think = json!({
      "type": "stream_event",
      "event": {
        "delta": { "type": "thinking_delta", "thinking": "hmm" }
      }
    });
    assert_eq!(
      parse_stream_event(&think),
      StreamEvent::ThinkingDelta("hmm".into())
    );

    let assistant = json!({
      "type": "assistant",
      "message": {
        "content": [
          { "type": "text", "text": "hello" },
          { "type": "tool_use", "name": "Bash" }
        ]
      }
    });
    assert_eq!(
      parse_stream_event(&assistant),
      StreamEvent::AssistantText("hello".into())
    );
  }

  #[test]
  fn parse_result_success_and_error() {
    let ok = json!({
      "type": "result",
      "subtype": "success",
      "is_error": false,
      "result": "done",
      "usage": { "input_tokens": 3, "output_tokens": 7 }
    });
    assert_eq!(
      parse_stream_event(&ok),
      StreamEvent::Result {
        content: "done".into(),
        is_error: false,
        error: None,
        prompt_tokens: 3,
        completion_tokens: 7,
      }
    );

    let err = json!({
      "type": "result",
      "subtype": "error",
      "is_error": true,
      "result": "not logged in"
    });
    match parse_stream_event(&err) {
      StreamEvent::Result {
        is_error,
        error,
        content,
        ..
      } => {
        assert!(is_error);
        assert_eq!(content, "not logged in");
        assert_eq!(error.as_deref(), Some("not logged in"));
      }
      other => panic!("unexpected {other:?}"),
    }
  }

  #[test]
  fn parse_control_request_shapes() {
    let a = json!({
      "type": "control_request",
      "request_id": "req_1",
      "request": { "subtype": "can_use_tool" }
    });
    assert_eq!(
      parse_stream_event(&a),
      StreamEvent::ControlRequest {
        request_id: "req_1".into()
      }
    );

    let b = json!({
      "type": "sdk_control_request",
      "request": { "subtype": "permission", "request_id": "perm_1" }
    });
    assert_eq!(
      parse_stream_event(&b),
      StreamEvent::ControlRequest {
        request_id: "perm_1".into()
      }
    );
  }
}
