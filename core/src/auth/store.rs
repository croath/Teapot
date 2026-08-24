//! Local auth store: **one JSON file per provider**, each holding that provider's
//! original account structs (no shared flattened credential schema).
//!
//! Layout:
//! ```text
//! {data_local}/auth/
//!   codex.json
//!   codex-cli.json   # unused: this provider has AuthMethod::None
//!   claude.json
//!   claude-cli.json  # unused: this provider has AuthMethod::None
//!   grok-cli.json    # unused: this provider has AuthMethod::None
//!   xai.json         # unused when hidden from CLI/UI
//!   antigravity.json
//!   vertex.json
//! ```
//!
//! Each file is an account map whose values are the provider's native
//! [`Serialize`]/[`Deserialize`] type, for example:
//!
//! ```json
//! {
//!   "user@example.com": {
//!     "auth_kind": "oauth",
//!     "access_token": "...",
//!     "refresh_token": "..."
//!   }
//! }
//! ```
//!
//! Common API (type parameter = provider-native struct):
//! - [`AuthStore::save_account`]
//! - [`AuthStore::load_account`]
//! - [`AuthStore::load_all`]
//! - [`AuthStore::remove`]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};
use tracing::{info, warn};

use crate::error::{AppError, AppResult};
use crate::providers::ProviderKind;

pub use crate::paths::default_auth_dir;

/// Path of the credentials file for one provider under the default auth dir.
pub fn default_auth_path() -> AppResult<PathBuf> {
  default_auth_dir()
}

/// Sanitize a free-form account label for use as a JSON object key.
pub fn sanitize_account_segment(value: &str) -> String {
  let mut out = String::with_capacity(value.len());
  for c in value.trim().chars() {
    match c {
      'a'..='z' | 'A'..='Z' | '0'..='9' | '@' | '.' | '_' | '-' => out.push(c),
      _ => out.push('-'),
    }
  }
  let trimmed = out.trim_matches('-').to_string();
  if trimmed.is_empty() {
    "default".into()
  } else {
    trimmed
  }
}

/// Per-provider JSON credential store.
///
/// Holds a base directory; each [`ProviderKind`] maps to `{dir}/{provider}.json`.
/// Records are deserialized into **caller-chosen** native structs via the generic
/// load helpers — the store never flattens provider fields.
#[derive(Debug, Clone)]
pub struct AuthStore {
  /// Directory that contains `{provider}.json` files.
  dir: PathBuf,
}

impl AuthStore {
  /// `path` may be:
  /// - an auth **directory** (`…/auth/`) — preferred
  /// - a legacy single `auth.json` file — parent directory is used, with migration
  /// - any other path — treated as the auth directory
  pub fn new(path: impl Into<PathBuf>) -> Self {
    let path = path.into();
    let dir = if path
      .file_name()
      .and_then(|n| n.to_str())
      .is_some_and(|n| n == "auth.json")
    {
      path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
        .join("auth")
    } else if path.extension().and_then(|e| e.to_str()) == Some("json") {
      // e.g. `…/auth/codex.json` → use parent as store dir
      path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
    } else {
      path
    };
    Self { dir }
  }

  /// Open the default auth directory under local app data.
  pub fn local() -> AppResult<Self> {
    let _ = crate::paths::ensure_legacy_migrated();
    let store = Self::new(default_auth_dir()?);
    store.ensure_dir()?;
    Ok(store)
  }

  /// Base auth directory (contains per-provider JSON files).
  pub fn path(&self) -> &Path {
    &self.dir
  }

  /// Alias for [`Self::path`] (CLI / display).
  pub fn dir(&self) -> PathBuf {
    self.dir.clone()
  }

  /// Absolute path of this provider's credentials file: `{dir}/{provider}.json`.
  pub fn provider_path(&self, provider: ProviderKind) -> PathBuf {
    self.dir.join(format!("{}.json", provider.as_str()))
  }

  pub fn ensure_dir(&self) -> AppResult<()> {
    fs::create_dir_all(&self.dir)?;
    set_private_dir(&self.dir);
    Ok(())
  }

  /// Back-compat: ensure the store directory exists.
  pub fn ensure_parent(&self) -> AppResult<()> {
    self.ensure_dir()
  }

