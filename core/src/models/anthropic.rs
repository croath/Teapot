//! Anthropic Claude Messages API compatible types.
//!
//! Wire shape follows the official `POST /v1/messages` contract
//! (`https://platform.claude.com/docs/en/api/messages/create`). Teapot serves
//! it at `POST /claude/v1/messages`.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::models::openai::Usage;

/// Official limit on `messages` length.
pub const MAX_MESSAGES: usize = 100_000;

// ---------------------------------------------------------------------------
// Request
// ---------------------------------------------------------------------------

/// `POST /v1/messages` request body.
///
/// Required: `model`, `messages`, `max_tokens`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessagesRequest {
  pub model: String,
  pub messages: Vec<InputMessage>,
  pub max_tokens: u32,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub system: Option<SystemPrompt>,
  #[serde(default)]
  pub stream: bool,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub temperature: Option<f32>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub top_p: Option<f32>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub top_k: Option<i64>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub stop_sequences: Option<Vec<String>>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub metadata: Option<Value>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub thinking: Option<Value>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub tool_choice: Option<Value>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub tools: Option<Vec<Value>>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub output_config: Option<Value>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub service_tier: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub cache_control: Option<Value>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub container: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub inference_geo: Option<String>,
  /// Extension: force a specific agent name.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub agent: Option<String>,
}

/// One turn in `messages[]` (`MessageParam`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InputMessage {
  pub role: String,
  pub content: MessageContent,
}

/// Back-compat alias used by older call sites.
pub type Message = InputMessage;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SystemPrompt {
  Text(String),
  Blocks(Vec<ContentBlock>),
}

impl SystemPrompt {
  pub fn as_text(&self) -> String {
    match self {
      Self::Text(s) => s.clone(),
      Self::Blocks(blocks) => join_block_text(blocks),
    }
  }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum MessageContent {
  Text(String),
  Blocks(Vec<ContentBlock>),
}

impl MessageContent {
  pub fn as_text(&self) -> String {
    match self {
      Self::Text(s) => s.clone(),
      Self::Blocks(blocks) => join_block_text(blocks),
    }
  }
}

/// Input / output content block. Discriminated on `type`.
///
/// Core variants match the official Messages schema. Unknown `type` values
/// deserialize as [`ContentBlock::Other`] so new server-tool blocks stay
/// forward-compatible.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
  Text {
    text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    citations: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cache_control: Option<Value>,
  },
  Image {
    source: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cache_control: Option<Value>,
  },
  ToolUse {
    id: String,
    name: String,
    input: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cache_control: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    caller: Option<Value>,
  },
  ToolResult {
    tool_use_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    content: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    is_error: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cache_control: Option<Value>,
  },
  Thinking {
    thinking: String,
    signature: String,
  },
  RedactedThinking {
    data: String,
  },
  #[serde(other)]
  Other,
}

impl ContentBlock {
  pub fn text(text: impl Into<String>) -> Self {
    Self::Text {
      text: text.into(),
      citations: None,
      cache_control: None,
    }
  }

  pub fn empty_text() -> Self {
    Self::text(String::new())
  }

  pub fn empty_thinking() -> Self {
    Self::Thinking {
      thinking: String::new(),
      signature: String::new(),
    }
  }

  /// Flatten this block to prompt text for [`crate::providers::execute::ExecRequest`].
  pub fn as_plain_text(&self) -> Option<String> {
    match self {
      Self::Text { text, .. } => Some(text.clone()),
      Self::Thinking { thinking, .. } => Some(thinking.clone()),
      Self::ToolResult { content, .. } => tool_result_as_text(content),
      Self::ToolUse { name, input, .. } => Some(format!("{name} {input}")),
      Self::Image { .. } | Self::RedactedThinking { .. } | Self::Other => None,
    }
  }
}

fn join_block_text(blocks: &[ContentBlock]) -> String {
  blocks
    .iter()
    .filter_map(ContentBlock::as_plain_text)
    .filter(|s| !s.is_empty())
    .collect::<Vec<_>>()
    .join("\n")
}

fn tool_result_as_text(content: &Option<Value>) -> Option<String> {
  match content {
    Some(Value::String(s)) => Some(s.clone()),
    Some(Value::Array(arr)) => {
      let parts: Vec<&str> = arr
        .iter()
        .filter_map(|v| v.get("text").and_then(|t| t.as_str()))
        .collect();
      if parts.is_empty() {
        None
      } else {
        Some(parts.join("\n"))
      }
    }
    _ => None,
  }
}

