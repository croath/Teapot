//! Claude Code model catalog.
//!
//! Claude Code has no `model/list` RPC. The catalog is the documented CLI
//! aliases plus well-known full ids that `--model` accepts. Requests still
//! go to the local `claude` binary; an unknown id fails at execute time.

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::providers::model_info::{ModelInfo, ProviderModel, find_provider_model};
use crate::providers::traits::resolve_binary;

use super::ClaudeCliProvider;

/// One selectable Claude Code model (alias or full id).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudeCliModel {
  pub id: String,
  #[serde(default)]
  pub display_name: String,
}

impl ProviderModel for ClaudeCliModel {
  fn model_id(&self) -> &str {
    self.id.as_str()
  }

  fn to_model_info(&self) -> ModelInfo {
    let display_name = if self.display_name.is_empty() {
      self.id.clone()
    } else {
      self.display_name.clone()
    };
    ModelInfo::new(self.id.clone(), "anthropic", 0, display_name)
  }
}

/// Documented Claude Code `--model` aliases and current full ids.
pub fn builtin_models() -> Vec<ClaudeCliModel> {
  [
    ("sonnet", "Claude Sonnet"),
    ("opus", "Claude Opus"),
    ("haiku", "Claude Haiku"),
    ("fable", "Claude Fable"),
    ("best", "Claude Best"),
    ("default", "Claude Default"),
    ("sonnet[1m]", "Claude Sonnet (1M)"),
    ("opus[1m]", "Claude Opus (1M)"),
    ("claude-sonnet-5", "Claude Sonnet 5"),
    ("claude-opus-5", "Claude Opus 5"),
    ("claude-fable-5", "Claude Fable 5"),
    ("claude-haiku-4-5", "Claude Haiku 4.5"),
    ("claude-sonnet-4-6", "Claude Sonnet 4.6"),
    ("claude-sonnet-4-5", "Claude Sonnet 4.5"),
    ("claude-opus-4-8", "Claude Opus 4.8"),
    ("claude-opus-4-6", "Claude Opus 4.6"),
  ]
  .into_iter()
  .map(|(id, display_name)| ClaudeCliModel {
    id: id.into(),
    display_name: display_name.into(),
  })
  .collect()
}

impl ClaudeCliProvider {
  pub async fn models(&self) -> AppResult<Vec<ClaudeCliModel>> {
    if resolve_binary("claude").is_none() {
      return Err(AppError::ProviderBinaryMissing(
        "claude: install Claude Code and ensure `claude` is on PATH".into(),
      ));
    }
    Ok(builtin_models())
  }

  pub async fn model(&self, id: &str) -> AppResult<Option<ClaudeCliModel>> {
    let list = self.models().await?;
    Ok(find_provider_model(&list, id).cloned())
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn catalog_includes_aliases() {
    let list = builtin_models();
    assert!(list.iter().any(|m| m.id == "sonnet"));
    assert!(list.iter().any(|m| m.id == "opus"));
    assert!(list.iter().any(|m| m.id == "haiku"));
    assert_eq!(list[0].to_model_info().owned_by, "anthropic");
  }
}
