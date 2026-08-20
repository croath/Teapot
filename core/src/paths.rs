//! Local data root for config, auth, and models.
//!
//! CLI and desktop share the same directory:
//! `{BaseDirs::data_local_dir()}/{APP_IDENTIFIER}`, which matches Tauri
//! `AppHandle.path().app_local_data_dir()` (`identifier` in `tauri.conf.json`).
//!
//! Override with `TEAPOT_DATA_DIR`. On first use, files are copied from the
//! legacy `directories::ProjectDirs::from("dev", "teapot", "teapot")` tree
//! when the new location does not already have them, then that old tree is
//! removed.
//!
//! Layout under the root:
//! ```text
//! config.toml
//! auth/{provider}.json
//! models/{provider}.json
//! ```

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{AppError, AppResult};

/// Env override for the data-local root (`config.toml`, `auth/`, `models/`).
pub const DATA_DIR_ENV: &str = "TEAPOT_DATA_DIR";
/// Env override for the auth directory (takes precedence over [`DATA_DIR_ENV`]).
pub const AUTH_DIR_ENV: &str = "TEAPOT_AUTH_DIR";
/// Env override for the models directory (takes precedence over [`DATA_DIR_ENV`]).
pub const MODELS_DIR_ENV: &str = "TEAPOT_MODELS_DIR";

/// Must match `identifier` in `app-tauri/tauri.conf.json`.
pub const APP_IDENTIFIER: &str = "com.cdxtheme.teapot";

const QUALIFIER: &str = "dev";
const ORGANIZATION: &str = "teapot";
const APPLICATION: &str = "teapot";

/// `directories` project used before the desktop app switched to Tauri's path API.
pub fn project_dirs() -> Option<directories::ProjectDirs> {
  directories::ProjectDirs::from(QUALIFIER, ORGANIZATION, APPLICATION)
}

fn env_path(key: &str) -> Option<PathBuf> {
  let value = std::env::var_os(key)?;
  if value.is_empty() {
    return None;
  }
  Some(PathBuf::from(value))
}

/// Data-local root: `TEAPOT_DATA_DIR`, else `{local_data_dir}/{APP_IDENTIFIER}`.
pub fn data_local_dir() -> AppResult<PathBuf> {
  if let Some(dir) = env_path(DATA_DIR_ENV) {
    return Ok(dir);
  }
  directories::BaseDirs::new()
    .map(|b| b.data_local_dir().join(APP_IDENTIFIER))
    .ok_or_else(|| AppError::Internal("could not resolve local app data directory".into()))
}

/// Resolve the shared data-local root and copy legacy ProjectDirs files into it
/// when config/auth/models are not already present.
pub fn ensure_legacy_migrated() -> AppResult<PathBuf> {
  let dest = data_local_dir()?;
  match migrate_legacy_project_data(&dest) {
    Ok(report) if report.did_anything() => {
      tracing::info!(
        dest = %dest.display(),
        config = report.config,
        auth = report.auth,
        models = report.models,
        legacy_auth_json = report.legacy_auth_json,
        removed_legacy = report.removed_legacy,
        "migrated data from ProjectDirs into data local dir"
      );
    }
    Ok(_) => {}
    Err(error) => {
      tracing::warn!(
        error = %error,
        dest = %dest.display(),
        "legacy data migrate failed"
      );
    }
  }
  Ok(dest)
}

/// Default auth directory: `{data_local}/auth/`.
pub fn default_auth_dir() -> AppResult<PathBuf> {
  if let Some(dir) = env_path(AUTH_DIR_ENV) {
    return Ok(dir);
  }
  Ok(data_local_dir()?.join("auth"))
}

/// Default models directory: `{data_local}/models/`.
pub fn default_models_dir() -> AppResult<PathBuf> {
  if let Some(dir) = env_path(MODELS_DIR_ENV) {
    return Ok(dir);
  }
  Ok(data_local_dir()?.join("models"))
}