  // -----------------------------------------------------------------------
  // Common typed read / write API
  // -----------------------------------------------------------------------

  /// Save one account under `{provider}.json` as its **original** struct `T`.
  ///
  /// Returns the path of that provider's JSON file.
  pub fn save_account<T: Serialize>(
    &self,
    provider: ProviderKind,
    account: &str,
    value: &T,
  ) -> AppResult<PathBuf> {
    let account = sanitize_account_segment(account);
    let path = self.provider_path(provider);
    let mut section = self.load_provider_map(provider)?;
    let entry = serde_json::to_value(value).map_err(|e| {
      AppError::Internal(format!(
        "serialize `{}` auth for account `{account}`: {e}",
        provider.as_str()
      ))
    })?;
    section.insert(account.clone(), entry);
    self.write_provider_map(provider, &section)?;
    info!(
      path = %path.display(),
      provider = %provider.as_str(),
      account = %account,
      "saved provider auth"
    );
    Ok(path)
  }

  /// Load one account and deserialize into the provider-native type `T`.
  pub fn load_account<T: DeserializeOwned>(
    &self,
    provider: ProviderKind,
    account: &str,
  ) -> AppResult<T> {
    let account = sanitize_account_segment(account);
    let section = self.load_provider_map(provider)?;
    let value = section.get(&account).ok_or_else(|| {
      AppError::NotFound(format!(
        "no auth for `{}` account `{account}` (file {})",
        provider.as_str(),
        self.provider_path(provider).display()
      ))
    })?;
    serde_json::from_value(value.clone()).map_err(|e| {
      AppError::Internal(format!(
        "parse `{}`/`{account}` auth as {}: {e}",
        provider.as_str(),
        std::any::type_name::<T>()
      ))
    })
  }

  /// Load every account for a provider as `(account_key, T)`.
  ///
  /// `T` is the provider's original stored struct (e.g. Codex `StoredAuth`).
  /// Unreadable accounts are skipped with a debug log.
  pub fn load_all<T: DeserializeOwned>(
    &self,
    provider: ProviderKind,
  ) -> AppResult<Vec<(String, T)>> {
    let section = self.load_provider_map(provider)?;
    let mut out = Vec::new();
    let keys: BTreeMap<_, _> = section.iter().collect();
    for (account, value) in keys {
      match serde_json::from_value::<T>(value.clone()) {
        Ok(v) => out.push((account.clone(), v)),
        Err(e) => {
          tracing::debug!(
            provider = %provider.as_str(),
            account = %account,
            error = %e,
            type_name = %std::any::type_name::<T>(),
            "skip unreadable provider auth account"
          );
        }
      }
    }
    Ok(out)
  }

  /// Remove one account, or the entire provider file when `account` is `None`.
  pub fn remove(&self, provider: ProviderKind, account: Option<&str>) -> AppResult<usize> {
    let path = self.provider_path(provider);
    if let Some(acc) = account {
      let acc = sanitize_account_segment(acc);
      let mut section = self.load_provider_map(provider)?;
      let removed = if section.remove(&acc).is_some() { 1 } else { 0 };
      if removed > 0 {
        if section.is_empty() {
          let _ = fs::remove_file(&path);
        } else {
          self.write_provider_map(provider, &section)?;
        }
        info!(
          path = %path.display(),
          provider = %provider.as_str(),
          account = %acc,
          "removed provider auth account"
        );
      }
      return Ok(removed);
    }

    // Remove whole provider file.
    if path.is_file() {
      let section = self.load_provider_map(provider)?;
      let n = section.len();
      let _ = fs::remove_file(&path);
      if n > 0 {
        info!(
          path = %path.display(),
          provider = %provider.as_str(),
          removed = n,
          "removed provider auth file"
        );
      }
      return Ok(n);
    }

    // Legacy single-file region (if still present).
    if let Some(n) = self.remove_legacy_region(provider)? {
      return Ok(n);
    }
    Ok(0)
  }

  // -----------------------------------------------------------------------
  // Internal: per-provider file I/O
  // -----------------------------------------------------------------------

