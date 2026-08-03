//! Tauri desktop shell: start/stop the local API server and host the Leptos UI.
//! User preferences (selected agent, listen address) are stored under app local data.

use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};
use teaport_core::{list_agent_infos, serve_with_shutdown, AgentInfo, Config};
use tokio::sync::oneshot;
use tracing_subscriber::EnvFilter;

const PREFS_FILE: &str = "preferences.json";

struct RunningServer {
  listen: String,
  agent: Option<String>,
  shutdown: oneshot::Sender<()>,
}

struct ServerState {
  inner: Mutex<Option<RunningServer>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct UserPreferences {
  #[serde(default = "default_listen")]
  listen: String,
  /// Selected agent CLI name (optional until user chooses).
  #[serde(default)]
  agent: Option<String>,
}

fn default_listen() -> String {
  "127.0.0.1:8080".into()
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ServerStatus {
  running: bool,
  listen: Option<String>,
  base_url: Option<String>,
  agent: Option<String>,
}

fn base_url_for(listen: &str) -> String {
  if listen.starts_with("http://") || listen.starts_with("https://") {
    listen.to_string()
  } else {
    format!("http://{listen}")
  }
}

fn status_from(server: Option<&RunningServer>) -> ServerStatus {
  match server {
    Some(s) => ServerStatus {
      running: true,
      listen: Some(s.listen.clone()),
      base_url: Some(base_url_for(&s.listen)),
      agent: s.agent.clone(),
    },
    None => ServerStatus {
      running: false,
      listen: None,
      base_url: None,
      agent: None,
    },
  }
}

fn prefs_path(app: &AppHandle) -> Result<PathBuf, String> {
  let dir = app
    .path()
    .app_local_data_dir()
    .map_err(|e| format!("app local data dir: {e}"))?;
  fs::create_dir_all(&dir).map_err(|e| format!("create app local data dir: {e}"))?;
  Ok(dir.join(PREFS_FILE))
}

fn read_prefs(app: &AppHandle) -> Result<UserPreferences, String> {
  let path = prefs_path(app)?;
  if !path.exists() {
    return Ok(UserPreferences::default());
  }
  let text = fs::read_to_string(&path).map_err(|e| format!("read preferences: {e}"))?;
  serde_json::from_str(&text).map_err(|e| format!("parse preferences: {e}"))
}

fn write_prefs(app: &AppHandle, prefs: &UserPreferences) -> Result<(), String> {
  let path = prefs_path(app)?;
  let text =
    serde_json::to_string_pretty(prefs).map_err(|e| format!("serialize preferences: {e}"))?;
  fs::write(&path, text).map_err(|e| format!("write preferences: {e}"))?;
  tracing::debug!(path = %path.display(), "saved preferences");
  Ok(())
}

#[tauri::command]
fn get_server_status(state: State<'_, ServerState>) -> Result<ServerStatus, String> {
  let guard = state.inner.lock().map_err(|e| e.to_string())?;
  Ok(status_from(guard.as_ref()))
}

#[tauri::command]
fn list_agents() -> Result<Vec<AgentInfo>, String> {
  let config = Config::default();
  Ok(list_agent_infos(&config))
}

#[tauri::command]
fn load_preferences(app: AppHandle) -> Result<UserPreferences, String> {
  read_prefs(&app)
}

#[tauri::command]
fn save_preferences(app: AppHandle, prefs: UserPreferences) -> Result<UserPreferences, String> {
  write_prefs(&app, &prefs)?;
  Ok(prefs)
}

/// Start the API server in the background (no-op if already running).
#[tauri::command]
async fn start_api_server(
  app: AppHandle,
  state: State<'_, ServerState>,
  listen: Option<String>,
  agent: Option<String>,
) -> Result<ServerStatus, String> {
  {
    let guard = state.inner.lock().map_err(|e| e.to_string())?;
    if guard.is_some() {
      return Ok(status_from(guard.as_ref()));
    }
  }

  // Merge with saved preferences
  let mut prefs = read_prefs(&app).unwrap_or_default();
  if let Some(addr) = listen.filter(|s| !s.trim().is_empty()) {
    prefs.listen = addr;
  }
  if let Some(a) = agent.filter(|s| !s.trim().is_empty()) {
    prefs.agent = Some(a);
  }
  write_prefs(&app, &prefs)?;

  let mut config = Config::default();
  config.listen = prefs.listen.clone();
  if let Some(a) = &prefs.agent {
    config
      .set_active_agent(a.clone())
      .map_err(|e| e.to_string())?;
  }

  let listen_addr = config.listen.clone();
  let agent_name = config.active_agent.clone();
  let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

  tauri::async_runtime::spawn(async move {
    let shutdown = async {
      let _ = shutdown_rx.await;
    };
    if let Err(e) = serve_with_shutdown(config, shutdown).await {
      tracing::error!(error = %e, "API server exited with error");
    }
  });

  tokio::time::sleep(std::time::Duration::from_millis(50)).await;

  let mut guard = state.inner.lock().map_err(|e| e.to_string())?;
  *guard = Some(RunningServer {
    listen: listen_addr.clone(),
    agent: agent_name.clone(),
    shutdown: shutdown_tx,
  });

  tracing::info!(
    listen = %listen_addr,
    agent = ?agent_name,
    "API server started from Tauri"
  );
  Ok(status_from(guard.as_ref()))
}

/// Stop the API server if it is running.
#[tauri::command]
fn stop_api_server(state: State<'_, ServerState>) -> Result<ServerStatus, String> {
  let mut guard = state.inner.lock().map_err(|e| e.to_string())?;
  if let Some(server) = guard.take() {
    tracing::info!(listen = %server.listen, "stopping API server");
    let _ = server.shutdown.send(());
  }
  Ok(status_from(None))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
  let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
  let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();

  tauri::Builder::default()
    .plugin(tauri_plugin_shell::init())
    .manage(ServerState {
      inner: Mutex::new(None),
    })
    .invoke_handler(tauri::generate_handler![
      get_server_status,
      list_agents,
      load_preferences,
      save_preferences,
      start_api_server,
      stop_api_server
    ])
    .run(tauri::generate_context!())
    .expect("error while running tauri application");
}
