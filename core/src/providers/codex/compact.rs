//! Codex native compact (`POST /responses/compact`).

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::models::openai::Usage;
use crate::providers::compact::{ExecCompactRequest, ExecCompactResponse};
use crate::providers::execute::{ExecStream, read_json_response};

use super::CodexProvider;

const DEFAULT_BASE: &str = "https://chatgpt.com/backend-api/codex";
const USER_AGENT: &str = "codex-cli/0.1.0";
const ORIGINATOR: &str = "codex-cli";

#[derive(Debug, Serialize)]
struct CodexCompactRequest {
  model: String,
  input: Vec<CodexInputItem>,
  #[serde(skip_serializing_if = "Option::is_none")]
  instructions: Option<String>,
}

#[derive(Debug, Serialize)]
struct CodexInputItem {
  #[serde(rename = "type")]
  item_type: &'static str,
  role: String,
  content: Vec<CodexContentPart>,
}

#[derive(Debug, Serialize)]
struct CodexContentPart {
  #[serde(rename = "type")]
  part_type: &'static str,
  text: String,
}

#[derive(Debug, Deserialize)]
struct CodexCompactResponse {
  #[serde(default)]
  id: Option<String>,
  #[serde(default)]
  output: Vec<CodexCompactOutputItem>,
  #[serde(default)]
  usage: Option<CodexUsage>,
}

#[derive(Debug, Deserialize)]
struct CodexCompactOutputItem {
  #[serde(rename = "type", default)]
  item_type: String,
  #[serde(default)]
  encrypted_content: Option<String>,
  #[serde(default)]
  content: Vec<CodexOutputContent>,
}

#[derive(Debug, Deserialize)]
struct CodexOutputContent {
  #[serde(default)]
  text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CodexUsage {
  #[serde(default)]
  input_tokens: Option<u32>,
  #[serde(default)]
  output_tokens: Option<u32>,
}

impl CodexProvider {
  /// Native compact via `POST {base}/responses/compact` (upstream is non-stream).
  pub async fn execute_compact(&self, req: &ExecCompactRequest) -> AppResult<ExecCompactResponse> {
    let creds = self.session().await?;
    let token = creds.require_access_token()?;
    let body = build_compact_body(req);
    let url = format!("{DEFAULT_BASE}/responses/compact");

    let mut builder = self
      .http
      .post(&url)
      .header("Content-Type", "application/json")
      .header("Authorization", format!("Bearer {token}"))
      .header("Accept", "application/json")
      .header("User-Agent", USER_AGENT)
      .header("Originator", ORIGINATOR);

    if let Some(id) = creds.account_id.as_deref().filter(|s| !s.is_empty()) {
      builder = builder.header("Chatgpt-Account-Id", id);
    }

    let http_resp = builder
      .json(&body)
      .send()
      .await
      .map_err(|e| AppError::ProviderFailed(format!("codex compact request failed: {e}")))?;

    let value: CodexCompactResponse = read_json_response("codex", http_resp).await?;
    parse_compact_response(value)
  }

  /// Stream compact: upstream compact is JSON-only; synthesize deltas from the full body.
  pub async fn execute_compact_stream(&self, req: &ExecCompactRequest) -> AppResult<ExecStream> {
    let full = self.execute_compact(&req.as_non_stream()).await?;
    Ok(full.into_stream())
  }
}

fn build_compact_body(req: &ExecCompactRequest) -> CodexCompactRequest {
  let mut system_parts: Vec<String> = Vec::new();
  let mut turns: Vec<(String, String)> = Vec::new();
  for msg in &req.messages {
    let text = msg.content.as_text();
    if text.is_empty() {
      continue;
    }
    match msg.role.as_str() {
      "system" | "developer" => system_parts.push(text),
      role => turns.push((role.to_string(), text)),
    }
  }
  let from_messages = if system_parts.is_empty() {
    None
  } else {
    Some(system_parts.join("\n\n"))
  };
  let instructions = match (req.instructions.as_ref(), from_messages) {
    (Some(i), Some(s)) => Some(format!("{i}\n\n{s}")),
    (Some(i), None) => Some(i.clone()),
    (None, s) => s,
  };

  CodexCompactRequest {
    model: req.model.clone(),
    input: turns_to_input_items(&turns),
    instructions,
  }
}

fn parse_compact_response(value: CodexCompactResponse) -> AppResult<ExecCompactResponse> {
  let mut content = String::new();
  // Prefer opaque compaction blob when present; fall back to message text.
  for item in &value.output {
    if item.item_type == "compaction" {
      if let Some(enc) = item.encrypted_content.as_ref().filter(|s| !s.is_empty()) {
        content = enc.clone();
        break;
      }
    }
  }
  if content.is_empty() {
    for item in &value.output {
      for part in &item.content {
        if let Some(t) = &part.text {
          content.push_str(t);
        }
      }
    }
  }
  if content.is_empty() {
    return Err(AppError::ProviderFailed(
      "codex compact: empty output (no compaction or text)".into(),
    ));
  }
  let prompt = value
    .usage
    .as_ref()
    .and_then(|u| u.input_tokens)
    .unwrap_or(0);
  let completion = value
    .usage
    .as_ref()
    .and_then(|u| u.output_tokens)
    .unwrap_or(0);
  let usage = Usage {
    prompt_tokens: prompt,
    completion_tokens: completion,
    total_tokens: prompt.saturating_add(completion),
  };
  let mut resp = ExecCompactResponse::new(content, usage);
  if let Some(id) = value.id.filter(|s| !s.is_empty()) {
    resp.id = id;
  }
  Ok(resp)
}

fn turns_to_input_items(turns: &[(String, String)]) -> Vec<CodexInputItem> {
  let mut input = Vec::new();
  for (role, text) in turns {
    let role = match role.as_str() {
      "assistant" | "model" => "assistant",
      _ => "user",
    };
    let part_type = if role == "assistant" {
      "output_text"
    } else {
      "input_text"
    };
    input.push(CodexInputItem {
      item_type: "message",
      role: role.to_string(),
      content: vec![CodexContentPart {
        part_type,
        text: text.clone(),
      }],
    });
  }
  input
}