  fn load_provider_map(&self, provider: ProviderKind) -> AppResult<Map<String, Value>> {
    let path = self.provider_path(provider);
    if path.is_file() {
      return read_account_map(&path);
    }

    // Migrate / fall back to legacy `{parent}/auth.json` top-level region.
    if let Some(map) = self.load_legacy_region(provider)? {
      // Best-effort one-shot migration into the per-provider file.
      if !map.is_empty() {
        if let Err(e) = self.write_provider_map(provider, &map) {
          warn!(
            provider = %provider.as_str(),
            error = %e,
            "failed to migrate legacy auth.json region; using in-memory data"
          );
        } else {
          info!(
            path = %path.display(),
            provider = %provider.as_str(),
            accounts = map.len(),
            "migrated auth from legacy auth.json"
          );
          // Drop migrated region from legacy file when possible.
          let _ = self.remove_legacy_region(provider);
        }
      }
      return Ok(map);
    }

    Ok(Map::new())
  }

  fn write_provider_map(
    &self,
    provider: ProviderKind,
    section: &Map<String, Value>,
  ) -> AppResult<()> {
    self.ensure_dir()?;
    let path = self.provider_path(provider);
    let text = serde_json::to_string_pretty(&Value::Object(section.clone()))
      .map_err(|e| AppError::Internal(format!("serialize auth: {e}")))?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, text.as_bytes())?;
    set_private_file(&tmp);
    fs::rename(&tmp, &path)?;
    set_private_file(&path);
    Ok(())
  }

  /// Legacy layout: sibling `auth.json` (or parent `auth.json`) with
  /// `{ "codex": { ... }, "claude": { ... } }`.
  fn legacy_auth_json_path(&self) -> PathBuf {
    // Prefer `{dir}/../auth.json` when dir ends with `auth/`, else `{dir}/auth.json`.
    if self
      .dir
      .file_name()
      .and_then(|n| n.to_str())
      .is_some_and(|n| n == "auth")
    {
      self
        .dir
        .parent()
        .map(|p| p.join("auth.json"))
        .unwrap_or_else(|| self.dir.join("auth.json"))
    } else {
      self.dir.join("auth.json")
    }
  }

  fn load_legacy_region(&self, provider: ProviderKind) -> AppResult<Option<Map<String, Value>>> {
    let legacy = self.legacy_auth_json_path();
    if !legacy.is_file() {
      return Ok(None);
    }
    let root = read_root_object(&legacy)?;
    let key = provider.as_str();
    Ok(root.get(key).and_then(|v| v.as_object()).cloned())
  }

  fn remove_legacy_region(&self, provider: ProviderKind) -> AppResult<Option<usize>> {
    let legacy = self.legacy_auth_json_path();
    if !legacy.is_file() {
      return Ok(None);
    }
    let mut root = read_root_object(&legacy)?;
    let key = provider.as_str();
    let Some(section) = root.get(key).and_then(|v| v.as_object()) else {
      return Ok(None);
    };
    let n = section.len();
    root.remove(key);
    if root.is_empty() {
      let _ = fs::remove_file(&legacy);
    } else {
      write_root_object(&legacy, &root)?;
    }
    Ok(Some(n))
  }
}

fn read_account_map(path: &Path) -> AppResult<Map<String, Value>> {
  let text = fs::read_to_string(path)?;
  if text.trim().is_empty() {
    return Ok(Map::new());
  }
  let value: Value = serde_json::from_str(&text)
    .map_err(|e| AppError::Internal(format!("parse auth {}: {e}", path.display())))?;
  match value {
    Value::Object(map) => Ok(map),
    _ => Err(AppError::Internal(format!(
      "auth file {} root must be a JSON object (account map)",
      path.display()
    ))),
  }
}

fn read_root_object(path: &Path) -> AppResult<Map<String, Value>> {
  read_account_map(path)
}

fn write_root_object(path: &Path, root: &Map<String, Value>) -> AppResult<()> {
  if let Some(parent) = path.parent() {
    fs::create_dir_all(parent)?;
    set_private_dir(parent);
  }
  let text = serde_json::to_string_pretty(&Value::Object(root.clone()))
    .map_err(|e| AppError::Internal(format!("serialize auth: {e}")))?;
  let tmp = path.with_extension("json.tmp");
  fs::write(&tmp, text.as_bytes())?;
  set_private_file(&tmp);
  fs::rename(&tmp, path)?;
  set_private_file(path);
  Ok(())
}

