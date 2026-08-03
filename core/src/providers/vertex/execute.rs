//! Vertex AI HTTP request execution (generateContent / streamGenerateContent).

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::models::openai::Usage;
use crate::providers::execute::{
  ExecRequest, ExecResponse, ExecStream, ExecStreamEvent, SseDataParser, exec_stream_channel,
  read_json_response, synthetic_stream_from_response,
};
use crate::providers::vertex::auth::ServiceAccount;

use super::VertexProvider;

const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const SCOPE: &str = "https://www.googleapis.com/auth/cloud-platform";
const API_VERSION: &str = "v1";

#[derive(Debug, Serialize)]
struct VertexGenerateRequest {
  contents: Vec<GeminiContent>,
  #[serde(rename = "systemInstruction", skip_serializing_if = "Option::is_none")]
  system_instruction: Option<GeminiSystemInstruction>,
  #[serde(rename = "generationConfig", skip_serializing_if = "Option::is_none")]
  generation_config: Option<GeminiGenerationConfig>,
}

#[derive(Debug, Serialize, Deserialize)]
struct GeminiContent {
  role: String,
  parts: Vec<GeminiPart>,
}

#[derive(Debug, Serialize, Deserialize)]
struct GeminiPart {
  #[serde(default)]
  text: Option<String>,
}

#[derive(Debug, Serialize)]
struct GeminiSystemInstruction {
  parts: Vec<GeminiPart>,
}

