//! Codex HTTP request execution (Responses API) — sync + stream.

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::models::openai::Usage;
use crate::providers::execute::{
  ExecRequest, ExecResponse, ExecStream, ExecStreamEvent, exec_stream_channel, read_body_checked,
  upstream_error,
};

use super::CodexProvider;

const DEFAULT_BASE: &str = "https://chatgpt.com/backend-api/codex";
const USER_AGENT: &str = "codex-cli/0.1.0";
const ORIGINATOR: &str = "codex-cli";

#[derive(Debug, Serialize)]
struct CodexResponsesRequest {
  model: String,
  input: Vec<CodexInputItem>,
  stream: bool,
  store: bool,
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
struct CodexSseEvent {
  #[serde(rename = "type", default)]
  event_type: String,
  #[serde(default)]
  delta: Option<String>,
  #[serde(default)]
  message: Option<String>,
  #[serde(default)]
  error: Option<CodexErrorBody>,
  #[serde(default)]
  response: Option<CodexResponseBody>,
}

#[derive(Debug, Deserialize)]
struct CodexErrorBody {
  #[serde(default)]
  message: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CodexResponseBody {
  #[serde(default)]
  output: Vec<CodexOutputItem>,
  #[serde(default)]
  usage: Option<CodexUsage>,
}

#[derive(Debug, Deserialize)]
struct CodexOutputItem {
  #[serde(rename = "type", default)]
  item_type: String,
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
  /// Non-stream execute (buffers the upstream SSE until complete).
  pub async fn execute(&self, req: &ExecRequest) -> AppResult<ExecResponse> {
    let http_resp = self.send_responses(req).await?;
    let (_status, text) = read_body_checked("codex", http_resp).await?;
    self.parse_sse_completed(&req.model, &text)
  }

  /// Stream execute: forward upstream Responses SSE as [`ExecStreamEvent`]s.
  pub async fn execute_stream(&self, req: &ExecRequest) -> AppResult<ExecStream> {
    let model = req.model.clone();
    let mut http_resp = self.send_responses(req).await?;
    let status = http_resp.status().as_u16();
    if !(200..300).contains(&status) {
      let text = http_resp.text().await.unwrap_or_default();
      return Err(upstream_error("codex", status, &text));
    }

    let (tx, rx) = exec_stream_channel(64);
    tokio::spawn(async move {
      use crate::providers::execute::SseDataParser;
      let mut parser = SseDataParser::new();
      let mut content = String::new();
      let mut usage: Option<Usage> = None;
      let mut saw_completed = false;
      let mut fatal: Option<AppError> = None;

      loop {
        let chunk = match http_resp.chunk().await {
          Ok(Some(c)) => c,
          Ok(None) => break,
          Err(e) => {
            fatal = Some(AppError::ProviderFailed(format!("codex: stream read: {e}")));
            break;
          }
        };
        for data in parser.push(&chunk) {
          if data == "[DONE]" {
            continue;
          }
          let Ok(event) = serde_json::from_str::<CodexSseEvent>(&data) else {
            continue;
          };
          match event.event_type.as_str() {
            "response.output_text.delta" => {
              if let Some(delta) = event.delta.filter(|s| !s.is_empty()) {
                content.push_str(&delta);
                if tx
                  .send(Ok(ExecStreamEvent::ContentDelta { text: delta }))
                  .await
                  .is_err()
                {
                  return;
                }
              }
            }
            "response.completed" | "response.incomplete" => {
              saw_completed = true;
              if let Some(resp) = &event.response {
                if content.is_empty() {
                  let full = extract_completed_text(resp);
                  if !full.is_empty() {
                    content = full.clone();
                    let _ = tx
                      .send(Ok(ExecStreamEvent::ContentDelta { text: full }))
                      .await;
                  }
                }
                if let Some(u) = &resp.usage {
                  let p = u.input_tokens.unwrap_or(0);
                  let c = u.output_tokens.unwrap_or(0);
                  usage = Some(Usage {
                    prompt_tokens: p,
                    completion_tokens: c,
                    total_tokens: p.saturating_add(c),
                  });
                }
              }
            }
            "error" | "response.failed" => {
              let msg = event
                .error
                .as_ref()
                .and_then(|e| e.message.as_deref())
                .or(event.message.as_deref())
                .unwrap_or("codex stream error");
              fatal = Some(AppError::ProviderFailed(msg.into()));
            }
            _ => {}
          }
          if fatal.is_some() || saw_completed {
            break;
          }
        }
        if fatal.is_some() || saw_completed {
          break;
        }
      }

      if let Some(e) = fatal {
        let _ = tx.send(Err(e)).await;
        return;
      }
      if !saw_completed && content.is_empty() {
        let _ = tx
          .send(Err(AppError::ProviderFailed(
            "codex: stream ended without response.completed".into(),
          )))
          .await;
        return;
      }
      let _ = tx
        .send(Ok(ExecStreamEvent::Meta {
          model: Some(model),
          id: None,
        }))
        .await;
      let _ = tx
        .send(Ok(ExecStreamEvent::Done {
          finish_reason: Some("stop"),
          usage,
        }))
        .await;
    });
    Ok(rx)
  }

