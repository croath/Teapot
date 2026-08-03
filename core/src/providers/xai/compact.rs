//! xAI native compact (`POST /responses/compact` on official API base).

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::models::openai::Usage;
use crate::providers::compact::{ExecCompactRequest, ExecCompactResponse};
use crate::providers::execute::{ExecStream, read_json_response};

use super::XaiProvider;

/// Official API base for compact (chat-proxy returns 404 on `/responses/compact`).
const OFFICIAL_API_BASE: &str = "https://api.x.ai/v1";

#[derive(Debug, Serialize)]
struct XaiCompactRequest {
  model: String,
  input: Vec<XaiCompactInputItem>,
  #[serde(skip_serializing_if = "Option::is_none")]
  instructions: Option<String>,
}

#[derive(Debug, Serialize)]
struct XaiCompactInputItem {
  #[serde(rename = "type")]
  item_type: &'static str,
  role: String,
  content: Vec<XaiCompactContentPart>,
}

#[derive(Debug, Serialize)]
struct XaiCompactContentPart {
  #[serde(rename = "type")]
  part_type: &'static str,
  text: String,
}

#[derive(Debug, Deserialize)]
struct XaiCompactResponse {
  #[serde(default)]
  id: Option<String>,
  #[serde(default)]
  output: Vec<XaiCompactOutputItem>,
  #[serde(default)]
  usage: Option<XaiCompactUsage>,
}

#[derive(Debug, Deserialize)]
struct XaiCompactOutputItem {
  #[serde(rename = "type", default)]
  item_type: String,
  #[serde(default)]
  encrypted_content: Option<String>,
  #[serde(default)]
  content: Option<XaiCompactOutputContent>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum XaiCompactOutputContent {
  Text(String),
  Parts(Vec<XaiCompactOutputPart>),
}

#[derive(Debug, Deserialize)]
struct XaiCompactOutputPart {
  #[serde(default)]
  text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct XaiCompactUsage {
  #[serde(default)]
  input_tokens: Option<u32>,
  #[serde(default)]
  output_tokens: Option<u32>,
  #[serde(default)]
  prompt_tokens: Option<u32>,
  #[serde(default)]
  completion_tokens: Option<u32>,
}

impl XaiProvider {
  /// Native compact via `POST {official}/responses/compact` (never chat-proxy).
  pub async fn execute_compact(&self, req: &ExecCompactRequest) -> AppResult<ExecCompactResponse> {
    let creds = self.session().await?;
    let token = creds.require_access_token()?;
    let base = compact_api_base(creds.api_base());
    let url = format!("{base}/responses/compact");
    let body = build_compact_body(req);

    let http_resp = self
      .http
      .post(&url)
      .header("Content-Type", "application/json")
      .header("Authorization", format!("Bearer {token}"))
      .header("Accept", "application/json")
      .header("Connection", "Keep-Alive")
      .json(&body)
      .send()
      .await
      .map_err(|e| AppError::ProviderFailed(format!("xai compact request failed: {e}")))?;

    let value: XaiCompactResponse = read_json_response("xai", http_resp).await?;
    parse_compact_response(value)
  }

  /// Stream compact: upstream compact is JSON-only; synthesize deltas from the full body.
  pub async fn execute_compact_stream(&self, req: &ExecCompactRequest) -> AppResult<ExecStream> {
    let full = self.execute_compact(&req.as_non_stream()).await?;
    Ok(full.into_stream())
  }
}

fn build_compact_body(req: &ExecCompactRequest) -> XaiCompactRequest {
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
    input.push(XaiCompactInputItem {
      item_type: "message",
      role: role.to_string(),
      content: vec![XaiCompactContentPart { part_type, text }],
    });
  }

  XaiCompactRequest {
    model: req.model.clone(),
    input,
    instructions,
  }
}

fn parse_compact_response(value: XaiCompactResponse) -> AppResult<ExecCompactResponse> {
  let mut content = String::new();
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
      match &item.content {
        Some(XaiCompactOutputContent::Text(t)) => content.push_str(t),
        Some(XaiCompactOutputContent::Parts(parts)) => {
          for p in parts {
            if let Some(t) = &p.text {
              content.push_str(t);
            }
          }
        }
        None => {
          if let Some(enc) = item.encrypted_content.as_ref().filter(|s| !s.is_empty()) {
            content.push_str(enc);
          }
        }
      }
    }
  }
  if content.is_empty() {
    return Err(AppError::ProviderFailed(
      "xai compact: empty output (no compaction or text)".into(),
    ));
  }
  let prompt = value
    .usage
    .as_ref()
    .and_then(|u| u.input_tokens.or(u.prompt_tokens))
    .unwrap_or(0);
  let completion = value
    .usage
    .as_ref()
    .and_then(|u| u.output_tokens.or(u.completion_tokens))
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

/// Compact must not use cli-chat-proxy (404); prefer official or non-proxy base.
fn compact_api_base(configured: &str) -> &str {
  let base = configured.trim().trim_end_matches('/');
  if base.is_empty() || base.contains("cli-chat-proxy") {
    OFFICIAL_API_BASE
  } else {
    base
  }
}
