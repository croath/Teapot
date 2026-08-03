//! Vertex AI models via publisher models list API.

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::providers::execute::read_json_response;
use crate::providers::model_info::{ModelInfo, ProviderModel, find_provider_model};

use super::VertexProvider;

const API_VERSION: &str = "v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VertexModelsResponse {
  #[serde(default, alias = "publisherModels")]
  pub publisher_models: Vec<VertexModel>,
  #[serde(default)]
  pub models: Vec<VertexModel>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VertexModel {
  #[serde(default)]
  pub name: String,
  #[serde(default, alias = "displayName")]
  pub display_name: Option<String>,
  #[serde(default)]
  pub description: Option<String>,
  #[serde(default, alias = "versionId")]
  pub version_id: Option<String>,
}

impl ProviderModel for VertexModel {
  fn model_id(&self) -> &str {
    if self.name.is_empty() {
      return "";
    }
    self.name.rsplit('/').next().unwrap_or(self.name.as_str())
  }

  fn to_model_info(&self) -> ModelInfo {
    let id = self.model_id().to_string();
    let display_name = self
      .display_name
      .clone()
      .filter(|s| !s.is_empty())
      .unwrap_or_else(|| id.clone());
    ModelInfo::new(id, "google", 0, display_name)
  }
}

impl VertexProvider {
  /// List models using this provider's in-memory [`super::VertexSession`].
  pub async fn models(&self) -> AppResult<Vec<VertexModel>> {
    let creds = self.session().await?;
    let token = creds.require_access_token()?;
    let project = creds.project_id();
    let location = creds.location();

    let base = vertex_base_url(location);
    let urls = [
      format!(
        "{base}/{API_VERSION}/projects/{project}/locations/{location}/publishers/google/models"
      ),
      format!("{base}/{API_VERSION}/publishers/google/models"),
    ];

    let mut last_err = AppError::ProviderFailed("vertex models: no list endpoint succeeded".into());
    for url in &urls {
      let http_resp = match self
        .http
        .get(url)
        .header("Authorization", format!("Bearer {token}"))
        .header("Accept", "application/json")
        .send()
        .await
      {
        Ok(r) => r,
        Err(e) => {
          last_err = AppError::ProviderFailed(format!("vertex models request failed: {e}"));
          continue;
        }
      };

      match read_json_response::<VertexModelsResponse>("vertex", http_resp).await {
        Ok(resp) => {
          let list = collect_vertex_models(resp);
          if !list.is_empty() {
            return Ok(list);
          }
          last_err = AppError::ProviderFailed("vertex models: empty publisherModels".into());
        }
        Err(e) => last_err = e,
      }
    }
    Err(last_err)
  }

  pub async fn model(&self, id: &str) -> AppResult<Option<VertexModel>> {
    let creds = self.session().await?;
    let token = creds.require_access_token()?;
    let project = creds.project_id();
    let location = creds.location();

    let base = vertex_base_url(location);
    let model = id
      .trim_start_matches("models/")
      .trim_start_matches("publishers/google/models/");

    let url = format!(
      "{base}/{API_VERSION}/projects/{project}/locations/{location}/publishers/google/models/{model}"
    );
    let http_resp = self
      .http
      .get(&url)
      .header("Authorization", format!("Bearer {token}"))
      .header("Accept", "application/json")
      .send()
      .await
      .map_err(|e| AppError::ProviderFailed(format!("vertex model request failed: {e}")))?;

    if http_resp.status().as_u16() == 404 {
      let list = self.models().await?;
      return Ok(
        find_provider_model(&list, id)
          .or_else(|| find_provider_model(&list, model))
          .cloned(),
      );
    }

    let model: VertexModel = read_json_response("vertex", http_resp).await?;
    if model.model_id().is_empty() {
      return Ok(None);
    }
    Ok(Some(model))
  }
}

fn vertex_base_url(location: &str) -> String {
  if location == "global" {
    "https://aiplatform.googleapis.com".into()
  } else {
    format!("https://{location}-aiplatform.googleapis.com")
  }
}

fn collect_vertex_models(resp: VertexModelsResponse) -> Vec<VertexModel> {
  let mut out = resp.publisher_models;
  out.extend(resp.models);
  out
    .into_iter()
    .filter(|m| !m.model_id().is_empty())
    .collect()
}