/// Default config file: `{data_local}/config.toml`.
pub fn default_config_file() -> AppResult<PathBuf> {
  Ok(data_local_dir()?.join("config.toml"))
}

/// Result of a one-shot copy from a legacy tree into a new data-local root.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MigrateReport {
  pub config: bool,
  pub auth: bool,
  pub models: bool,
  pub legacy_auth_json: bool,
  pub removed_legacy: bool,
}

impl MigrateReport {
  pub fn did_anything(self) -> bool {
    self.config || self.auth || self.models || self.legacy_auth_json || self.removed_legacy
  }
}

/// Copy `config.toml`, `auth/`, and `models/` from the legacy ProjectDirs tree
/// into `dest` when those items are not already present, then delete the old
/// ProjectDirs directories.
pub fn migrate_legacy_project_data(dest: &Path) -> AppResult<MigrateReport> {
  let Some(proj) = project_dirs() else {
    return Ok(MigrateReport::default());
  };
  let mut report = migrate_from_dirs(proj.config_dir(), proj.data_local_dir(), dest)?;
  report.removed_legacy = match remove_legacy_dirs(
    &[proj.data_local_dir(), proj.config_dir(), proj.data_dir()],
    dest,
  ) {
    Ok(removed) => removed,
    Err(error) => {
      tracing::warn!(error = %error, "failed to remove legacy data directory");
      false
    }
  };
  Ok(report)
}

/// Copy known store files from `legacy_config_dir` / `legacy_data_dir` into `dest`.
///
/// Existing destination files and non-empty `auth/` / `models/` dirs are left alone.
pub fn migrate_from_dirs(
  legacy_config_dir: &Path,
  legacy_data_dir: &Path,
  dest: &Path,
) -> AppResult<MigrateReport> {
  if same_dir(legacy_data_dir, dest) && same_dir(legacy_config_dir, dest) {
    return Ok(MigrateReport::default());
  }

  fs::create_dir_all(dest)?;

  let mut report = MigrateReport::default();
  let dest_config = dest.join("config.toml");
  report.config = copy_file_if_missing(&legacy_config_dir.join("config.toml"), &dest_config)?;
  if !dest_config.exists() {
    report.config = copy_file_if_missing(&legacy_data_dir.join("config.toml"), &dest_config)?;
  }

  report.auth = copy_dir_if_missing(&legacy_data_dir.join("auth"), &dest.join("auth"))?;
  report.legacy_auth_json =
    copy_file_if_missing(&legacy_data_dir.join("auth.json"), &dest.join("auth.json"))?;
  report.models = copy_dir_if_missing(&legacy_data_dir.join("models"), &dest.join("models"))?;
  Ok(report)
}

/// Delete legacy ProjectDirs roots after a successful copy into `dest`.
///
/// Skips any path that is missing or the same as `dest`. Empty parent
/// directories left behind (e.g. `{LocalAppData}/teapot`) are removed too.
pub fn remove_legacy_dirs(legacy_dirs: &[&Path], dest: &Path) -> AppResult<bool> {
  let mut removed = false;
  let mut seen: Vec<PathBuf> = Vec::new();
  for path in legacy_dirs {
    if seen.iter().any(|p| same_dir(p, path)) {
      continue;
    }
    seen.push((*path).to_path_buf());
    if remove_dir_if_distinct(path, dest)? {
      removed = true;
    }
  }
  Ok(removed)
}

fn remove_dir_if_distinct(path: &Path, dest: &Path) -> AppResult<bool> {
  if !path.exists() || same_dir(path, dest) {
    return Ok(false);
  }
  fs::remove_dir_all(path)?;
  tracing::info!(path = %path.display(), "removed legacy data directory");
  if let Some(parent) = path.parent() {
    let _ = fs::remove_dir(parent);
  }
  Ok(true)
}