// ---------------------------------------------------------------------------
// Response
// ---------------------------------------------------------------------------

/// Non-stream `Message` object returned by `POST /v1/messages`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MessagesResponse {
  pub id: String,
  #[serde(rename = "type")]
  pub response_type: &'static str,
  pub role: &'static str,
  pub content: Vec<ContentBlock>,
  pub model: String,
  pub stop_reason: Option<String>,
  pub stop_sequence: Option<String>,
  pub usage: AnthropicUsage,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub container: Option<Value>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub stop_details: Option<Value>,
}

impl MessagesResponse {
  /// Completed non-stream assistant message (one text block when `content` is non-empty).
  pub fn completed(
    id: impl Into<String>,
    model: impl Into<String>,
    content: impl Into<String>,
    usage: &Usage,
    stop_reason: &'static str,
  ) -> Self {
    let text = content.into();
    let content = if text.is_empty() {
      Vec::new()
    } else {
      vec![ContentBlock::text(text)]
    };
    Self {
      id: id.into(),
      response_type: "message",
      role: "assistant",
      content,
      model: model.into(),
      stop_reason: Some(stop_reason.to_string()),
      stop_sequence: None,
      usage: AnthropicUsage::from_openai(usage),
      container: None,
      stop_details: None,
    }
  }

  /// Partial message used as the `message_start` payload (`stop_reason` null).
  pub fn stream_start(id: impl Into<String>, model: impl Into<String>) -> Self {
    Self {
      id: id.into(),
      response_type: "message",
      role: "assistant",
      content: Vec::new(),
      model: model.into(),
      stop_reason: None,
      stop_sequence: None,
      usage: AnthropicUsage::default(),
      container: None,
      stop_details: None,
    }
  }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct AnthropicUsage {
  pub input_tokens: u32,
  pub output_tokens: u32,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub cache_creation_input_tokens: Option<u32>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub cache_read_input_tokens: Option<u32>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub cache_creation: Option<Value>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub inference_geo: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub output_tokens_details: Option<Value>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub server_tool_use: Option<Value>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub service_tier: Option<String>,
}

impl AnthropicUsage {
  pub fn new(input_tokens: u32, output_tokens: u32) -> Self {
    Self {
      input_tokens,
      output_tokens,
      ..Self::default()
    }
  }

  pub fn from_openai(usage: &Usage) -> Self {
    Self::new(usage.prompt_tokens, usage.completion_tokens)
  }
}

/// Map an OpenAI-style finish reason onto Anthropic `stop_reason`.
pub fn stop_reason_from_finish(finish: Option<&str>) -> &'static str {
  match finish {
    Some("length") | Some("max_tokens") => "max_tokens",
    Some("tool_calls") | Some("tool_use") => "tool_use",
    Some("stop_sequence") => "stop_sequence",
    Some("refusal") => "refusal",
    Some("pause_turn") => "pause_turn",
    Some("model_context_window_exceeded") => "model_context_window_exceeded",
    _ => "end_turn",
  }
}

/// Surface-facing message id (`msg_…`). Reuses the suffix of `chatcmpl-…`.
pub fn anthropic_message_id(exec_id: &str) -> String {
  if exec_id.starts_with("msg_") {
    return exec_id.to_string();
  }
  if let Some(rest) = exec_id.strip_prefix("chatcmpl-")
    && !rest.is_empty()
  {
    return format!("msg_{rest}");
  }
  format!("msg_{}", uuid::Uuid::new_v4().simple())
}

// ---------------------------------------------------------------------------
// Streaming events (SSE `event:` + `data:`)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct MessageStartEvent {
  #[serde(rename = "type")]
  pub event_type: &'static str,
  pub message: MessagesResponse,
}

impl MessageStartEvent {
  pub fn new(message: MessagesResponse) -> Self {
    Self {
      event_type: "message_start",
      message,
    }
  }
}

#[derive(Debug, Clone, Serialize)]
pub struct ContentBlockStartEvent {
  #[serde(rename = "type")]
  pub event_type: &'static str,
  pub index: u32,
  pub content_block: ContentBlock,
}

impl ContentBlockStartEvent {
  pub fn new(index: u32, content_block: ContentBlock) -> Self {
    Self {
      event_type: "content_block_start",
      index,
      content_block,
    }
  }
}

