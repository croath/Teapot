//! Per-provider models cache on disk.
//!
//! Layout:
//! ```text
//! {data_local}/models/
//!   codex.json
//!   claude.json
//!   claude-cli.json
//!   grok-cli.json
//!   xai.json
//!   antigravity.json
//!   vertex.json
//! ```
//!
//! Each file holds **only that provider's native model list** (no shared flattened
//! schema, no cross-provider keys). A process pinned to one provider only
//! reads/writes its own file.
//!
//! File shape:
//! ```json
//! {
//!   "updated_at": "2026-01-01T00:00:00Z",
//!   "models": [ /* provider-native structs */ ]
//! }
//! ```
//!
//! Common typed API:
//! - [`ModelsStore::save_models`]
//! - [`ModelsStore::load_models`]

use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tracing::info;

use crate::error::{AppError, AppResult};
use crate::providers::ProviderKind;

pub use crate::paths::default_models_dir;

/// Per-provider models cache store.
#[derive(Debug, Clone)]
pub struct ModelsStore {
  dir: PathBuf,
}

impl ModelsStore {
  /// `path` is the models directory (or a `*.json` file → parent is used).
  pub fn new(path: impl Into<PathBuf>) -> Self {
    let path = path.into();
    let dir = if path.extension().and_then(|e| e.to_str()) == Some("json") {
      path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
    } else {
      path
    };
    Self { dir }
  }

  /// Open the default models directory under local app data.
  pub fn local() -> AppResult<Self> {
    let _ = crate::paths::ensure_legacy_migrated();
    let store = Self::new(default_models_dir()?);
    store.ensure_dir()?;
    Ok(store)
  }

  pub fn dir(&self) -> &Path {
    &self.dir
  }

  /// Absolute path of this provider's models file: `{dir}/{provider}.json`.
  pub fn provider_path(&self, provider: ProviderKind) -> PathBuf {
    self.dir.join(format!("{}.json", provider.as_str()))
  }

  pub fn ensure_dir(&self) -> AppResult<()> {
    fs::create_dir_all(&self.dir)?;
    Ok(())
  }

