//! Check GitHub Releases and install signed updater artifacts.

use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_updater::{Update, UpdaterExt};

pub struct PendingUpdate(Mutex<Option<Update>>);

impl PendingUpdate {
  pub fn new() -> Self {
    Self(Mutex::new(None))
  }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheck {
  pub available: bool,
  pub current_version: String,
  pub version: Option<String>,
  pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadProgress {
  pub downloaded: u64,
  pub content_length: Option<u64>,
}

#[tauri::command]
pub async fn check_for_update(
  app: AppHandle,
  pending: State<'_, PendingUpdate>,
) -> Result<UpdateCheck, String> {
  let updater = app.updater().map_err(|e| e.to_string())?;
  let found = updater.check().await.map_err(|e| e.to_string())?;

  match found {
    Some(update) => {
      let info = UpdateCheck {
        available: true,
        current_version: update.current_version.clone(),
        version: Some(update.version.clone()),
        notes: update.body.clone(),
      };
      *pending.0.lock().map_err(|e| e.to_string())? = Some(update);
      tracing::info!(
        current = %info.current_version,
        next = ?info.version,
        "update available"
      );
      Ok(info)
    }
    None => {
      *pending.0.lock().map_err(|e| e.to_string())? = None;
      let current_version = app.package_info().version.to_string();
      tracing::info!(version = %current_version, "app is up to date");
      Ok(UpdateCheck {
        available: false,
        current_version,
        version: None,
        notes: None,
      })
    }
  }
}

#[tauri::command]
pub async fn install_update(
  app: AppHandle,
  pending: State<'_, PendingUpdate>,
) -> Result<(), String> {
  let update = pending
    .0
    .lock()
    .map_err(|e| e.to_string())?
    .take()
    .ok_or_else(|| "No pending update. Check for updates first.".to_string())?;

  tracing::info!(version = %update.version, "downloading update");
  let handle = app.clone();
  let mut downloaded = 0u64;
  update
    .download_and_install(
      |chunk_length, content_length| {
        downloaded += chunk_length as u64;
        let _ = handle.emit(
          "updater-progress",
          DownloadProgress {
            downloaded,
            content_length,
          },
        );
      },
      || {
        let _ = handle.emit("updater-finished", ());
      },
    )
    .await
    .map_err(|e| e.to_string())?;

  tracing::info!("update installed, restarting");
  app.restart();
}
