//! Shared model listing for ChatGPT and Claude compatible APIs.
//!
//! Only models from **installed** agent CLIs are returned.

use chrono::Utc;

use crate::agents::discovery::{DiscoveredModel, discover_models};
use crate::config::Config;
use crate::error::{AppError, AppResult};
use crate::models::anthropic as anth;
use crate::models::openai as oai;

/// Discover models backed by installed agent CLIs.
pub async fn list_discovered(config: &Config) -> Vec<DiscoveredModel> {
  discover_models(config).await
}

/// OpenAI-compatible `GET /models` body.
pub async fn openai_model_list(config: &Config) -> oai::ModelList {
  let created = Utc::now().timestamp();
  let models = list_discovered(config).await;
  let data = models
    .into_iter()
    .map(|m| oai::ModelObject {
      id: m.id,
      object: "model",
      created,
      owned_by: format!("teaport/{}", m.agent),
    })
    .collect();

  oai::ModelList {
    object: "list",
    data,
  }
}

/// OpenAI-compatible `GET /models/{id}`.
pub async fn openai_get_model(config: &Config, model_id: &str) -> AppResult<oai::ModelObject> {
  let created = Utc::now().timestamp();
  let models = list_discovered(config).await;
  models
    .into_iter()
    .find(|m| m.id == model_id)
    .map(|m| oai::ModelObject {
      id: m.id,
      object: "model",
      created,
      owned_by: format!("teaport/{}", m.agent),
    })
    .ok_or_else(|| AppError::AgentNotFound(format!("model not found: {model_id}")))
}

/// Anthropic-compatible `GET /models` body.
pub async fn anthropic_model_list(config: &Config) -> anth::ModelList {
  let created_at = Utc::now().to_rfc3339();
  let models = list_discovered(config).await;
  let data: Vec<anth::ModelObject> = models
    .into_iter()
    .map(|m| anth::ModelObject {
      object_type: "model",
      id: m.id,
      display_name: m.display_name,
      created_at: created_at.clone(),
    })
    .collect();

  let first_id = data.first().map(|m| m.id.clone());
  let last_id = data.last().map(|m| m.id.clone());

  anth::ModelList {
    data,
    has_more: false,
    first_id,
    last_id,
  }
}

/// Anthropic-compatible `GET /models/{id}`.
pub async fn anthropic_get_model(config: &Config, model_id: &str) -> AppResult<anth::ModelObject> {
  let created_at = Utc::now().to_rfc3339();
  let models = list_discovered(config).await;
  models
    .into_iter()
    .find(|m| m.id == model_id)
    .map(|m| anth::ModelObject {
      object_type: "model",
      id: m.id,
      display_name: m.display_name,
      created_at,
    })
    .ok_or_else(|| AppError::AgentNotFound(format!("model not found: {model_id}")))
}
