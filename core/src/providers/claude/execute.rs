//! Claude HTTP request execution (Messages API) — sync + stream.

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::models::openai::Usage;
use crate::providers::execute::{
  ExecRequest, ExecResponse, ExecStream, ExecStreamEvent, SseDataParser, exec_stream_channel,
  read_json_response, upstream_error,
};

use super::ClaudeProvider;

const DEFAULT_BASE: &str = "https://api.anthropic.com";
const ANTHROPIC_VERSION: &str = "2023-06-01";

#[derive(Debug, Serialize)]
struct ClaudeMessagesRequest {
  model: String,
  max_tokens: u32,
  messages: Vec<ClaudeMessage>,
  stream: bool,
  #[serde(skip_serializing_if = "Option::is_none")]
  system: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  temperature: Option<f32>,
}

#[derive(Debug, Serialize)]
struct ClaudeMessage {
  role: String,
  content: String,
}

#[derive(Debug, Deserialize)]
struct ClaudeMessagesResponse {
  #[serde(default)]
  model: Option<String>,
  #[serde(default)]
  content: Vec<ClaudeContentBlock>,
  #[serde(default)]
  usage: Option<ClaudeUsage>,
  #[serde(default)]
  error: Option<ClaudeErrorBody>,
}

#[derive(Debug, Deserialize)]
struct ClaudeContentBlock {
  #[serde(rename = "type", default)]
  block_type: String,
  #[serde(default)]
  text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ClaudeUsage {
  #[serde(default)]
  input_tokens: Option<u32>,
  #[serde(default)]
  output_tokens: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct ClaudeErrorBody {
  #[serde(default)]
  message: Option<String>,
}

impl ClaudeProvider {
  /// Execute using this provider's in-memory [`super::StoredAuth`] session.
  pub async fn execute(&self, req: &ExecRequest) -> AppResult<ExecResponse> {
    let mut body = self.build_body(req);
    body.stream = false;
    let http_resp = self.send_messages(&body, false).await?;
    let value: ClaudeMessagesResponse = read_json_response("claude", http_resp).await?;
    self.parse_response(&req.model, value)
  }

  /// Stream Messages API SSE (`content_block_delta` text deltas).
  pub async fn execute_stream(&self, req: &ExecRequest) -> AppResult<ExecStream> {
    let mut body = self.build_body(req);
    body.stream = true;
    let mut http_resp = self.send_messages(&body, true).await?;
    let status = http_resp.status().as_u16();
    if !(200..300).contains(&status) {
      let text = http_resp.text().await.unwrap_or_default();
      return Err(upstream_error("claude", status, &text));
    }

    let model = req.model.clone();
    let (tx, rx) = exec_stream_channel(64);
    tokio::spawn(async move {
      let mut parser = SseDataParser::new();
      let mut usage: Option<Usage> = None;
      let mut done = false;
      let mut fatal: Option<AppError> = None;

      loop {
        let chunk = match http_resp.chunk().await {
          Ok(Some(c)) => c,
          Ok(None) => break,
          Err(e) => {
            fatal = Some(AppError::ProviderFailed(format!(
              "claude: stream read: {e}"
            )));
            break;
          }
        };
        for data in parser.push(&chunk) {
          if data == "[DONE]" {
            done = true;
            break;
          }
          let Ok(v) = serde_json::from_str::<serde_json::Value>(&data) else {
            continue;
          };
          let event_type = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
          match event_type {
            "content_block_delta" => {
              let text = v
                .pointer("/delta/text")
                .and_then(|t| t.as_str())
                .unwrap_or("");
              if !text.is_empty() {
                if tx
                  .send(Ok(ExecStreamEvent::ContentDelta {
                    text: text.to_string(),
                  }))
                  .await
                  .is_err()
                {
                  return;
                }
              }
            }
            "message_delta" => {
              if let Some(out) = v.pointer("/usage/output_tokens").and_then(|n| n.as_u64()) {
                let input = v
                  .pointer("/usage/input_tokens")
                  .and_then(|n| n.as_u64())
                  .unwrap_or(0) as u32;
                let output = out as u32;
                usage = Some(Usage {
                  prompt_tokens: input,
                  completion_tokens: output,
                  total_tokens: input.saturating_add(output),
                });
              }
            }
            "message_start" => {
              if let Some(m) = v.pointer("/message/model").and_then(|t| t.as_str()) {
                let _ = tx
                  .send(Ok(ExecStreamEvent::Meta {
                    model: Some(m.to_string()),
                    id: v
                      .pointer("/message/id")
                      .and_then(|t| t.as_str())
                      .map(|s| s.to_string()),
                  }))
                  .await;
              }
              if let Some(input) = v
                .pointer("/message/usage/input_tokens")
                .and_then(|n| n.as_u64())
              {
                usage = Some(Usage {
                  prompt_tokens: input as u32,
                  completion_tokens: usage.as_ref().map(|u| u.completion_tokens).unwrap_or(0),
                  total_tokens: input as u32,
                });
              }
            }
            "message_stop" => {
              done = true;
            }
            "error" => {
              let msg = v
                .pointer("/error/message")
                .and_then(|t| t.as_str())
                .unwrap_or("claude stream error");
              fatal = Some(AppError::ProviderFailed(msg.into()));
            }
            _ => {}
          }
          if done || fatal.is_some() {
            break;
          }
        }
        if done || fatal.is_some() {
          break;
        }
      }

      if let Some(e) = fatal {
        let _ = tx.send(Err(e)).await;
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

  async fn send_messages(
    &self,
    body: &ClaudeMessagesRequest,
    stream: bool,
  ) -> AppResult<reqwest::Response> {
    let creds = self.session().await?;
    let token = creds.require_access_token()?;
    let url = format!("{DEFAULT_BASE}/v1/messages");
    let accept = if stream {
      "text/event-stream"
    } else {
      "application/json"
    };
    self
      .http
      .post(&url)
      .header("Content-Type", "application/json")
      .header("Authorization", format!("Bearer {token}"))
      .header("anthropic-version", ANTHROPIC_VERSION)
      .header("Accept", accept)
      .json(body)
      .send()
      .await
      .map_err(|e| AppError::ProviderFailed(format!("claude request failed: {e}")))
  }

  fn build_body(&self, req: &ExecRequest) -> ClaudeMessagesRequest {
    let (system, turns) = req.system_and_turns();
    let messages = turns
      .into_iter()
      .map(|(role, text)| {
        let role = match role.as_str() {
          "assistant" | "model" => "assistant",
          _ => "user",
        };
        ClaudeMessage {
          role: role.to_string(),
          content: text,
        }
      })
      .collect();

    ClaudeMessagesRequest {
      model: req.model.clone(),
      max_tokens: req.max_tokens.unwrap_or(4096),
      messages,
      stream: req.stream,
      system,
      temperature: req.temperature,
    }
  }

  fn parse_response(&self, model: &str, value: ClaudeMessagesResponse) -> AppResult<ExecResponse> {
    let mut content = String::new();
    for part in &value.content {
      if part.block_type == "text" {
        if let Some(t) = &part.text {
          content.push_str(t);
        }
      }
    }
    if content.is_empty() {
      if let Some(msg) = value.error.as_ref().and_then(|e| e.message.as_deref()) {
        return Err(AppError::ProviderFailed(format!("claude: {msg}")));
      }
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
    let resp_model = value.model.unwrap_or_else(|| model.to_string());

    Ok(ExecResponse::new(resp_model, content).with_usage(prompt, completion))
  }
}
