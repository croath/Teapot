//! Shared types for provider HTTP request execution (sync + stream).
//!
//! Each provider implements `execute` / `execute_stream` on its struct.
//! Streaming uses upstream SSE where available and yields [`ExecStreamEvent`]s.
//!
//! Conversation compaction lives in [`crate::providers::compact`].

use serde::de::DeserializeOwned;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use crate::error::{AppError, AppResult};
use crate::models::openai::{
  ChatCompletionRequest, ChatContent, ChatMessage, ResponsesRequest, Usage,
};

/// Basic chat request forwarded to a provider upstream API.
#[derive(Debug, Clone)]
pub struct ExecRequest {
  pub model: String,
  pub messages: Vec<ChatMessage>,
  pub stream: bool,
  pub temperature: Option<f32>,
  pub max_tokens: Option<u32>,
}

impl ExecRequest {
  pub fn from_chat(req: &ChatCompletionRequest) -> Self {
    Self {
      model: req.model.clone(),
      messages: req.messages.clone(),
      stream: req.stream,
      temperature: req.temperature,
      max_tokens: req.max_tokens,
    }
  }

  /// Build from an OpenAI Responses create body (`input` + `instructions`).
  pub fn from_responses(req: &ResponsesRequest) -> Self {
    let (input_system, turns) = req.input.as_prompt_messages();
    let system = match (req.instructions.as_ref(), input_system) {
      (Some(i), Some(s)) => Some(format!("{i}\n\n{s}")),
      (Some(i), None) => Some(i.clone()),
      (None, s) => s,
    };

    let mut messages = Vec::new();
    if let Some(sys) = system.filter(|s| !s.is_empty()) {
      messages.push(ChatMessage {
        role: "system".into(),
        content: ChatContent::Text(sys),
        name: None,
      });
    }
    for (role, text) in turns {
      if text.is_empty() {
        continue;
      }
      messages.push(ChatMessage {
        role,
        content: ChatContent::Text(text),
        name: None,
      });
    }

    Self {
      model: req.model.clone(),
      messages,
      stream: req.stream,
      temperature: req.temperature,
      max_tokens: req.max_output_tokens,
    }
  }

  /// Non-streaming copy of this request (providers that only stream internally).
  pub fn as_non_stream(&self) -> Self {
    let mut r = self.clone();
    r.stream = false;
    r
  }

  /// Split system text and non-system turns (role, text).
  pub fn system_and_turns(&self) -> (Option<String>, Vec<(String, String)>) {
    let mut system_parts: Vec<String> = Vec::new();
    let mut turns: Vec<(String, String)> = Vec::new();
    for msg in &self.messages {
      let text = msg.content.as_text();
      if text.is_empty() {
        continue;
      }
      match msg.role.as_str() {
        "system" | "developer" => system_parts.push(text),
        role => turns.push((role.to_string(), text)),
      }
    }
    let system = if system_parts.is_empty() {
      None
    } else {
      Some(system_parts.join("\n\n"))
    };
    (system, turns)
  }
}

/// Non-stream result from a provider `execute` call.
#[derive(Debug, Clone)]
pub struct ExecResponse {
  pub id: String,
  pub model: String,
  pub content: String,
  pub usage: Usage,
}

impl ExecResponse {
  pub fn new(model: impl Into<String>, content: impl Into<String>) -> Self {
    Self {
      id: format!("chatcmpl-{}", uuid::Uuid::new_v4().simple()),
      model: model.into(),
      content: content.into(),
      usage: Usage {
        prompt_tokens: 0,
        completion_tokens: 0,
        total_tokens: 0,
      },
    }
  }

  pub fn with_usage(mut self, prompt: u32, completion: u32) -> Self {
    self.usage = Usage {
      prompt_tokens: prompt,
      completion_tokens: completion,
      total_tokens: prompt.saturating_add(completion),
    };
    self
  }

  pub fn into_chat_message(self) -> ChatMessage {
    ChatMessage {
      role: "assistant".into(),
      content: ChatContent::Text(self.content),
      name: None,
    }
  }
}

/// One event in a provider streaming response (normalized for Chat Completions SSE).
#[derive(Debug, Clone)]
pub enum ExecStreamEvent {
  /// Optional: model id as returned by upstream.
  Meta {
    model: Option<String>,
    id: Option<String>,
  },
  /// Assistant content text delta.
  ContentDelta { text: String },
  /// Optional reasoning / thinking delta.
  ReasoningDelta { text: String },
  /// Stream finished (usage may be present on last event).
  Done {
    finish_reason: Option<&'static str>,
    usage: Option<Usage>,
  },
}

/// Channel-backed stream of provider events.
pub type ExecStream = ReceiverStream<AppResult<ExecStreamEvent>>;

/// Open an [`ExecStream`] producer channel (buffer size for backpressure).
pub fn exec_stream_channel(
  buffer: usize,
) -> (mpsc::Sender<AppResult<ExecStreamEvent>>, ExecStream) {
  let (tx, rx) = mpsc::channel(buffer);
  (tx, ReceiverStream::new(rx))
}

