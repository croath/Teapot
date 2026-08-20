//! Desktop storage roots from `AppHandle.path()`.
//!
//! Config, auth, and models live under the app-scoped local data directory
//! (`app_local_data_dir` = `{local_data_dir}/{bundleIdentifier}`). Same path
//! as `teapot_core::paths::data_local_dir()` used by the CLI.

use std::fs;
use std::path::PathBuf;

use tauri::{AppHandle, Manager};
use teapot_core::paths::{DATA_DIR_ENV, copy_file_if_missing, migrate_legacy_project_data};

const CONFIG_FILE: &str = "config.toml";

/// App-scoped local data directory (`AppHandle.path().app_local_data_dir()`).
pub fn data_local_dir(app: &AppHandle) -> Result<PathBuf, String> {
  app
    .path()
    .app_local_data_dir()
    .map_err(|e| format!("resolve data local dir: {e}"))
}

pub fn config_path(app: &AppHandle) -> Result<PathBuf, String> {
  let dir = data_local_dir(app)?;
  fs::create_dir_all(&dir).map_err(|e| format!("create data local dir: {e}"))?;
  Ok(dir.join(CONFIG_FILE))
}

pub fn auth_dir(app: &AppHandle) -> Result<PathBuf, String> {
  Ok(data_local_dir(app)?.join("auth"))
}

pub fn models_dir(app: &AppHandle) -> Result<PathBuf, String> {
  Ok(data_local_dir(app)?.join("models"))
}

/// If the Tauri data-local tree has no config/auth/models yet, copy them from
/// the legacy `directories::ProjectDirs::from("dev", "teapot", "teapot")` tree
/// (and from the previous desktop `app_config_dir` for `config.toml`).
pub fn ensure_migrated(app: &AppHandle) {
  let dest = match data_local_dir(app) {
    Ok(dir) => dir,
    Err(error) => {
      tracing::warn!(%error, "skip data migrate: resolve dest failed");
      return;
    }
  };

  if let Err(error) = fs::create_dir_all(&dest) {
    tracing::warn!(
      %error,
      dest = %dest.display(),
      "skip data migrate: create dest failed"
    );
    return;
  }

  if let Ok(cfg_dir) = app.path().app_config_dir() {
    let src = cfg_dir.join(CONFIG_FILE);
    let dst = dest.join(CONFIG_FILE);
    if src != dst {
      if let Err(error) = copy_file_if_missing(&src, &dst) {
        tracing::warn!(%error, "migrate desktop config.toml failed");
      }
    }
  }

  match migrate_legacy_project_data(&dest) {
    Ok(report) if report.did_anything() => {
      tracing::info!(
        dest = %dest.display(),
        config = report.config,
        auth = report.auth,
        models = report.models,
        legacy_auth_json = report.legacy_auth_json,
        removed_legacy = report.removed_legacy,
        "migrated data from ProjectDirs into Tauri data local dir"
      );
    }
    Ok(_) => {}
    Err(error) => {
      tracing::warn!(
        %error,
        dest = %dest.display(),
        "legacy data migrate failed"
      );
    }
  }

  tracing::info!(
    dest = %dest.display(),
    env = DATA_DIR_ENV,
    "desktop data local dir"
  );
}
