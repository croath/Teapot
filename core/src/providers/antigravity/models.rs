//! Antigravity models via upstream API: POST /v1internal:fetchAvailableModels

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::providers::execute::read_json_response;
use crate::providers::model_info::{ModelInfo, ProviderModel, find_provider_model};

use super::AntigravityProvider;

const BASE_URLS: &[&str] = &[
  "https://cloudcode-pa.googleapis.com",
  "https://daily-cloudcode-pa.googleapis.com",
];
const MODELS_PATH: &str = "/v1internal:fetchAvailableModels";
const USER_AGENT: &str = "antigravity/hub/2.2.1";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AntigravityModelsResponse {
  #[serde(default)]
  pub models: HashMap<String, AntigravityModelFields>,
  #[serde(default, alias = "webSearchModelIds")]
  pub web_search_model_ids: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AntigravityModelFields {
  #[serde(default, alias = "displayName")]
  pub display_name: Option<String>,
  #[serde(default, alias = "maxTokens")]
  pub max_tokens: Option<i64>,
  #[serde(default, alias = "maxOutputTokens")]
  pub max_output_tokens: Option<i64>,
}

/// Stored native model (map key + fields).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AntigravityModel {
  pub id: String,
  #[serde(default)]
  pub display_name: Option<String>,
  #[serde(default)]
  pub max_tokens: Option<i64>,
  #[serde(default)]
  pub max_output_tokens: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
struct FetchModelsRequest {
  #[serde(skip_serializing_if = "Option::is_none")]
  project: Option<String>,
}

impl ProviderModel for AntigravityModel {
  fn model_id(&self) -> &str {
    self.id.as_str()
  }

  fn to_model_info(&self) -> ModelInfo {
    let display_name = self
      .display_name
      .clone()
      .filter(|s| !s.is_empty())
      .unwrap_or_else(|| self.id.clone());
    ModelInfo::new(self.id.clone(), "antigravity", 0, display_name)
  }
}

impl AntigravityProvider {
  /// List models using this provider's in-memory [`super::StoredAuth`] session.
  pub async fn models(&self) -> AppResult<Vec<AntigravityModel>> {
    let creds = self.session().await?;
    let token = creds.require_access_token()?;
    let body = FetchModelsRequest {
      project: creds
        .project_id
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty()),
    };

    let mut last_err = AppError::ProviderFailed("antigravity models: no base URL succeeded".into());
    for base in BASE_URLS {
      let url = format!("{base}{MODELS_PATH}");
      let http_resp = match self
        .http
        .post(&url)
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {token}"))
        .header("User-Agent", USER_AGENT)
        .header("Accept", "application/json")
        .json(&body)
        .send()
        .await
      {
        Ok(r) => r,
        Err(e) => {
          last_err = AppError::ProviderFailed(format!("antigravity models request failed: {e}"));
          continue;
        }
      };

      match read_json_response::<AntigravityModelsResponse>("antigravity", http_resp).await {
        Ok(resp) => {
          let list = flatten_antigravity_models(resp);
          if !list.is_empty() {
            return Ok(list);
          }
          last_err =
            AppError::ProviderFailed("antigravity models: empty models map in response".into());
        }
        Err(e) => last_err = e,
      }
    }
    Err(last_err)
  }

  pub async fn model(&self, id: &str) -> AppResult<Option<AntigravityModel>> {
    let list = self.models().await?;
    Ok(find_provider_model(&list, id).cloned())
  }
}

fn flatten_antigravity_models(resp: AntigravityModelsResponse) -> Vec<AntigravityModel> {
  let mut out = Vec::new();
  for (id, fields) in resp.models {
    let id = id.trim().to_string();
    if id.is_empty() {
      continue;
    }
    match id.as_str() {
      "chat_20706"
      | "chat_23310"
      | "tab_flash_lite_preview"
      | "tab_jump_flash_lite_preview"
      | "gemini-2.5-flash-thinking" => continue,
      _ => {}
    }
    out.push(AntigravityModel {
      id,
      display_name: fields.display_name,
      max_tokens: fields.max_tokens,
      max_output_tokens: fields.max_output_tokens,
    });
  }
  out.sort_by(|a, b| a.id.cmp(&b.id));
  out
}
