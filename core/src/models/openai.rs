//! OpenAI-compatible request/response types (Chat Completions + Responses).

use serde::{Deserialize, Serialize};
use serde_json::Value; // only for open metadata extension fields

// ---------------------------------------------------------------------------
// Chat Completions
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct ChatCompletionRequest {
  pub model: String,
  pub messages: Vec<ChatMessage>,
  #[serde(default)]
  pub stream: bool,
  #[serde(default)]
  pub temperature: Option<f32>,
  #[serde(default)]
  pub max_tokens: Option<u32>,
  #[serde(default)]
  pub user: Option<String>,
  /// Extension: force a specific agent name, overriding model mapping.
  #[serde(default)]
  pub agent: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
  pub role: String,
  #[serde(default)]
  pub content: ChatContent,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ChatContent {
  Text(String),
  Parts(Vec<ContentPart>),
  Null,
}

impl Default for ChatContent {
  fn default() -> Self {
    Self::Null
  }
}

impl ChatContent {
  pub fn as_text(&self) -> String {
    match self {
      Self::Text(s) => s.clone(),
      Self::Parts(parts) => parts
        .iter()
        .filter_map(|p| match p {
          ContentPart::Text { text } => Some(text.as_str()),
          ContentPart::Other => None,
        })
        .collect::<Vec<_>>()
        .join("\n"),
      Self::Null => String::new(),
    }
  }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ContentPart {
  #[serde(rename = "text")]
  Text { text: String },
  #[serde(other)]
  Other,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChatCompletionResponse {
  pub id: String,
  pub object: &'static str,
  pub created: i64,
  pub model: String,
  pub choices: Vec<ChatChoice>,
  pub usage: Usage,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChatChoice {
  pub index: u32,
  pub message: ChatMessage,
  pub finish_reason: Option<&'static str>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Usage {
  pub prompt_tokens: u32,
  pub completion_tokens: u32,
  pub total_tokens: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChatCompletionChunk {
  pub id: String,
  pub object: &'static str,
  pub created: i64,
  pub model: String,
  pub choices: Vec<ChatChunkChoice>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChatChunkChoice {
  pub index: u32,
  pub delta: ChatDelta,
  pub finish_reason: Option<&'static str>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct ChatDelta {
  #[serde(skip_serializing_if = "Option::is_none")]
  pub role: Option<&'static str>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub content: Option<String>,
  /// Optional agent reasoning (e.g. Codex `reasoning` items). Not final answer.
  #[serde(skip_serializing_if = "Option::is_none")]
  pub reasoning_content: Option<String>,
  /// Optional tool/command progress (Teapot extension; safe for clients to ignore).
  #[serde(skip_serializing_if = "Option::is_none")]
  pub status: Option<String>,
}

// ---------------------------------------------------------------------------
// Responses API (/v1/responses)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct ResponsesRequest {
  pub model: String,
  #[serde(default)]
  pub input: ResponsesInput,
  #[serde(default)]
  pub instructions: Option<String>,
  #[serde(default)]
  pub stream: bool,
  /// Accepted for wire compatibility. Only `false` is supported today;
  /// `true` is rejected until async background runs are implemented.
  #[serde(default)]
  pub background: bool,
  /// Accepted for wire compatibility; Teapot does not persist responses.
  #[serde(default = "default_true")]
  pub store: bool,
  #[serde(default)]
  pub temperature: Option<f32>,
  #[serde(default)]
  pub max_output_tokens: Option<u32>,
  #[serde(default)]
  pub agent: Option<String>,
  #[serde(default)]
  pub metadata: Option<Value>,
  /// Continue from a previous response (stored for clients; Teapot flattens input only).
  #[serde(default)]
  pub previous_response_id: Option<String>,
}

fn default_true() -> bool {
  true
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum ResponsesInput {
  Text(String),
  Items(Vec<ResponseInputItem>),
}

impl Default for ResponsesInput {
  fn default() -> Self {
    Self::Text(String::new())
  }
}

impl ResponsesInput {
  pub fn as_prompt_messages(&self) -> (Option<String>, Vec<(String, String)>) {
    match self {
      Self::Text(t) => (None, vec![("user".into(), t.clone())]),
      Self::Items(items) => {
        let mut system = None;
        let mut msgs = Vec::new();
        for item in items {
          match item.role.as_deref().unwrap_or("user") {
            "system" | "developer" => {
              let text = item.text_content();
              system = Some(match system {
                Some(s) => format!("{s}\n\n{text}"),
                None => text,
              });
            }
            role => msgs.push((role.to_string(), item.text_content())),
          }
        }
        (system, msgs)
      }
    }
  }

  /// Flatten input into a single text blob for token estimation.
  pub fn as_flat_text(&self) -> String {
    match self {
      Self::Text(t) => t.clone(),
      Self::Items(items) => items
        .iter()
        .map(|i| i.text_content())
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join("\n"),
    }
  }

  /// Convert request input into list-shaped items for `GET .../input_items`.
  pub fn to_input_item_list(&self) -> Vec<ResponseListItem> {
    match self {
      Self::Text(t) => vec![ResponseListItem {
        id: format!("msg_{}", uuid_simple()),
        item_type: "message".into(),
        role: Some("user".into()),
        content: Some(ResponseContent::Text(t.clone())),
        encrypted_content: None,
        status: Some("completed".into()),
      }],
      Self::Items(items) => items
        .iter()
        .map(|item| ResponseListItem {
          id: format!("msg_{}", uuid_simple()),
          item_type: item.item_type.clone().unwrap_or_else(|| "message".into()),
          role: item.role.clone(),
          content: item.content.clone(),
          encrypted_content: None,
          status: Some("completed".into()),
        })
        .collect(),
    }
  }
}

fn uuid_simple() -> String {
  uuid::Uuid::new_v4().simple().to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseInputItem {
  #[serde(default, rename = "type")]
  pub item_type: Option<String>,
  #[serde(default)]
  pub role: Option<String>,
  #[serde(default)]
  pub content: Option<ResponseContent>,
}

impl ResponseInputItem {
  pub fn text_content(&self) -> String {
    match &self.content {
      Some(ResponseContent::Text(s)) => s.clone(),
      Some(ResponseContent::Parts(parts)) => parts
        .iter()
        .filter_map(|p| p.as_text())
        .collect::<Vec<_>>()
        .join("\n"),
      None => String::new(),
    }
  }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ResponseContent {
  Text(String),
  Parts(Vec<ResponseContentPart>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ResponseContentPart {
  #[serde(rename = "text")]
  Text { text: String },
  /// Responses API input content part.
  #[serde(rename = "input_text")]
  InputText { text: String },
  /// Responses API output content part (when replaying prior turns).
  #[serde(rename = "output_text")]
  OutputText { text: String },
  #[serde(other)]
  Other,
}

impl ResponseContentPart {
  pub fn as_text(&self) -> Option<&str> {
    match self {
      Self::Text { text } | Self::InputText { text } | Self::OutputText { text } => {
        Some(text.as_str())
      }
      Self::Other => None,
    }
  }
}

impl From<&Usage> for ResponseUsage {
  fn from(u: &Usage) -> Self {
    Self {
      input_tokens: u.prompt_tokens,
      output_tokens: u.completion_tokens,
      total_tokens: u.total_tokens,
    }
  }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponsesResponse {
  pub id: String,
  pub object: String,
  pub created_at: i64,
  pub status: String,
  pub model: String,
  pub output: Vec<ResponseOutputItem>,
  pub usage: ResponseUsage,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub error: Option<ResponseErrorBody>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseErrorBody {
  pub message: String,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub code: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseOutputItem {
  pub id: String,
  #[serde(rename = "type")]
  pub item_type: String,
  pub role: String,
  pub content: Vec<ResponseOutputContent>,
  pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseOutputContent {
  #[serde(rename = "type")]
  pub content_type: String,
  pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ResponseUsage {
  pub input_tokens: u32,
  pub output_tokens: u32,
  pub total_tokens: u32,
}

/// `DELETE /responses/{id}` body.
#[derive(Debug, Clone, Serialize)]
pub struct ResponseDeleted {
  pub id: String,
  pub object: &'static str,
  pub deleted: bool,
}

/// `GET /responses/{id}/input_items` body.
#[derive(Debug, Clone, Serialize)]
pub struct ResponseItemList {
  pub object: &'static str,
  pub data: Vec<ResponseListItem>,
  pub first_id: Option<String>,
  pub last_id: Option<String>,
  pub has_more: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseListItem {
  pub id: String,
  #[serde(rename = "type")]
  pub item_type: String,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub role: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub content: Option<ResponseContent>,
  /// Opaque compaction payload (OpenAI wire field).
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub encrypted_content: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub status: Option<String>,
}

/// `POST /responses/input_tokens` request.
#[derive(Debug, Clone, Deserialize)]
pub struct InputTokensRequest {
  #[serde(default)]
  pub model: Option<String>,
  #[serde(default)]
  pub input: ResponsesInput,
  #[serde(default)]
  pub instructions: Option<String>,
  #[serde(default)]
  pub conversation: Option<Value>,
}

/// `POST /responses/input_tokens` response.
#[derive(Debug, Clone, Serialize)]
pub struct InputTokensResponse {
  pub object: &'static str,
  pub input_tokens: u32,
}

/// `POST /responses/compact` request.
#[derive(Debug, Clone, Deserialize)]
pub struct CompactRequest {
  #[serde(default)]
  pub model: Option<String>,
  #[serde(default)]
  pub input: ResponsesInput,
  #[serde(default)]
  pub instructions: Option<String>,
  /// Accepted for wire compatibility; Teapot does not persist responses, so
  /// callers must pass `input` when compacting.
  #[serde(default)]
  pub previous_response_id: Option<String>,
  /// Teapot extension: stream compaction deltas (OpenAI public compact is
  /// non-stream). When true, SSE uses Responses-style events and ends with
  /// `response.completed` carrying a `response.compaction` object.
  #[serde(default)]
  pub stream: bool,
}

/// `POST /responses/compact` response.
#[derive(Debug, Clone, Serialize)]
pub struct CompactedResponse {
  pub id: String,
  pub created_at: i64,
  pub object: &'static str,
  pub output: Vec<ResponseListItem>,
  pub usage: ResponseUsage,
}

// ---------------------------------------------------------------------------
// Models API (`GET /models`, `GET /models/{model}`)
// OpenAI wire: https://developers.openai.com/api/reference/resources/models
// ---------------------------------------------------------------------------

/// `GET /models` response body.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelList {
  /// Always `"list"`.
  pub object: String,
  pub data: Vec<Model>,
}

impl ModelList {
  pub fn new(data: Vec<Model>) -> Self {
    Self {
      object: "list".into(),
      data,
    }
  }
}

/// Single model object returned by list/retrieve.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Model {
  pub id: String,
  /// Always `"model"`.
  pub object: String,
  /// Unix timestamp (seconds).
  pub created: i64,
  pub owned_by: String,
}

impl Model {
  pub fn new(id: impl Into<String>, created: i64, owned_by: impl Into<String>) -> Self {
    Self {
      id: id.into(),
      object: "model".into(),
      created,
      owned_by: owned_by.into(),
    }
  }
}

/// Backward-compatible alias used by older call sites.
pub type ModelObject = Model;
