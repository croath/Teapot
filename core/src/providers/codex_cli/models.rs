//! Codex CLI models via app-server `model/list`.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::error::{AppError, AppResult};
use crate::providers::model_info::{ModelInfo, ProviderModel, find_provider_model};

use super::CodexCliProvider;

/// One model row from `model/list` (fields kept for local storage).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexCliModel {
  #[serde(default)]
  pub id: String,
  #[serde(default)]
  pub slug: String,
  #[serde(default)]
  pub model: String,
  #[serde(default, alias = "displayName")]
  pub display_name: String,
  #[serde(default)]
  pub hidden: Option<bool>,
}

impl CodexCliModel {
  fn resolved_id(&self) -> &str {
    if !self.id.is_empty() {
      &self.id
    } else if !self.slug.is_empty() {
      &self.slug
    } else {
      &self.model
    }
  }
}

impl ProviderModel for CodexCliModel {
  fn model_id(&self) -> &str {
    self.resolved_id()
  }

  fn to_model_info(&self) -> ModelInfo {
    let id = self.resolved_id().to_string();
    let display_name = if self.display_name.is_empty() {
      id.clone()
    } else {
      self.display_name.clone()
    };
    ModelInfo::new(id, "openai", 0, display_name)
  }
}

/// Parse a `model/list` JSON-RPC result into native rows.
pub fn parse_model_list(result: &Value) -> Vec<CodexCliModel> {
  let rows = result
    .get("data")
    .or_else(|| result.get("models"))
    .and_then(Value::as_array)
    .cloned()
    .unwrap_or_default();
  rows
    .into_iter()
    .filter_map(|row| serde_json::from_value::<CodexCliModel>(row).ok())
    .filter(|m| !m.model_id().is_empty())
    .collect()
}

fn next_cursor(result: &Value) -> Option<String> {
  result
    .get("nextCursor")
    .or_else(|| result.get("next_cursor"))
    .and_then(Value::as_str)
    .filter(|s| !s.is_empty())
    .map(str::to_string)
}

impl CodexCliProvider {
  pub async fn models(&self) -> AppResult<Vec<CodexCliModel>> {
    let mut guard = self.lock_session().await?;
    let session = Self::session_mut(&mut guard)?;
    let mut out: Vec<CodexCliModel> = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..20 {
      let mut params = json!({ "includeHidden": false });
      if let Some(c) = &cursor {
        params["cursor"] = json!(c);
      }
      let result = session.request("model/list", params).await?;
      let page = parse_model_list(&result);
      if page.is_empty() && cursor.is_none() {
        return Err(AppError::ProviderFailed(
          "codex-cli: model/list returned an empty catalog (run `codex login` if needed)".into(),
        ));
      }
      out.extend(page);
      match next_cursor(&result) {
        Some(next) => cursor = Some(next),
        None => break,
      }
    }
    if out.is_empty() {
      return Err(AppError::ProviderFailed(
        "codex-cli: no models after pagination".into(),
      ));
    }
    Ok(out)
  }

  pub async fn model(&self, id: &str) -> AppResult<Option<CodexCliModel>> {
    let list = self.models().await?;
    Ok(find_provider_model(&list, id).cloned())
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn parse_data_and_slug_fallback() {
    let result = json!({
      "data": [
        {"id":"gpt-5.1-codex","displayName":"GPT-5.1 Codex"},
        {"slug":"gpt-5.1-codex-mini"}
      ]
    });
    let list = parse_model_list(&result);
    assert_eq!(list.len(), 2);
    assert_eq!(list[0].model_id(), "gpt-5.1-codex");
    assert_eq!(list[0].display_name, "GPT-5.1 Codex");
    assert_eq!(list[1].model_id(), "gpt-5.1-codex-mini");
  }
}
