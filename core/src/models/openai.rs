//! OpenAI-compatible request/response types (Chat Completions + Responses).

use serde::{Deserialize, Serialize};
use serde_json::Value;

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
  #[serde(default)]
  pub temperature: Option<f32>,
  #[serde(default)]
  pub max_output_tokens: Option<u32>,
  #[serde(default)]
  pub agent: Option<String>,
  #[serde(default)]
  pub metadata: Option<Value>,
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
}

#[derive(Debug, Clone, Deserialize)]
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
        .filter_map(|p| p.get("text").and_then(|v| v.as_str()))
        .collect::<Vec<_>>()
        .join("\n"),
      None => String::new(),
    }
  }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum ResponseContent {
  Text(String),
  Parts(Vec<Value>),
}

#[derive(Debug, Clone, Serialize)]
pub struct ResponsesResponse {
  pub id: String,
  pub object: &'static str,
  pub created_at: i64,
  pub status: &'static str,
  pub model: String,
  pub output: Vec<ResponseOutputItem>,
  pub usage: ResponseUsage,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResponseOutputItem {
  pub id: String,
  #[serde(rename = "type")]
  pub item_type: &'static str,
  pub role: &'static str,
  pub content: Vec<ResponseOutputContent>,
  pub status: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResponseOutputContent {
  #[serde(rename = "type")]
  pub content_type: &'static str,
  pub text: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResponseUsage {
  pub input_tokens: u32,
  pub output_tokens: u32,
  pub total_tokens: u32,
}

// ---------------------------------------------------------------------------
// Models list
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct ModelList {
  pub object: &'static str,
  pub data: Vec<ModelObject>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelObject {
  pub id: String,
  pub object: &'static str,
  pub created: i64,
  pub owned_by: String,
}