/// Emit a full non-stream result as a short synthetic stream (content + done).
pub fn synthetic_stream_from_response(resp: ExecResponse) -> ExecStream {
  let (tx, rx) = exec_stream_channel(4);
  tokio::spawn(async move {
    let _ = tx
      .send(Ok(ExecStreamEvent::Meta {
        model: Some(resp.model.clone()),
        id: Some(resp.id.clone()),
      }))
      .await;
    if !resp.content.is_empty() {
      let _ = tx
        .send(Ok(ExecStreamEvent::ContentDelta { text: resp.content }))
        .await;
    }
    let _ = tx
      .send(Ok(ExecStreamEvent::Done {
        finish_reason: Some("stop"),
        usage: Some(resp.usage),
      }))
      .await;
  });
  rx
}

/// Incremental SSE line parser for `data:` frames from an HTTP body.
#[derive(Debug, Default)]
pub struct SseDataParser {
  buf: String,
}

impl SseDataParser {
  pub fn new() -> Self {
    Self::default()
  }

  /// Feed raw bytes; returns complete `data:` payloads (joined multi-line data).
  pub fn push(&mut self, chunk: &[u8]) -> Vec<String> {
    self.buf.push_str(&String::from_utf8_lossy(chunk));
    let mut out = Vec::new();
    loop {
      let Some(nl) = self.buf.find('\n') else {
        break;
      };
      let mut line = self.buf[..nl].to_string();
      self.buf.drain(..=nl);
      if line.ends_with('\r') {
        line.pop();
      }
      if line.is_empty() {
        // event boundary — flush pending via separate state if needed
        continue;
      }
      if let Some(data) = line.strip_prefix("data:") {
        let data = data.trim_start();
        if !data.is_empty() {
          out.push(data.to_string());
        }
      }
      // ignore event:/id:/retry: for now
    }
    out
  }

  /// Flush any remaining incomplete buffer as a data payload if it looks complete.
  pub fn finish(&mut self) -> Vec<String> {
    if self.buf.trim().is_empty() {
      return Vec::new();
    }
    let leftover = std::mem::take(&mut self.buf);
    let mut out = Vec::new();
    for line in leftover.lines() {
      if let Some(data) = line.strip_prefix("data:") {
        let data = data.trim_start();
        if !data.is_empty() {
          out.push(data.to_string());
        }
      }
    }
    out
  }
}

/// Read an HTTP response as SSE `data:` frames and invoke `on_data` for each.
///
/// Stops early when `on_data` returns `true` (done).
pub async fn consume_sse_data<F>(
  provider: &str,
  mut resp: reqwest::Response,
  mut on_data: F,
) -> AppResult<()>
where
  F: FnMut(&str) -> AppResult<bool> + Send,
{
  let status = resp.status().as_u16();
  if !(200..300).contains(&status) {
    let text = resp.text().await.unwrap_or_else(|_| String::new());
    return Err(upstream_error(provider, status, &text));
  }

  let mut parser = SseDataParser::new();
  loop {
    let chunk = resp
      .chunk()
      .await
      .map_err(|e| AppError::ProviderFailed(format!("{provider}: stream read: {e}")))?;
    let Some(chunk) = chunk else {
      break;
    };
    for data in parser.push(&chunk) {
      if on_data(&data)? {
        return Ok(());
      }
    }
  }
  for data in parser.finish() {
    if on_data(&data)? {
      return Ok(());
    }
  }
  Ok(())
}

/// Map a non-success HTTP status + body into [`AppError`].
pub fn upstream_error(provider: &str, status: u16, body: &str) -> AppError {
  let snippet: String = body.chars().take(500).collect();
  if status == 401 || status == 403 {
    return AppError::Unauthorized(format!("{provider} upstream {status}: {snippet}"));
  }
  if status == 400 || status == 404 || status == 422 {
    return AppError::BadRequest(format!("{provider} upstream {status}: {snippet}"));
  }
  AppError::ProviderFailed(format!("{provider} upstream {status}: {snippet}"))
}

/// Read and deserialize a JSON body, checking HTTP status.
pub async fn read_json_response<T: DeserializeOwned>(
  provider: &str,
  resp: reqwest::Response,
) -> AppResult<T> {
  let status = resp.status().as_u16();
  let text = resp
    .text()
    .await
    .map_err(|e| AppError::ProviderFailed(format!("{provider}: read body: {e}")))?;
  if !(200..300).contains(&status) {
    return Err(upstream_error(provider, status, &text));
  }
  serde_json::from_str(&text).map_err(|e| {
    AppError::ProviderFailed(format!(
      "{provider}: invalid JSON response: {e}; body={text}"
    ))
  })
}

/// Read raw body bytes, checking HTTP status.
pub async fn read_body_checked(
  provider: &str,
  resp: reqwest::Response,
) -> AppResult<(u16, String)> {
  let status = resp.status().as_u16();
  let text = resp
    .text()
    .await
    .map_err(|e| AppError::ProviderFailed(format!("{provider}: read body: {e}")))?;
  if !(200..300).contains(&status) {
    return Err(upstream_error(provider, status, &text));
  }
  Ok((status, text))
}
