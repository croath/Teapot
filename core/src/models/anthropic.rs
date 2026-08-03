//! Anthropic Claude Messages API compatible types.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Deserialize)]
pub struct MessagesRequest {
  pub model: String,
  pub messages: Vec<Message>,
  #[serde(default)]
  pub system: Option<SystemPrompt>,
  #[serde(default)]
  pub max_tokens: u32,
  #[serde(default)]
  pub stream: bool,
  #[serde(default)]
  pub temperature: Option<f32>,
  #[serde(default)]
  pub top_p: Option<f32>,
  #[serde(default)]
  pub stop_sequences: Option<Vec<String>>,
  #[serde(default)]
  pub metadata: Option<Value>,
  /// Extension: force a specific agent name.
  #[serde(default)]
  pub agent: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum SystemPrompt {
  Text(String),
  Blocks(Vec<ContentBlock>),
}

impl SystemPrompt {
  pub fn as_text(&self) -> String {
    match self {
      Self::Text(s) => s.clone(),
      Self::Blocks(blocks) => blocks
        .iter()
        .filter_map(|b| match b {
          ContentBlock::Text { text, .. } => Some(text.as_str()),
          _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n"),
    }
  }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
  pub role: String,
  pub content: MessageContent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
  Text(String),
  Blocks(Vec<ContentBlock>),
}

impl MessageContent {
  pub fn as_text(&self) -> String {
    match self {
      Self::Text(s) => s.clone(),
      Self::Blocks(blocks) => blocks
        .iter()
        .filter_map(|b| match b {
          ContentBlock::Text { text, .. } => Some(text.as_str()),
          _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n"),
    }
  }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ContentBlock {
  #[serde(rename = "text")]
  Text {
    text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    citations: Option<Value>,
  },
  #[serde(other)]
  Other,
}

#[derive(Debug, Clone, Serialize)]
pub struct MessagesResponse {
  pub id: String,
  #[serde(rename = "type")]
  pub response_type: &'static str,
  pub role: &'static str,
  pub content: Vec<ContentBlock>,
  pub model: String,
  pub stop_reason: Option<&'static str>,
  pub stop_sequence: Option<String>,
  pub usage: AnthropicUsage,
}

#[derive(Debug, Clone, Serialize)]
pub struct AnthropicUsage {
  pub input_tokens: u32,
  pub output_tokens: u32,
}

// Streaming event payloads (Anthropic SSE)

#[derive(Debug, Clone, Serialize)]
pub struct StreamEvent {
  #[serde(rename = "type")]
  pub event_type: &'static str,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub message: Option<MessagesResponse>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub index: Option<u32>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub content_block: Option<ContentBlock>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub delta: Option<StreamDelta>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub usage: Option<AnthropicUsage>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StreamDelta {
  #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
  pub delta_type: Option<&'static str>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub text: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub stop_reason: Option<&'static str>,
}

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