  /// Replace this provider's entire model list with native structs `T`.
  ///
  /// Only touches `{provider}.json` — other providers' files are left alone.
  pub fn save_models<T: Serialize>(
    &self,
    provider: ProviderKind,
    models: &[T],
  ) -> AppResult<PathBuf> {
    self.ensure_dir()?;
    let path = self.provider_path(provider);
    let value = serde_json::json!({
      "updated_at": Utc::now().to_rfc3339(),
      "models": models,
    });
    let text = serde_json::to_string_pretty(&value)
      .map_err(|e| AppError::Internal(format!("serialize `{}` models: {e}", provider.as_str())))?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, text.as_bytes())?;
    fs::rename(&tmp, &path)?;
    info!(
      path = %path.display(),
      provider = %provider.as_str(),
      count = models.len(),
      "saved provider models"
    );
    Ok(path)
  }

  /// Load this provider's native model list as `Vec<T>`.
  ///
  /// Returns `Ok(None)` when the file is missing or empty. Only reads
  /// `{provider}.json` for the given kind.
  pub fn load_models<T: DeserializeOwned>(
    &self,
    provider: ProviderKind,
  ) -> AppResult<Option<Vec<T>>> {
    let path = self.provider_path(provider);
    if !path.is_file() {
      return Ok(None);
    }
    let text = fs::read_to_string(&path)?;
    if text.trim().is_empty() {
      return Ok(None);
    }
    let value: Value = serde_json::from_str(&text)
      .map_err(|e| AppError::Internal(format!("parse models cache {}: {e}", path.display())))?;

    let models_value = match &value {
      // Preferred: { "updated_at": "...", "models": [ ... ] }
      Value::Object(map) => map.get("models").cloned().unwrap_or(Value::Null),
      // Plain array of native models.
      Value::Array(_) => value.clone(),
      other => {
        return Err(AppError::Internal(format!(
          "models cache {} root must be object or array, got {}",
          path.display(),
          match other {
            Value::Null => "null",
            Value::Bool(_) => "bool",
            Value::Number(_) => "number",
            Value::String(_) => "string",
            _ => "unknown",
          }
        )));
      }
    };

    if models_value.is_null() {
      return Ok(None);
    }

    let models: Vec<T> = serde_json::from_value(models_value).map_err(|e| {
      AppError::Internal(format!(
        "parse `{}` models as {}: {e}",
        provider.as_str(),
        std::any::type_name::<T>()
      ))
    })?;

    if models.is_empty() {
      return Ok(None);
    }
    Ok(Some(models))
  }

  /// Remove this provider's models file (does not touch other providers).
  pub fn clear(&self, provider: ProviderKind) -> AppResult<bool> {
    let path = self.provider_path(provider);
    if path.is_file() {
      fs::remove_file(&path)?;
      info!(
        path = %path.display(),
        provider = %provider.as_str(),
        "removed provider models file"
      );
      Ok(true)
    } else {
      Ok(false)
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde::{Deserialize, Serialize};

  #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
  struct CodexLike {
    slug: String,
    display_name: String,
  }

  #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
  struct ClaudeLike {
    id: String,
    display_name: String,
  }

  #[test]
  fn each_provider_has_own_file_and_native_type() {
    let dir = std::env::temp_dir().join(format!("teapot-models-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let store = ModelsStore::new(&dir);

    store
      .save_models(
        ProviderKind::Codex,
        &[CodexLike {
          slug: "gpt-5".into(),
          display_name: "GPT-5".into(),
        }],
      )
      .unwrap();
    store
      .save_models(
        ProviderKind::Claude,
        &[ClaudeLike {
          id: "claude-sonnet".into(),
          display_name: "Sonnet".into(),
        }],
      )
      .unwrap();

    assert!(store.provider_path(ProviderKind::Codex).is_file());
    assert!(store.provider_path(ProviderKind::Claude).is_file());
    assert_ne!(
      store.provider_path(ProviderKind::Codex),
      store.provider_path(ProviderKind::Claude)
    );

    let codex_text = fs::read_to_string(store.provider_path(ProviderKind::Codex)).unwrap();
    assert!(codex_text.contains("gpt-5"));
    assert!(!codex_text.contains("claude-sonnet"));
    // No cross-provider tag / other provider keys.
    assert!(!codex_text.contains("\"provider\""));
    assert!(!codex_text.contains("claude"));

    let codex: Vec<CodexLike> = store.load_models(ProviderKind::Codex).unwrap().unwrap();
    assert_eq!(codex.len(), 1);
    assert_eq!(codex[0].slug, "gpt-5");

    let claude: Vec<ClaudeLike> = store.load_models(ProviderKind::Claude).unwrap().unwrap();
    assert_eq!(claude[0].id, "claude-sonnet");

    // Updating codex does not clear claude.
    store
      .save_models(
        ProviderKind::Codex,
        &[
          CodexLike {
            slug: "gpt-5".into(),
            display_name: "GPT-5".into(),
          },
          CodexLike {
            slug: "o3".into(),
            display_name: "o3".into(),
          },
        ],
      )
      .unwrap();
    let codex2: Vec<CodexLike> = store.load_models(ProviderKind::Codex).unwrap().unwrap();
    assert_eq!(codex2.len(), 2);
    let claude2: Vec<ClaudeLike> = store.load_models(ProviderKind::Claude).unwrap().unwrap();
    assert_eq!(claude2.len(), 1);

    store.clear(ProviderKind::Codex).unwrap();
    assert!(!store.provider_path(ProviderKind::Codex).exists());
    assert!(store.provider_path(ProviderKind::Claude).is_file());

    let _ = fs::remove_dir_all(&dir);
  }

  #[test]
  fn missing_file_returns_none() {
    let dir = std::env::temp_dir().join(format!("teapot-models-miss-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let store = ModelsStore::new(&dir);
    let none: Option<Vec<CodexLike>> = store.load_models(ProviderKind::Xai).unwrap();
    assert!(none.is_none());
    let _ = fs::remove_dir_all(&dir);
  }
}
