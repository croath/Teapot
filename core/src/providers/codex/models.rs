//! Codex models via upstream API: GET /backend-api/codex/models

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::providers::execute::read_json_response;
use crate::providers::model_info::{ModelInfo, ProviderModel, find_provider_model};

use super::CodexProvider;

const DEFAULT_BASE: &str = "https://chatgpt.com/backend-api/codex";
const CLIENT_VERSION: &str = "0.144.1";
const USER_AGENT: &str = "codex_cli_rs/0.144.1 (Mac OS 26.3.1; arm64) iTerm.app/3.6.9";
const ORIGINATOR: &str = "codex_cli_rs";

/// Upstream list payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexModelsResponse {
  #[serde(default)]
  pub models: Vec<CodexModel>,
}

/// One Codex upstream model (fields kept for local storage).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexModel {
  #[serde(default)]
  pub slug: String,
  #[serde(default)]
  pub display_name: String,
  #[serde(default)]
  pub description: Option<String>,
  #[serde(default)]
  pub context_window: Option<i64>,
  #[serde(default)]
  pub max_context_window: Option<i64>,
  #[serde(default)]
  pub priority: Option<i64>,
  #[serde(default)]
  pub visibility: Option<String>,
  #[serde(default)]
  pub default_reasoning_level: Option<String>,
  #[serde(default)]
  pub minimal_client_version: Option<String>,
}

impl ProviderModel for CodexModel {
  fn model_id(&self) -> &str {
    self.slug.as_str()
  }

  fn to_model_info(&self) -> ModelInfo {
    let id = self.slug.clone();
    let display_name = if self.display_name.is_empty() {
      id.clone()
    } else {
      self.display_name.clone()
    };
    ModelInfo::new(id, "openai", 0, display_name)
  }
}

impl CodexProvider {
  /// List models using this provider's in-memory [`super::StoredAuth`] session.
  pub async fn models(&self) -> AppResult<Vec<CodexModel>> {
    let creds = self.session().await?;
    let token = creds.require_access_token()?;
    let url = format!("{DEFAULT_BASE}/models?client_version={CLIENT_VERSION}");
    let mut req = self
      .http
      .get(&url)
      .header("Accept", "application/json")
      .header("Authorization", format!("Bearer {token}"))
      .header("Originator", ORIGINATOR)
      .header("User-Agent", USER_AGENT);

    if let Some(id) = creds.account_id.as_deref().filter(|s| !s.is_empty()) {
      req = req.header("Chatgpt-Account-Id", id);
    }

    let http_resp = req
      .send()
      .await
      .map_err(|e| AppError::ProviderFailed(format!("codex models request failed: {e}")))?;

    let resp: CodexModelsResponse = read_json_response("codex", http_resp).await?;
    Ok(
      resp
        .models
        .into_iter()
        .filter(|m| !m.model_id().is_empty())
        .collect(),
    )
  }

  pub async fn model(&self, id: &str) -> AppResult<Option<CodexModel>> {
    let list = self.models().await?;
    Ok(find_provider_model(&list, id).cloned())
  }
}