/// Copy `src` to `dest` when `dest` does not already exist.
pub fn copy_file_if_missing(src: &Path, dest: &Path) -> AppResult<bool> {
  if dest.exists() || !src.is_file() {
    return Ok(false);
  }
  if let Some(parent) = dest.parent() {
    fs::create_dir_all(parent)?;
  }
  fs::copy(src, dest)?;
  Ok(true)
}

fn copy_dir_if_missing(src: &Path, dest: &Path) -> AppResult<bool> {
  if !src.is_dir() || dir_has_data(dest) {
    return Ok(false);
  }
  copy_dir_recursive(src, dest)?;
  Ok(true)
}

fn dir_has_data(path: &Path) -> bool {
  if path.is_file() {
    return true;
  }
  let Ok(entries) = fs::read_dir(path) else {
    return false;
  };
  for entry in entries.flatten() {
    let child = entry.path();
    if child.is_file() || (child.is_dir() && dir_has_data(&child)) {
      return true;
    }
  }
  false
}

fn copy_dir_recursive(src: &Path, dest: &Path) -> std::io::Result<()> {
  fs::create_dir_all(dest)?;
  for entry in fs::read_dir(src)? {
    let entry = entry?;
    let from = entry.path();
    let to = dest.join(entry.file_name());
    if entry.file_type()?.is_dir() {
      copy_dir_recursive(&from, &to)?;
    } else if entry.file_type()?.is_file() && !to.exists() {
      if let Some(parent) = to.parent() {
        fs::create_dir_all(parent)?;
      }
      fs::copy(&from, &to)?;
    }
  }
  Ok(())
}