  async fn send_responses(&self, req: &ExecRequest) -> AppResult<reqwest::Response> {
    let creds = self.session().await?;
    let token = creds.require_access_token()?;
    let body = self.build_body(req);
    let url = format!("{DEFAULT_BASE}/responses");

    let mut builder = self
      .http
      .post(&url)
      .header("Content-Type", "application/json")
      .header("Authorization", format!("Bearer {token}"))
      .header("Accept", "text/event-stream")
      .header("User-Agent", USER_AGENT)
      .header("Originator", ORIGINATOR);

    if let Some(id) = creds.account_id.as_deref().filter(|s| !s.is_empty()) {
      builder = builder.header("Chatgpt-Account-Id", id);
    }

    builder
      .json(&body)
      .send()
      .await
      .map_err(|e| AppError::ProviderFailed(format!("codex request failed: {e}")))
  }

  fn build_body(&self, req: &ExecRequest) -> CodexResponsesRequest {
    let (system, turns) = req.system_and_turns();
    CodexResponsesRequest {
      model: req.model.clone(),
      input: turns_to_input_items(&turns),
      stream: true,
      store: false,
      instructions: system,
    }
  }

  fn parse_sse_completed(&self, model: &str, raw: &str) -> AppResult<ExecResponse> {
    let mut content = String::new();
    let mut prompt_tokens = 0u32;
    let mut completion_tokens = 0u32;
    let mut saw_completed = false;

    for line in raw.lines() {
      let line = line.trim();
      let Some(data) = line.strip_prefix("data:") else {
        continue;
      };
      let data = data.trim();
      if data.is_empty() || data == "[DONE]" {
        continue;
      }
      let Ok(event) = serde_json::from_str::<CodexSseEvent>(data) else {
        continue;
      };
      match event.event_type.as_str() {
        "response.output_text.delta" => {
          if let Some(delta) = event.delta {
            content.push_str(&delta);
          }
        }
        "response.completed" | "response.incomplete" => {
          saw_completed = true;
          if let Some(resp) = &event.response {
            if content.is_empty() {
              content = extract_completed_text(resp);
            }
            if let Some(usage) = &resp.usage {
              prompt_tokens = usage.input_tokens.unwrap_or(0);
              completion_tokens = usage.output_tokens.unwrap_or(0);
            }
          }
        }
        "error" | "response.failed" => {
          let msg = event
            .error
            .as_ref()
            .and_then(|e| e.message.as_deref())
            .or(event.message.as_deref())
            .unwrap_or("codex stream error");
          return Err(AppError::ProviderFailed(msg.into()));
        }
        _ => {}
      }
    }

    if !saw_completed && content.is_empty() {
      return Err(upstream_error(
        "codex",
        502,
        "stream ended without response.completed",
      ));
    }

    Ok(ExecResponse::new(model, content).with_usage(prompt_tokens, completion_tokens))
  }
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

fn extract_completed_text(resp: &CodexResponseBody) -> String {
  let mut text = String::new();
  for item in &resp.output {
    if item.item_type != "message" {
      continue;
    }
    for part in &item.content {
      if let Some(t) = &part.text {
        text.push_str(t);
      }
    }
  }
  text
}
