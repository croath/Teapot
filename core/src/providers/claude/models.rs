//! Claude models via upstream API: GET /v1/models

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::providers::execute::read_json_response;
use crate::providers::model_info::{ModelInfo, ProviderModel, find_provider_model};

use super::ClaudeProvider;

const DEFAULT_BASE: &str = "https://api.anthropic.com";
const ANTHROPIC_VERSION: &str = "2023-06-01";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudeModelsResponse {
  #[serde(default)]
  pub data: Vec<ClaudeModel>,
  #[serde(default)]
  pub has_more: bool,
  #[serde(default)]
  pub first_id: Option<String>,
  #[serde(default)]
  pub last_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudeModel {
  pub id: String,
  #[serde(default)]
  pub display_name: String,
  #[serde(default)]
  pub created_at: Option<String>,
  #[serde(default, rename = "type")]
  pub object_type: Option<String>,
}

impl ProviderModel for ClaudeModel {
  fn model_id(&self) -> &str {
    self.id.as_str()
  }

  fn to_model_info(&self) -> ModelInfo {
    let display_name = if self.display_name.is_empty() {
      self.id.clone()
    } else {
      self.display_name.clone()
    };
    let created = self
      .created_at
      .as_deref()
      .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
      .map(|dt| dt.timestamp())
      .unwrap_or(0);
    ModelInfo::new(self.id.clone(), "anthropic", created, display_name)
  }
}

impl ClaudeProvider {
  /// List models using this provider's in-memory [`super::StoredAuth`] session.
  pub async fn models(&self) -> AppResult<Vec<ClaudeModel>> {
    let creds = self.session().await?;
    let token = creds.require_access_token()?;
    let mut out = Vec::new();
    let mut after_id: Option<String> = None;

    loop {
      let mut url = format!("{DEFAULT_BASE}/v1/models?limit=100");
      if let Some(aid) = &after_id {
        url.push_str("&after_id=");
        url.push_str(aid);
      }

      let http_resp = self
        .http
        .get(&url)
        .header("Accept", "application/json")
        .header("Authorization", format!("Bearer {token}"))
        .header("anthropic-version", ANTHROPIC_VERSION)
        .send()
        .await
        .map_err(|e| AppError::ProviderFailed(format!("claude models request failed: {e}")))?;

      let page: ClaudeModelsResponse = read_json_response("claude", http_resp).await?;
      if page.data.is_empty() {
        break;
      }
      let last_id = page
        .last_id
        .clone()
        .or_else(|| page.data.last().map(|m| m.id.clone()));
      out.extend(page.data.into_iter().filter(|m| !m.id.is_empty()));

      if !page.has_more {
        break;
      }
      match last_id {
        Some(id) => after_id = Some(id),
        None => break,
      }
    }

    Ok(out)
  }

  pub async fn model(&self, id: &str) -> AppResult<Option<ClaudeModel>> {
    let creds = self.session().await?;
    let token = creds.require_access_token()?;
    let url = format!("{DEFAULT_BASE}/v1/models/{}", urlencoding_path(id));
    let http_resp = self
      .http
      .get(&url)
      .header("Accept", "application/json")
      .header("Authorization", format!("Bearer {token}"))
      .header("anthropic-version", ANTHROPIC_VERSION)
      .send()
      .await
      .map_err(|e| AppError::ProviderFailed(format!("claude model request failed: {e}")))?;

    if http_resp.status().as_u16() == 404 {
      let list = self.models().await?;
      return Ok(find_provider_model(&list, id).cloned());
    }

    let model: ClaudeModel = read_json_response("claude", http_resp).await?;
    if model.id.is_empty() {
      return Ok(None);
    }
    Ok(Some(model))
  }
}

fn urlencoding_path(value: &str) -> String {
  let mut out = String::with_capacity(value.len());
  for b in value.bytes() {
    match b {
      b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
      _ => {
        out.push('%');
        out.push(char::from(b"0123456789ABCDEF"[(b >> 4) as usize]));
        out.push(char::from(b"0123456789ABCDEF"[(b & 0xf) as usize]));
      }
    }
  }
  out
}