fn same_dir(a: &Path, b: &Path) -> bool {
  if a == b {
    return true;
  }
  match (fs::canonicalize(a), fs::canonicalize(b)) {
    (Ok(left), Ok(right)) => left == right,
    _ => false,
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn temp_tree(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
      "teapot-paths-{tag}-{}-{}",
      std::process::id(),
      std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
  }

  #[test]
  fn migrate_copies_missing_config_auth_models() {
    let src = temp_tree("src");
    let dest = temp_tree("dest");
    fs::write(src.join("config.toml"), "listen = \"127.0.0.1:9\"\n").unwrap();
    fs::create_dir_all(src.join("auth")).unwrap();
    fs::write(src.join("auth").join("codex.json"), "{\"a\":{}}").unwrap();
    fs::write(src.join("auth.json"), "{\"codex\":{}}").unwrap();
    fs::create_dir_all(src.join("models")).unwrap();
    fs::write(src.join("models").join("codex.json"), "{\"models\":[]}").unwrap();

    let report = migrate_from_dirs(&src, &src, &dest).unwrap();
    assert!(report.did_anything());
    assert!(report.config && report.auth && report.models && report.legacy_auth_json);
    assert_eq!(
      fs::read_to_string(dest.join("config.toml")).unwrap(),
      "listen = \"127.0.0.1:9\"\n"
    );
    assert!(dest.join("auth").join("codex.json").is_file());
    assert!(dest.join("models").join("codex.json").is_file());
    assert!(dest.join("auth.json").is_file());

    let _ = fs::remove_dir_all(&src);
    let _ = fs::remove_dir_all(&dest);
  }

  #[test]
  fn migrate_does_not_overwrite_existing_dest_data() {
    let src = temp_tree("src2");
    let dest = temp_tree("dest2");
    fs::write(src.join("config.toml"), "from = \"src\"\n").unwrap();
    fs::create_dir_all(src.join("auth")).unwrap();
    fs::write(src.join("auth").join("codex.json"), "src-auth").unwrap();
    fs::create_dir_all(src.join("models")).unwrap();
    fs::write(src.join("models").join("codex.json"), "src-models").unwrap();

    fs::write(dest.join("config.toml"), "from = \"dest\"\n").unwrap();
    fs::create_dir_all(dest.join("auth")).unwrap();
    fs::write(dest.join("auth").join("codex.json"), "dest-auth").unwrap();
    fs::create_dir_all(dest.join("models")).unwrap();
    fs::write(dest.join("models").join("codex.json"), "dest-models").unwrap();

    let report = migrate_from_dirs(&src, &src, &dest).unwrap();
    assert!(!report.did_anything());
    assert_eq!(
      fs::read_to_string(dest.join("config.toml")).unwrap(),
      "from = \"dest\"\n"
    );
    assert_eq!(
      fs::read_to_string(dest.join("auth").join("codex.json")).unwrap(),
      "dest-auth"
    );
    assert_eq!(
      fs::read_to_string(dest.join("models").join("codex.json")).unwrap(),
      "dest-models"
    );

    let _ = fs::remove_dir_all(&src);
    let _ = fs::remove_dir_all(&dest);
  }

  #[test]
  fn migrate_noop_when_legacy_missing() {
    let src = temp_tree("empty-src");
    let dest = temp_tree("empty-dest");
    let report = migrate_from_dirs(&src, &src, &dest).unwrap();
    assert!(!report.did_anything());
    let _ = fs::remove_dir_all(&src);
    let _ = fs::remove_dir_all(&dest);
  }

  #[test]
  fn migrate_skips_when_src_is_dest() {
    let dir = temp_tree("same");
    fs::write(dir.join("config.toml"), "x = 1\n").unwrap();
    let report = migrate_from_dirs(&dir, &dir, &dir).unwrap();
    assert!(!report.did_anything());
    let _ = fs::remove_dir_all(&dir);
  }

  #[test]
  fn removes_legacy_dir_after_copy() {
    let src = temp_tree("src-rm");
    let dest = temp_tree("dest-rm");
    fs::write(src.join("config.toml"), "listen = \"127.0.0.1:9\"\n").unwrap();
    fs::create_dir_all(src.join("auth")).unwrap();
    fs::write(src.join("auth").join("codex.json"), "{\"a\":{}}").unwrap();

    migrate_from_dirs(&src, &src, &dest).unwrap();
    assert!(remove_legacy_dirs(&[&src], &dest).unwrap());
    assert!(!src.exists());
    assert!(dest.join("auth").join("codex.json").is_file());
    assert_eq!(
      fs::read_to_string(dest.join("config.toml")).unwrap(),
      "listen = \"127.0.0.1:9\"\n"
    );

    let _ = fs::remove_dir_all(&dest);
  }

  #[test]
  fn removes_legacy_dir_when_dest_already_has_data() {
    let src = temp_tree("src-rm2");
    let dest = temp_tree("dest-rm2");
    fs::write(src.join("config.toml"), "from = \"src\"\n").unwrap();
    fs::write(dest.join("config.toml"), "from = \"dest\"\n").unwrap();

    migrate_from_dirs(&src, &src, &dest).unwrap();
    assert!(remove_legacy_dirs(&[&src], &dest).unwrap());
    assert!(!src.exists());
    assert_eq!(
      fs::read_to_string(dest.join("config.toml")).unwrap(),
      "from = \"dest\"\n"
    );

    let _ = fs::remove_dir_all(&dest);
  }

  #[test]
  fn remove_skips_when_legacy_is_dest() {
    let dir = temp_tree("same-rm");
    fs::write(dir.join("config.toml"), "x = 1\n").unwrap();
    assert!(!remove_legacy_dirs(&[&dir], &dir).unwrap());
    assert!(dir.join("config.toml").is_file());
    let _ = fs::remove_dir_all(&dir);
  }

  #[test]
  fn default_data_local_dir_uses_app_identifier() {
    if std::env::var_os(DATA_DIR_ENV).is_some() {
      return;
    }
    let dir = data_local_dir().unwrap();
    assert_eq!(
      dir.file_name().and_then(|n| n.to_str()),
      Some(APP_IDENTIFIER)
    );
  }
}
