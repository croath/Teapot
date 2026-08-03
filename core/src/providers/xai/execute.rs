//! xAI HTTP request execution — Chat Completions (sync + stream).

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::models::openai::Usage;
use crate::providers::execute::{
  ExecRequest, ExecResponse, ExecStream, ExecStreamEvent, SseDataParser, exec_stream_channel,
  read_json_response, upstream_error,
};

use super::XaiProvider;

#[derive(Debug, Serialize)]
struct XaiChatRequest {
  model: String,
  messages: Vec<XaiChatMessage>,
  stream: bool,
  #[serde(skip_serializing_if = "Option::is_none")]
  temperature: Option<f32>,
  #[serde(skip_serializing_if = "Option::is_none")]
  max_tokens: Option<u32>,
}

#[derive(Debug, Serialize)]
struct XaiChatMessage {
  role: String,
  content: String,
}

#[derive(Debug, Deserialize)]
struct XaiChatResponse {
  #[serde(default)]
  id: Option<String>,
  #[serde(default)]
  model: Option<String>,
  #[serde(default)]
  choices: Vec<XaiChoice>,
  #[serde(default)]
  usage: Option<XaiUsage>,
  #[serde(default)]
  error: Option<XaiErrorBody>,
}

#[derive(Debug, Deserialize)]
struct XaiChoice {
  #[serde(default)]
  message: Option<XaiChatMessageOut>,
}

#[derive(Debug, Deserialize)]
struct XaiChatMessageOut {
  #[serde(default)]
  content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct XaiUsage {
  #[serde(default)]
  prompt_tokens: Option<u32>,
  #[serde(default)]
  completion_tokens: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct XaiErrorBody {
  #[serde(default)]
  message: Option<String>,
}

impl XaiProvider {
  /// Execute using this provider's in-memory [`super::StoredAuth`] session.
  pub async fn execute(&self, req: &ExecRequest) -> AppResult<ExecResponse> {
    let mut body = self.build_body(req);
    body.stream = false;
    let http_resp = self.send_chat(&body, false).await?;
    let value: XaiChatResponse = read_json_response("xai", http_resp).await?;
    self.parse_response(&req.model, value)
  }

  /// Stream OpenAI-compatible chat.completion.chunk SSE from xAI.
  pub async fn execute_stream(&self, req: &ExecRequest) -> AppResult<ExecStream> {
    let mut body = self.build_body(req);
    body.stream = true;
    let mut http_resp = self.send_chat(&body, true).await?;
    let status = http_resp.status().as_u16();
    if !(200..300).contains(&status) {
      let text = http_resp.text().await.unwrap_or_default();
      return Err(upstream_error("xai", status, &text));
    }

    let model = req.model.clone();
    let (tx, rx) = exec_stream_channel(64);
    tokio::spawn(async move {
      let mut parser = SseDataParser::new();
      let mut usage: Option<Usage> = None;
      let mut stream_id: Option<String> = None;
      let mut stream_model: Option<String> = Some(model);
      let mut done = false;
      let mut fatal: Option<AppError> = None;

      loop {
        let chunk = match http_resp.chunk().await {
          Ok(Some(c)) => c,
          Ok(None) => break,
          Err(e) => {
            fatal = Some(AppError::ProviderFailed(format!("xai: stream read: {e}")));
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
          if let Some(id) = v.get("id").and_then(|t| t.as_str()) {
            stream_id = Some(id.to_string());
          }
          if let Some(m) = v.get("model").and_then(|t| t.as_str()) {
            stream_model = Some(m.to_string());
          }
          if let Some(u) = v.get("usage") {
            let p = u.get("prompt_tokens").and_then(|n| n.as_u64()).unwrap_or(0) as u32;
            let c = u
              .get("completion_tokens")
              .and_then(|n| n.as_u64())
              .unwrap_or(0) as u32;
            usage = Some(Usage {
              prompt_tokens: p,
              completion_tokens: c,
              total_tokens: p.saturating_add(c),
            });
          }
          if let Some(choices) = v.get("choices").and_then(|c| c.as_array()) {
            for choice in choices {
              if let Some(text) = choice
                .pointer("/delta/content")
                .and_then(|t| t.as_str())
                .filter(|s| !s.is_empty())
              {
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
              if choice
                .get("finish_reason")
                .and_then(|t| t.as_str())
                .is_some()
              {
                done = true;
              }
            }
          }
          if let Some(msg) = v.pointer("/error/message").and_then(|t| t.as_str()) {
            fatal = Some(AppError::ProviderFailed(format!("xai: {msg}")));
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
          model: stream_model,
          id: stream_id,
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

  async fn send_chat(&self, body: &XaiChatRequest, stream: bool) -> AppResult<reqwest::Response> {
    let creds = self.session().await?;
    let token = creds.require_access_token()?;
    let base = creds.api_base().trim_end_matches('/');
    let url = format!("{base}/chat/completions");
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
      .header("Accept", accept)
      .json(body)
      .send()
      .await
      .map_err(|e| AppError::ProviderFailed(format!("xai request failed: {e}")))
  }

  fn build_body(&self, req: &ExecRequest) -> XaiChatRequest {
    let messages = req
      .messages
      .iter()
      .map(|m| XaiChatMessage {
        role: m.role.clone(),
        content: m.content.as_text(),
      })
      .collect();

    XaiChatRequest {
      model: req.model.clone(),
      messages,
      stream: req.stream,
      temperature: req.temperature,
      max_tokens: req.max_tokens,
    }
  }

  fn parse_response(&self, model: &str, value: XaiChatResponse) -> AppResult<ExecResponse> {
    let content = value
      .choices
      .first()
      .and_then(|c| c.message.as_ref())
      .and_then(|m| m.content.clone())
      .unwrap_or_default();

    if content.is_empty() {
      if let Some(msg) = value.error.as_ref().and_then(|e| e.message.as_deref()) {
        return Err(AppError::ProviderFailed(format!("xai: {msg}")));
      }
    }

    let prompt = value
      .usage
      .as_ref()
      .and_then(|u| u.prompt_tokens)
      .unwrap_or(0);
    let completion = value
      .usage
      .as_ref()
      .and_then(|u| u.completion_tokens)
      .unwrap_or(0);
    let resp_model = value.model.unwrap_or_else(|| model.to_string());

    let mut resp = ExecResponse::new(resp_model, content).with_usage(prompt, completion);
    if let Some(id) = value.id {
      resp.id = id;
    }
    Ok(resp)
  }
}