fn set_private_dir(path: &Path) {
  #[cfg(unix)]
  {
    use std::os::unix::fs::PermissionsExt;
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o700));
  }
  let _ = path;
}

fn set_private_file(path: &Path) {
  #[cfg(unix)]
  {
    use std::os::unix::fs::PermissionsExt;
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
  }
  let _ = path;
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde::{Deserialize, Serialize};

  #[derive(Debug, Serialize, Deserialize, PartialEq)]
  struct CodexSample {
    access_token: String,
    account_id: Option<String>,
  }

  #[derive(Debug, Serialize, Deserialize, PartialEq)]
  struct ClaudeSample {
    access_token: String,
    scopes: Vec<String>,
  }

  #[test]
  fn separate_files_and_typed_load() {
    let dir = std::env::temp_dir().join(format!("teapot-auth-files-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let store = AuthStore::new(&dir);

    store
      .save_account(
        ProviderKind::Codex,
        "user@example.com",
        &CodexSample {
          access_token: "codex-tok".into(),
          account_id: Some("acc-1".into()),
        },
      )
      .unwrap();
    store
      .save_account(
        ProviderKind::Claude,
        "other@example.com",
        &ClaudeSample {
          access_token: "claude-tok".into(),
          scopes: vec!["org:create_api_key".into()],
        },
      )
      .unwrap();

    assert!(store.provider_path(ProviderKind::Codex).is_file());
    assert!(store.provider_path(ProviderKind::Claude).is_file());
    assert_ne!(
      store.provider_path(ProviderKind::Codex),
      store.provider_path(ProviderKind::Claude)
    );

    let codex_text = fs::read_to_string(store.provider_path(ProviderKind::Codex)).unwrap();
    assert!(codex_text.contains("codex-tok"));
    assert!(!codex_text.contains("claude-tok"));
    assert!(!codex_text.contains("\"codex\"")); // no wrapper key; file *is* the account map

    let codex: Vec<(String, CodexSample)> = store.load_all(ProviderKind::Codex).unwrap();
    assert_eq!(codex.len(), 1);
    assert_eq!(codex[0].0, "user@example.com");
    assert_eq!(codex[0].1.access_token, "codex-tok");

    let claude: ClaudeSample = store
      .load_account(ProviderKind::Claude, "other@example.com")
      .unwrap();
    assert_eq!(claude.access_token, "claude-tok");
    assert_eq!(claude.scopes.len(), 1);

    assert_eq!(
      store
        .remove(ProviderKind::Codex, Some("user@example.com"))
        .unwrap(),
      1
    );
    assert!(!store.provider_path(ProviderKind::Codex).exists());
    assert_eq!(store.remove(ProviderKind::Claude, None).unwrap(), 1);
    assert!(!store.provider_path(ProviderKind::Claude).exists());

    let _ = fs::remove_dir_all(&dir);
  }

  #[test]
  fn migrates_legacy_single_auth_json() {
    let base = std::env::temp_dir().join(format!("teapot-auth-legacy-{}", std::process::id()));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).unwrap();

    // Old layout: {base}/auth.json with top-level provider keys.
    let legacy = base.join("auth.json");
    fs::write(
      &legacy,
      r#"{
        "codex": {
          "a@b.com": { "access_token": "legacy", "account_id": null }
        }
      }"#,
    )
    .unwrap();

    let store = AuthStore::new(base.join("auth"));
    let loaded: Vec<(String, CodexSample)> = store.load_all(ProviderKind::Codex).unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].1.access_token, "legacy");
    assert!(store.provider_path(ProviderKind::Codex).is_file());

    let _ = fs::remove_dir_all(&base);
  }

  #[test]
  fn sanitize_segments() {
    assert_eq!(sanitize_account_segment("a@b.com"), "a@b.com");
    assert_eq!(sanitize_account_segment("a/b c"), "a-b-c");
    assert_eq!(sanitize_account_segment("!!!"), "default");
  }
}
