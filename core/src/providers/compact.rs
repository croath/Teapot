//! Provider conversation compaction (`execute_compact` / `execute_compact_stream`).
//!
//! Native compact (`POST /responses/compact`) for providers that expose it
//! (codex, xai). Others summarize via `execute` / `execute_stream`.

use crate::models::openai::{ChatContent, ChatMessage, CompactRequest, Usage};
use crate::providers::execute::{
  ExecRequest, ExecResponse, ExecStream, ExecStreamEvent, exec_stream_channel,
};

/// System prompt used when a provider has no native compact endpoint.
pub const COMPACT_SUMMARY_SYSTEM: &str = "Compress the following conversation into a concise summary that preserves key facts, decisions, and open questions for a future turn. Reply with the summary only.";

/// Request for provider-side conversation compaction.
#[derive(Debug, Clone)]
pub struct ExecCompactRequest {
  pub model: String,
  /// Conversation turns derived from Responses `input` (+ optional system).
  pub messages: Vec<ChatMessage>,
  /// Top-level Responses `instructions` (native compact body field).
  pub instructions: Option<String>,
  /// When true, prefer streaming the compaction result (deltas of summary /
  /// encrypted content). Native compact endpoints are non-stream; providers
  /// synthesize a short stream after the full body arrives.
  pub stream: bool,
}

impl ExecCompactRequest {
  /// Build from an OpenAI compact request body.
  pub fn from_compact(req: &CompactRequest, model: impl Into<String>) -> Self {
    let (input_system, turns) = req.input.as_prompt_messages();
    let mut messages = Vec::new();
    if let Some(sys) = input_system.filter(|s| !s.is_empty()) {
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
      model: model.into(),
      messages,
      instructions: req.instructions.clone(),
      stream: req.stream,
    }
  }

  /// Non-streaming copy (providers that only support one-shot compact).
  pub fn as_non_stream(&self) -> Self {
    let mut r = self.clone();
    r.stream = false;
    r
  }

  /// Flatten conversation into a single text blob (for summarize workaround).
  pub fn conversation_text(&self) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(instr) = self.instructions.as_deref().filter(|s| !s.is_empty()) {
      parts.push(instr.to_string());
    }
    for msg in &self.messages {
      let text = msg.content.as_text();
      if text.is_empty() {
        continue;
      }
      match msg.role.as_str() {
        "system" | "developer" => parts.push(text),
        role => parts.push(format!("{role}: {text}")),
      }
    }
    parts.join("\n\n")
  }

  /// Build an `execute` / `execute_stream` request that asks the model to summarize.
  ///
  /// Used by providers without a native compact endpoint (claude, antigravity,
  /// vertex, …).
  pub fn as_summary_exec_request(&self) -> ExecRequest {
    let text = self.conversation_text();
    ExecRequest {
      model: self.model.clone(),
      messages: vec![
        ChatMessage {
          role: "system".into(),
          content: ChatContent::Text(COMPACT_SUMMARY_SYSTEM.into()),
          name: None,
        },
        ChatMessage {
          role: "user".into(),
          content: ChatContent::Text(text),
          name: None,
        },
      ],
      stream: self.stream,
      temperature: Some(0.0),
      max_tokens: Some(2048),
    }
  }
}

/// Result of provider compaction (native or summarize workaround).
#[derive(Debug, Clone)]
pub struct ExecCompactResponse {
  pub id: String,
  /// Summary text or upstream opaque `encrypted_content`.
  pub content: String,
  pub usage: Usage,
}

impl ExecCompactResponse {
  pub fn from_exec(resp: ExecResponse) -> Self {
    let id = if resp.id.starts_with("resp_") {
      resp.id
    } else {
      format!("resp_{}", uuid::Uuid::new_v4().simple())
    };
    Self {
      id,
      content: resp.content,
      usage: resp.usage,
    }
  }

  pub fn new(content: impl Into<String>, usage: Usage) -> Self {
    Self {
      id: format!("resp_{}", uuid::Uuid::new_v4().simple()),
      content: content.into(),
      usage,
    }
  }

  /// Convert to a short synthetic stream (meta + content delta + done).
  pub fn into_stream(self) -> ExecStream {
    synthetic_stream_from_compact(self)
  }
}

/// Emit a full non-stream compact result as a short synthetic stream.
pub fn synthetic_stream_from_compact(resp: ExecCompactResponse) -> ExecStream {
  let (tx, rx) = exec_stream_channel(4);
  tokio::spawn(async move {
    let _ = tx
      .send(Ok(ExecStreamEvent::Meta {
        model: None,
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