#[derive(Debug, Clone, Serialize)]
pub struct ContentBlockDeltaEvent {
  #[serde(rename = "type")]
  pub event_type: &'static str,
  pub index: u32,
  pub delta: StreamDelta,
}

impl ContentBlockDeltaEvent {
  pub fn text(index: u32, text: impl Into<String>) -> Self {
    Self {
      event_type: "content_block_delta",
      index,
      delta: StreamDelta::text(text),
    }
  }

  pub fn thinking(index: u32, thinking: impl Into<String>) -> Self {
    Self {
      event_type: "content_block_delta",
      index,
      delta: StreamDelta::thinking(thinking),
    }
  }
}

#[derive(Debug, Clone, Serialize)]
pub struct ContentBlockStopEvent {
  #[serde(rename = "type")]
  pub event_type: &'static str,
  pub index: u32,
}

impl ContentBlockStopEvent {
  pub fn new(index: u32) -> Self {
    Self {
      event_type: "content_block_stop",
      index,
    }
  }
}

#[derive(Debug, Clone, Serialize)]
pub struct MessageDeltaEvent {
  #[serde(rename = "type")]
  pub event_type: &'static str,
  pub delta: MessageDelta,
  pub usage: MessageDeltaUsage,
}

impl MessageDeltaEvent {
  pub fn new(stop_reason: &'static str, output_tokens: u32) -> Self {
    Self {
      event_type: "message_delta",
      delta: MessageDelta {
        stop_reason: Some(stop_reason.to_string()),
        stop_sequence: None,
      },
      usage: MessageDeltaUsage { output_tokens },
    }
  }
}

