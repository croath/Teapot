//! Antigravity HTTP request execution (generateContent) — sync + stream.

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::providers::execute::{
  ExecRequest, ExecResponse, ExecStream, read_json_response, synthetic_stream_from_response,
};

use super::{AntigravityProvider, StoredAuth};

const DEFAULT_BASE: &str = "https://cloudcode-pa.googleapis.com";
const GENERATE_PATH: &str = "/v1internal:generateContent";
const USER_AGENT: &str = "antigravity/hub/2.2.1";

#[derive(Debug, Serialize)]
struct AntigravityGenerateRequest {
  model: String,
  #[serde(rename = "userAgent")]
  user_agent: &'static str,
  #[serde(rename = "requestType")]
  request_type: &'static str,
  request: GeminiGenerateBody,
  #[serde(skip_serializing_if = "Option::is_none")]
  project: Option<String>,
}

#[derive(Debug, Serialize)]
struct GeminiGenerateBody {
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
struct AntigravityGenerateResponse {
  #[serde(default)]
  response: Option<GeminiGenerateResult>,
  #[serde(default)]
  candidates: Vec<GeminiCandidate>,
  #[serde(default, rename = "usageMetadata")]
  usage_metadata: Option<GeminiUsageMetadata>,
  #[serde(default)]
  error: Option<ApiErrorMessage>,
}

#[derive(Debug, Deserialize)]
struct GeminiGenerateResult {
  #[serde(default)]
  candidates: Vec<GeminiCandidate>,
  #[serde(default, rename = "usageMetadata")]
  usage_metadata: Option<GeminiUsageMetadata>,
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

impl AntigravityProvider {
  /// Execute using this provider's in-memory [`StoredAuth`] session.
  pub async fn execute(&self, req: &ExecRequest) -> AppResult<ExecResponse> {
    let creds = self.session().await?;
    let token = creds.require_access_token()?;
    let body = self.build_body(&creds, req);
    let url = format!("{DEFAULT_BASE}{GENERATE_PATH}");

    let http_resp = self
      .http
      .post(&url)
      .header("Content-Type", "application/json")
      .header("Authorization", format!("Bearer {token}"))
      .header("User-Agent", USER_AGENT)
      .header("Accept", "application/json")
      .json(&body)
      .send()
      .await
      .map_err(|e| AppError::ProviderFailed(format!("antigravity request failed: {e}")))?;

    let value: AntigravityGenerateResponse = read_json_response("antigravity", http_resp).await?;
    self.parse_response(&req.model, value)
  }

  /// Stream: upstream has no stable public SSE path here; synthesize from full response.
  pub async fn execute_stream(&self, req: &ExecRequest) -> AppResult<ExecStream> {
    let full = self.execute(&req.as_non_stream()).await?;
    Ok(synthetic_stream_from_response(full))
  }

  fn build_body(&self, creds: &StoredAuth, req: &ExecRequest) -> AntigravityGenerateRequest {
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

    AntigravityGenerateRequest {
      model: req.model.clone(),
      user_agent: "antigravity",
      request_type: "agent",
      request: GeminiGenerateBody {
        contents,
        system_instruction,
        generation_config,
      },
      project: creds
        .project_id
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty()),
    }
  }

  fn parse_response(
    &self,
    model: &str,
    value: AntigravityGenerateResponse,
  ) -> AppResult<ExecResponse> {
    let content = candidate_text(&value);
    if content.is_empty() {
      if let Some(msg) = value.error.as_ref().and_then(|e| e.message.as_deref()) {
        return Err(AppError::ProviderFailed(format!("antigravity: {msg}")));
      }
    }
    let (prompt, completion) = usage_tokens(&value);
    Ok(ExecResponse::new(model, content).with_usage(prompt, completion))
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

fn candidate_text(value: &AntigravityGenerateResponse) -> String {
  let candidates = if !value.candidates.is_empty() {
    &value.candidates
  } else if let Some(resp) = &value.response {
    &resp.candidates
  } else {
    return String::new();
  };
  let mut out = String::new();
  for cand in candidates {
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

fn usage_tokens(value: &AntigravityGenerateResponse) -> (u32, u32) {
  let meta = value.usage_metadata.as_ref().or_else(|| {
    value
      .response
      .as_ref()
      .and_then(|r| r.usage_metadata.as_ref())
  });
  let prompt = meta.and_then(|m| m.prompt_token_count).unwrap_or(0);
  let completion = meta.and_then(|m| m.candidates_token_count).unwrap_or(0);
  (prompt, completion)
}
