//! Grok Build CLI model catalog via `grok models`.

use serde::{Deserialize, Serialize};
use tokio::process::Command;

use crate::error::{AppError, AppResult};
use crate::providers::model_info::{ModelInfo, ProviderModel, find_provider_model};
use crate::providers::traits::{augmented_path, resolve_binary};

use super::GrokCliProvider;

/// One selectable Grok Build CLI model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GrokCliModel {
  pub id: String,
  #[serde(default)]
  pub display_name: String,
  #[serde(default)]
  pub is_default: bool,
}

impl ProviderModel for GrokCliModel {
  fn model_id(&self) -> &str {
    self.id.as_str()
  }

  fn to_model_info(&self) -> ModelInfo {
    let display_name = if self.display_name.is_empty() {
      self.id.clone()
    } else {
      self.display_name.clone()
    };
    ModelInfo::new(self.id.clone(), "xai", 0, display_name)
  }
}

/// Parse `grok models` text output into native rows.
pub fn parse_models_output(text: &str) -> Vec<GrokCliModel> {
  let mut out = Vec::new();
  for line in text.lines() {
    let trimmed = line.trim();
    let (marker, rest) = if let Some(rest) = trimmed.strip_prefix("* ") {
      ("*", rest)
    } else if let Some(rest) = trimmed.strip_prefix("- ") {
      ("-", rest)
    } else if let Some(rest) = trimmed.strip_prefix("• ") {
      ("•", rest)
    } else {
      continue;
    };
    let id = rest
      .split_whitespace()
      .next()
      .unwrap_or("")
      .trim()
      .trim_end_matches(':');
    if id.is_empty() {
      continue;
    }
    let is_default = marker == "*" || rest.contains("(default)");
    let display_name = rest.trim().trim_end_matches("(default)").trim().to_string();
    if out.iter().any(|m: &GrokCliModel| m.id == id) {
      continue;
    }
    out.push(GrokCliModel {
      id: id.to_string(),
      display_name,
      is_default,
    });
  }
  out
}

impl GrokCliProvider {
  pub async fn models(&self) -> AppResult<Vec<GrokCliModel>> {
    let program = resolve_binary("grok").ok_or_else(|| {
      AppError::ProviderBinaryMissing(
        "grok: install Grok Build CLI and ensure `grok` is on PATH".into(),
      )
    })?;
    let output = Command::new(&program)
      .arg("models")
      .env("PATH", augmented_path())
      .env("GROK_DISABLE_AUTOUPDATER", "1")
      .output()
      .await
      .map_err(|e| AppError::ProviderFailed(format!("grok-cli: spawn `{program} models`: {e}")))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() {
      let detail = stderr.trim();
      return Err(AppError::ProviderFailed(if detail.is_empty() {
        "grok-cli: `grok models` failed (run `grok login` if needed)".into()
      } else {
        format!("grok-cli: `grok models` failed: {detail}")
      }));
    }
    let list = parse_models_output(&stdout);
    if list.is_empty() {
      return Err(AppError::ProviderFailed(
        "grok-cli: `grok models` returned an empty catalog (run `grok login` if needed)".into(),
      ));
    }
    Ok(list)
  }

  pub async fn model(&self, id: &str) -> AppResult<Option<GrokCliModel>> {
    let list = self.models().await?;
    Ok(find_provider_model(&list, id).cloned())
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn parse_star_and_dash_rows() {
    let text = "\
You are logged in with grok.com.

Default model: grok-4.6

Available models:
  * grok-4.6 (default)
  - grok-4.5
  - grok-build
";
    let list = parse_models_output(text);
    assert_eq!(list.len(), 3);
    assert_eq!(list[0].id, "grok-4.6");
    assert!(list[0].is_default);
    assert_eq!(list[1].id, "grok-4.5");
    assert!(!list[1].is_default);
    assert_eq!(list[2].id, "grok-build");
    assert_eq!(list[0].to_model_info().owned_by, "xai");
  }
}