/// `message_delta` usage fragment — official payload is `output_tokens` only.
#[derive(Debug, Clone, Serialize)]
pub struct MessageDeltaUsage {
  pub output_tokens: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct MessageDelta {
  pub stop_reason: Option<String>,
  pub stop_sequence: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MessageStopEvent {
  #[serde(rename = "type")]
  pub event_type: &'static str,
}

impl MessageStopEvent {
  pub fn new() -> Self {
    Self {
      event_type: "message_stop",
    }
  }
}

impl Default for MessageStopEvent {
  fn default() -> Self {
    Self::new()
  }
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct StreamDelta {
  #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
  pub delta_type: Option<&'static str>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub text: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub thinking: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub stop_reason: Option<&'static str>,
}

impl StreamDelta {
  pub fn text(text: impl Into<String>) -> Self {
    Self {
      delta_type: Some("text_delta"),
      text: Some(text.into()),
      thinking: None,
      stop_reason: None,
    }
  }

  pub fn thinking(thinking: impl Into<String>) -> Self {
    Self {
      delta_type: Some("thinking_delta"),
      text: None,
      thinking: Some(thinking.into()),
      stop_reason: None,
    }
  }
}

// ---------------------------------------------------------------------------
// Models API
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct ModelList {
  pub data: Vec<ModelObject>,
  pub has_more: bool,
  pub first_id: Option<String>,
  pub last_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelObject {
  #[serde(rename = "type")]
  pub object_type: &'static str,
  pub id: String,
  pub display_name: String,
  pub created_at: String,
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn deserializes_official_minimal_request() {
    let req: MessagesRequest = serde_json::from_str(
      r#"{
        "model": "claude-sonnet-4-6",
        "max_tokens": 1024,
        "messages": [{ "role": "user", "content": "Hello, Claude" }]
      }"#,
    )
    .unwrap();
    assert_eq!(req.model, "claude-sonnet-4-6");
    assert_eq!(req.max_tokens, 1024);
    assert!(!req.stream);
    assert_eq!(req.messages.len(), 1);
    assert_eq!(req.messages[0].role, "user");
    assert_eq!(req.messages[0].content.as_text(), "Hello, Claude");
  }

  #[test]
  fn rejects_missing_max_tokens() {
    let err = serde_json::from_str::<MessagesRequest>(
      r#"{
        "model": "claude-sonnet-4-6",
        "messages": [{ "role": "user", "content": "Hello" }]
      }"#,
    )
    .unwrap_err();
    assert!(err.to_string().contains("max_tokens"), "{err}");
  }

  #[test]
  fn deserializes_block_content_and_tools() {
    let req: MessagesRequest = serde_json::from_str(
      r#"{
        "model": "claude-sonnet-4-6",
        "max_tokens": 1024,
        "system": "Be brief.",
        "tools": [{ "name": "get_stock_price", "input_schema": { "type": "object" } }],
        "messages": [{
          "role": "user",
          "content": [
            { "type": "text", "text": "price?" },
            {
              "type": "tool_result",
              "tool_use_id": "toolu_1",
              "content": "259.75 USD"
            }
          ]
        }]
      }"#,
    )
    .unwrap();
    assert_eq!(req.system.as_ref().unwrap().as_text(), "Be brief.");
    assert!(req.tools.is_some());
    assert_eq!(req.messages[0].content.as_text(), "price?\n259.75 USD");
  }

  #[test]
  fn serializes_official_minimal_response() {
    let resp = MessagesResponse {
      id: "msg_013Zva2CMHLNnXjNJJKqJ2EF".into(),
      response_type: "message",
      role: "assistant",
      content: vec![ContentBlock::text("Hi! How can I help you today?")],
      model: "claude-sonnet-4-6".into(),
      stop_reason: Some("end_turn".into()),
      stop_sequence: None,
      usage: AnthropicUsage::new(12, 18),
      container: None,
      stop_details: None,
    };
    let v = serde_json::to_value(&resp).unwrap();
    assert_eq!(v["type"], "message");
    assert_eq!(v["role"], "assistant");
    assert_eq!(v["content"][0]["type"], "text");
    assert_eq!(v["stop_reason"], "end_turn");
    assert!(v["stop_sequence"].is_null());
    assert_eq!(v["usage"]["input_tokens"], 12);
    assert_eq!(v["usage"]["output_tokens"], 18);
  }

  #[test]
  fn maps_exec_id_and_finish_reason() {
    assert_eq!(anthropic_message_id("chatcmpl-abc"), "msg_abc");
    assert_eq!(anthropic_message_id("msg_already"), "msg_already");
    assert_eq!(stop_reason_from_finish(Some("stop")), "end_turn");
    assert_eq!(stop_reason_from_finish(Some("length")), "max_tokens");
    assert_eq!(stop_reason_from_finish(Some("tool_calls")), "tool_use");
  }

  #[test]
  fn completed_response_wraps_text_and_usage() {
    let usage = Usage {
      prompt_tokens: 12,
      completion_tokens: 18,
      total_tokens: 30,
    };
    let resp =
      MessagesResponse::completed("msg_abc", "claude-sonnet-4-6", "Hi!", &usage, "end_turn");
    assert_eq!(resp.id, "msg_abc");
    assert_eq!(resp.response_type, "message");
    assert_eq!(resp.role, "assistant");
    assert_eq!(resp.content, vec![ContentBlock::text("Hi!")]);
    assert_eq!(resp.stop_reason.as_deref(), Some("end_turn"));
    assert_eq!(resp.usage.input_tokens, 12);
    assert_eq!(resp.usage.output_tokens, 18);
  }

  #[test]
  fn stream_start_has_null_stop_reason() {
    let start = MessagesResponse::stream_start("msg_1", "claude-sonnet-4-6");
    let v = serde_json::to_value(&start).unwrap();
    assert!(v["stop_reason"].is_null());
    assert_eq!(v["content"], serde_json::json!([]));
    assert_eq!(v["usage"]["input_tokens"], 0);
  }

  #[test]
  fn stream_events_match_official_shape() {
    let start = serde_json::to_value(MessageStartEvent::new(MessagesResponse::stream_start(
      "msg_1",
      "claude-sonnet-4-6",
    )))
    .unwrap();
    assert_eq!(start["type"], "message_start");
    assert_eq!(start["message"]["type"], "message");

    let delta = serde_json::to_value(ContentBlockDeltaEvent::text(0, "Hi")).unwrap();
    assert_eq!(delta["type"], "content_block_delta");
    assert_eq!(delta["index"], 0);
    assert_eq!(delta["delta"]["type"], "text_delta");
    assert_eq!(delta["delta"]["text"], "Hi");

    let end = serde_json::to_value(MessageDeltaEvent::new("end_turn", 18)).unwrap();
    assert_eq!(end["type"], "message_delta");
    assert_eq!(end["delta"]["stop_reason"], "end_turn");
    assert!(end["delta"]["stop_sequence"].is_null());
    assert_eq!(end["usage"]["output_tokens"], 18);
    assert!(end["usage"].get("input_tokens").is_none());
  }
}