#[derive(Debug, Serialize)]
struct GeminiGenerationConfig {
  #[serde(skip_serializing_if = "Option::is_none")]
  temperature: Option<f32>,
  #[serde(rename = "maxOutputTokens", skip_serializing_if = "Option::is_none")]
  max_output_tokens: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct VertexGenerateResponse {
  #[serde(default)]
  candidates: Vec<GeminiCandidate>,
  #[serde(default, rename = "usageMetadata")]
  usage_metadata: Option<GeminiUsageMetadata>,
  #[serde(default)]
  error: Option<ApiErrorMessage>,
}

#[derive(Debug, Deserialize)]
struct GeminiCandidate {
  #[serde(default)]
  content: Option<GeminiContent>,
}

#[derive(Debug, Deserialize)]
struct GeminiUsageMetadata {
  #[serde(default, rename = "promptTokenCount")]
  prompt_token_count: Option<u32>,
  #[serde(default, rename = "candidatesTokenCount")]
  candidates_token_count: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct ApiErrorMessage {
  #[serde(default)]
  message: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TokenResp {
  access_token: String,
}

#[derive(Serialize)]
struct SaClaims {
  iss: String,
  scope: String,
  aud: String,
  iat: i64,
  exp: i64,
}

impl VertexProvider {
  /// Execute using this provider's in-memory [`super::VertexSession`].
  pub async fn execute(&self, req: &ExecRequest) -> AppResult<ExecResponse> {
    let http_resp = self.send_generate(req, false).await?;
    let value: VertexGenerateResponse = read_json_response("vertex", http_resp).await?;
    self.parse_response(&req.model, value)
  }

  /// Stream via `:streamGenerateContent?alt=sse` when available; falls back to synthetic.
  pub async fn execute_stream(&self, req: &ExecRequest) -> AppResult<ExecStream> {
    match self.send_generate(req, true).await {
      Ok(mut http_resp) => {
        let status = http_resp.status().as_u16();
        if !(200..300).contains(&status) {
          // Fall back to non-stream if streaming endpoint fails.
          let text = http_resp.text().await.unwrap_or_default();
          tracing::debug!(status, body = %text.chars().take(200).collect::<String>(), "vertex stream failed; fallback");
          let full = self.execute(&req.as_non_stream()).await?;
          return Ok(synthetic_stream_from_response(full));
        }
        let model = req.model.clone();
        let (tx, rx) = exec_stream_channel(64);
        tokio::spawn(async move {
          let mut parser = SseDataParser::new();
          let mut usage: Option<Usage> = None;
          let mut fatal: Option<AppError> = None;
          loop {
            let chunk = match http_resp.chunk().await {
              Ok(Some(c)) => c,
              Ok(None) => break,
              Err(e) => {
                fatal = Some(AppError::ProviderFailed(format!(
                  "vertex: stream read: {e}"
                )));
                break;
              }
            };
            for data in parser.push(&chunk) {
              if data == "[DONE]" {
                continue;
              }
              let Ok(v) = serde_json::from_str::<serde_json::Value>(&data) else {
                continue;
              };
              // candidates[0].content.parts[].text
              if let Some(parts) = v
                .pointer("/candidates/0/content/parts")
                .and_then(|p| p.as_array())
              {
                for part in parts {
                  if let Some(text) = part
                    .get("text")
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
                }
              }
              if let Some(meta) = v.get("usageMetadata") {
                let p = meta
                  .get("promptTokenCount")
                  .and_then(|n| n.as_u64())
                  .unwrap_or(0) as u32;
                let c = meta
                  .get("candidatesTokenCount")
                  .and_then(|n| n.as_u64())
                  .unwrap_or(0) as u32;
                usage = Some(Usage {
                  prompt_tokens: p,
                  completion_tokens: c,
                  total_tokens: p.saturating_add(c),
                });
              }
              if let Some(msg) = v.pointer("/error/message").and_then(|t| t.as_str()) {
                fatal = Some(AppError::ProviderFailed(format!("vertex: {msg}")));
                break;
              }
            }
            if fatal.is_some() {
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
      Err(_) => {
        let full = self.execute(&req.as_non_stream()).await?;
        Ok(synthetic_stream_from_response(full))
      }
    }
  }

  async fn send_generate(&self, req: &ExecRequest, stream: bool) -> AppResult<reqwest::Response> {
    let creds = self.session().await?;
    let token = creds.require_access_token()?;
    let project = creds.project_id();
    let location = creds.location();

    let body = self.build_body(req);
    let base = vertex_base_url(location);
    let model = req.model.trim_start_matches("models/");
    let method = if stream {
      "streamGenerateContent"
    } else {
      "generateContent"
    };
    let url = if stream {
      format!(
        "{base}/{API_VERSION}/projects/{project}/locations/{location}/publishers/google/models/{model}:{method}?alt=sse"
      )
    } else {
      format!(
        "{base}/{API_VERSION}/projects/{project}/locations/{location}/publishers/google/models/{model}:{method}"
      )
    };
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
      .json(&body)
      .send()
      .await
      .map_err(|e| AppError::ProviderFailed(format!("vertex request failed: {e}")))
  }

  fn build_body(&self, req: &ExecRequest) -> VertexGenerateRequest {
    let (system, turns) = req.system_and_turns();
    let contents = contents_from_turns(&turns);
    let system_instruction = system.map(|sys| GeminiSystemInstruction {
      parts: vec![GeminiPart { text: Some(sys) }],
    });
    let generation_config = if req.temperature.is_some() || req.max_tokens.is_some() {
      Some(GeminiGenerationConfig {
        temperature: req.temperature,
        max_output_tokens: req.max_tokens,
      })
    } else {
      None
    };
    VertexGenerateRequest {
      contents,
      system_instruction,
      generation_config,
    }
  }

  fn parse_response(&self, model: &str, value: VertexGenerateResponse) -> AppResult<ExecResponse> {
    let content = candidate_text(&value);
    if content.is_empty() {
      if let Some(msg) = value.error.as_ref().and_then(|e| e.message.as_deref()) {
        return Err(AppError::ProviderFailed(format!("vertex: {msg}")));
      }
    }
    let prompt = value
      .usage_metadata
      .as_ref()
      .and_then(|m| m.prompt_token_count)
      .unwrap_or(0);
    let completion = value
      .usage_metadata
      .as_ref()
      .and_then(|m| m.candidates_token_count)
      .unwrap_or(0);
    Ok(ExecResponse::new(model, content).with_usage(prompt, completion))
  }

  pub(crate) async fn fetch_access_token(&self, sa: &ServiceAccount) -> AppResult<String> {
    let client_email = sa.client_email.trim();
    if client_email.is_empty() {
      return Err(AppError::Unauthorized(
        "vertex: service_account missing client_email".into(),
      ));
    }
    let private_key = sa.private_key.trim();
    if private_key.is_empty() {
      return Err(AppError::Unauthorized(
        "vertex: service_account missing private_key".into(),
      ));
    }
    let token_uri = sa
      .token_uri
      .as_deref()
      .map(str::trim)
      .filter(|s| !s.is_empty())
      .unwrap_or(TOKEN_URL);

    let assertion = sign_service_account_jwt(client_email, private_key, token_uri)?;

    let body = format!(
      "grant_type={}&assertion={}",
      urlencoding_form("urn:ietf:params:oauth:grant-type:jwt-bearer"),
      urlencoding_form(&assertion)
    );
    let resp = self
      .http
      .post(token_uri)
      .header("Content-Type", "application/x-www-form-urlencoded")
      .body(body)
      .send()
      .await
      .map_err(|e| AppError::ProviderFailed(format!("vertex token request failed: {e}")))?;

    let status = resp.status().as_u16();
    let text = resp
      .text()
      .await
      .map_err(|e| AppError::ProviderFailed(format!("vertex token body: {e}")))?;
    if !(200..300).contains(&status) {
      return Err(AppError::Unauthorized(format!(
        "vertex token exchange {status}: {text}"
      )));
    }
    let tok: TokenResp = serde_json::from_str(&text)
      .map_err(|e| AppError::ProviderFailed(format!("vertex token JSON: {e}")))?;
    Ok(tok.access_token)
  }
}

fn vertex_base_url(location: &str) -> String {
  if location == "global" {
    "https://aiplatform.googleapis.com".into()
  } else {
    format!("https://{location}-aiplatform.googleapis.com")
  }
}

fn contents_from_turns(turns: &[(String, String)]) -> Vec<GeminiContent> {
  turns
    .iter()
    .map(|(role, text)| {
      let role = match role.as_str() {
        "assistant" | "model" => "model",
        _ => "user",
      };
      GeminiContent {
        role: role.to_string(),
        parts: vec![GeminiPart {
          text: Some(text.clone()),
        }],
      }
    })
    .collect()
}

fn candidate_text(value: &VertexGenerateResponse) -> String {
  let mut out = String::new();
  for cand in &value.candidates {
    if let Some(content) = &cand.content {
      for part in &content.parts {
        if let Some(t) = &part.text {
          out.push_str(t);
        }
      }
    }
  }
  out
}

fn urlencoding_form(value: &str) -> String {
  let mut out = String::with_capacity(value.len());
  for b in value.bytes() {
    match b {
      b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
        out.push(b as char);
      }
      b' ' => out.push('+'),
      _ => {
        out.push('%');
        out.push(char::from(b"0123456789ABCDEF"[(b >> 4) as usize]));
        out.push(char::from(b"0123456789ABCDEF"[(b & 0xf) as usize]));
      }
    }
  }
  out
}

fn sign_service_account_jwt(
  client_email: &str,
  private_key_pem: &str,
  audience: &str,
) -> AppResult<String> {
  let now = SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .unwrap_or(Duration::from_secs(0))
    .as_secs() as i64;

  let claims = SaClaims {
    iss: client_email.to_string(),
    scope: SCOPE.to_string(),
    aud: audience.to_string(),
    iat: now,
    exp: now + 3600,
  };

  let key = EncodingKey::from_rsa_pem(private_key_pem.as_bytes())
    .map_err(|e| AppError::Internal(format!("vertex private_key: {e}")))?;
  encode(&Header::new(Algorithm::RS256), &claims, &key)
    .map_err(|e| AppError::Internal(format!("vertex jwt sign: {e}")))
}
