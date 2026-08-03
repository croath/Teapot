//! xAI models via upstream API: GET /v1/models

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::providers::execute::read_json_response;
use crate::providers::model_info::{ModelInfo, ProviderModel, find_provider_model};

use super::XaiProvider;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XaiModelsResponse {
  #[serde(default)]
  pub data: Vec<XaiModel>,
  #[serde(default)]
  pub object: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XaiModel {
  pub id: String,
  #[serde(default)]
  pub object: Option<String>,
  #[serde(default)]
  pub created: Option<i64>,
  #[serde(default)]
  pub owned_by: Option<String>,
}

impl ProviderModel for XaiModel {
  fn model_id(&self) -> &str {
    self.id.as_str()
  }

  fn to_model_info(&self) -> ModelInfo {
    let owned_by = self.owned_by.clone().unwrap_or_else(|| "xai".to_string());
    ModelInfo::new(
      self.id.clone(),
      owned_by,
      self.created.unwrap_or(0),
      self.id.clone(),
    )
  }
}

impl XaiProvider {
  /// List models using this provider's in-memory [`super::StoredAuth`] session.
  pub async fn models(&self) -> AppResult<Vec<XaiModel>> {
    let creds = self.session().await?;
    let token = creds.require_access_token()?;
    let base = creds.api_base().trim_end_matches('/');

    let url = format!("{base}/models");
    let http_resp = self
      .http
      .get(&url)
      .header("Accept", "application/json")
      .header("Authorization", format!("Bearer {token}"))
      .send()
      .await
      .map_err(|e| AppError::ProviderFailed(format!("xai models request failed: {e}")))?;

    let resp: XaiModelsResponse = read_json_response("xai", http_resp).await?;
    Ok(resp.data.into_iter().filter(|m| !m.id.is_empty()).collect())
  }

  pub async fn model(&self, id: &str) -> AppResult<Option<XaiModel>> {
    let creds = self.session().await?;
    let token = creds.require_access_token()?;
    let base = creds.api_base().trim_end_matches('/');

    let url = format!("{base}/models/{id}");
    let http_resp = self
      .http
      .get(&url)
      .header("Accept", "application/json")
      .header("Authorization", format!("Bearer {token}"))
      .send()
      .await
      .map_err(|e| AppError::ProviderFailed(format!("xai model request failed: {e}")))?;

    if http_resp.status().as_u16() == 404 {
      let list = self.models().await?;
      return Ok(find_provider_model(&list, id).cloned());
    }

    let model: XaiModel = read_json_response("xai", http_resp).await?;
    if model.id.is_empty() {
      return Ok(None);
    }
    Ok(Some(model))
  }
}
