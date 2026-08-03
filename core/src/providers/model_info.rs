//! Shared model type used by the API and chat validation.
//!
//! Providers decode upstream HTTP into **their own** structs, persist those,
//! and convert to [`ModelInfo`] only when the catalog is consumed.

use crate::models::anthropic;
use crate::models::openai;

/// Normalized model entry for Teapot surfaces (OpenAI / Anthropic lists, checks).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelInfo {
  pub id: String,
  pub owned_by: String,
  pub created: i64,
  pub display_name: String,
}

impl ModelInfo {
  pub fn new(
    id: impl Into<String>,
    owned_by: impl Into<String>,
    created: i64,
    display_name: impl Into<String>,
  ) -> Self {
    Self {
      id: id.into(),
      owned_by: owned_by.into(),
      created,
      display_name: display_name.into(),
    }
  }

  pub fn to_openai(&self) -> openai::Model {
    openai::Model::new(self.id.clone(), self.created, self.owned_by.clone())
  }

  pub fn to_anthropic(&self) -> anthropic::ModelObject {
    anthropic::ModelObject {
      object_type: "model",
      id: self.id.clone(),
      display_name: self.display_name.clone(),
      created_at: chrono::DateTime::from_timestamp(self.created, 0)
        .unwrap_or_else(|| chrono::DateTime::UNIX_EPOCH)
        .to_rfc3339(),
    }
  }
}

/// Provider-native model that can be converted to [`ModelInfo`].
pub trait ProviderModel: Clone + Send + Sync {
  fn model_id(&self) -> &str;
  fn to_model_info(&self) -> ModelInfo;
}

/// Look up by id in a slice of provider-native models (case-sensitive then insensitive).
pub fn find_provider_model<'a, T: ProviderModel>(catalog: &'a [T], id: &str) -> Option<&'a T> {
  catalog.iter().find(|m| m.model_id() == id).or_else(|| {
    catalog
      .iter()
      .find(|m| m.model_id().eq_ignore_ascii_case(id))
  })
}

/// Convert a native catalog to shared [`ModelInfo`] values.
pub fn to_model_infos<T: ProviderModel>(catalog: &[T]) -> Vec<ModelInfo> {
  catalog.iter().map(|m| m.to_model_info()).collect()
}

/// Look up and convert one model.
pub fn find_model_info<T: ProviderModel>(catalog: &[T], id: &str) -> Option<ModelInfo> {
  find_provider_model(catalog, id).map(|m| m.to_model_info())
}
