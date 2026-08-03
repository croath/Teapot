//! Thin bridge to Tauri `invoke` when running inside the desktop shell.
//! Preferences fall back to `localStorage` in plain browser mode.

use js_sys::{Function, Promise, Reflect};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;

const LS_PREFS_KEY: &str = "teaport.preferences";

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerStatus {
  pub running: bool,
  pub listen: Option<String>,
  pub base_url: Option<String>,
  #[serde(default)]
  pub agent: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentInfo {
  pub name: String,
  pub command: String,
  pub description: String,
  pub installed: bool,
  #[serde(default)]
  pub binary: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UserPreferences {
  #[serde(default = "default_listen")]
  pub listen: String,
  #[serde(default)]
  pub agent: Option<String>,
}

fn default_listen() -> String {
  "127.0.0.1:8080".into()
}

/// Whether `window.__TAURI__` is present (desktop app).
pub fn is_tauri() -> bool {
  web_sys::window()
    .and_then(|w| Reflect::get(&w, &"__TAURI__".into()).ok())
    .map(|v| !v.is_undefined() && !v.is_null())
    .unwrap_or(false)
}

async fn invoke(cmd: &str, args: Value) -> Result<Value, String> {
  let window = web_sys::window().ok_or_else(|| "no window".to_string())?;
  let tauri = Reflect::get(&window, &"__TAURI__".into())
    .map_err(|_| "Tauri bridge not available (run the desktop app)".to_string())?;
  if tauri.is_undefined() || tauri.is_null() {
    return Err("Tauri bridge not available (run the desktop app)".into());
  }

  let core =
    Reflect::get(&tauri, &"core".into()).map_err(|_| "missing __TAURI__.core".to_string())?;
  let invoke_fn = Reflect::get(&core, &"invoke".into())
    .map_err(|_| "missing __TAURI__.core.invoke".to_string())?;
  let invoke_fn: Function = invoke_fn
    .dyn_into()
    .map_err(|_| "invoke is not a function".to_string())?;

  let args_js = js_sys::JSON::parse(&serde_json::to_string(&args).map_err(|e| e.to_string())?)
    .map_err(|_| "failed to serialize invoke args".to_string())?;

  let result = invoke_fn
    .call2(&core, &cmd.into(), &args_js)
    .map_err(|e| format!("invoke failed: {e:?}"))?;

  let promise = Promise::resolve(&result);
  let value = JsFuture::from(promise)
    .await
    .map_err(|e| format!("invoke rejected: {e:?}"))?;

  if value.is_undefined() || value.is_null() {
    return Ok(Value::Null);
  }

  let json = js_sys::JSON::stringify(&value)
    .map_err(|_| "failed to stringify invoke result".to_string())?
    .as_string()
    .ok_or_else(|| "invoke result is not a string".to_string())?;

  serde_json::from_str(&json).map_err(|e| e.to_string())
}

pub async fn get_server_status() -> Result<ServerStatus, String> {
  let value = invoke("get_server_status", json!({})).await?;
  serde_json::from_value(value).map_err(|e| e.to_string())
}

pub async fn list_agents() -> Result<Vec<AgentInfo>, String> {
  if is_tauri() {
    let value = invoke("list_agents", json!({})).await?;
    return serde_json::from_value(value).map_err(|e| e.to_string());
  }
  // Browser fallback: known defaults (install status unknown)
  Ok(vec![
    AgentInfo {
      name: "codex".into(),
      command: "codex".into(),
      description: "OpenAI Codex CLI agent".into(),
      installed: true,
      binary: None,
    },
    AgentInfo {
      name: "claude".into(),
      command: "claude".into(),
      description: "Anthropic Claude Code CLI agent".into(),
      installed: true,
      binary: None,
    },
    AgentInfo {
      name: "grok".into(),
      command: "grok".into(),
      description: "xAI Grok CLI agent".into(),
      installed: true,
      binary: None,
    },
    AgentInfo {
      name: "antigravity".into(),
      command: "antigravity-cli".into(),
      description: "Antigravity CLI agent".into(),
      installed: true,
      binary: None,
    },
  ])
}

pub async fn load_preferences() -> Result<UserPreferences, String> {
  if is_tauri() {
    let value = invoke("load_preferences", json!({})).await?;
    return serde_json::from_value(value).map_err(|e| e.to_string());
  }
  load_prefs_local_storage()
}

pub async fn save_preferences(prefs: &UserPreferences) -> Result<UserPreferences, String> {
  if is_tauri() {
    let value = invoke("save_preferences", json!({ "prefs": prefs })).await?;
    return serde_json::from_value(value).map_err(|e| e.to_string());
  }
  save_prefs_local_storage(prefs)?;
  Ok(prefs.clone())
}

pub async fn start_api_server(
  listen: Option<String>,
  agent: Option<String>,
) -> Result<ServerStatus, String> {
  let value = invoke(
    "start_api_server",
    json!({ "listen": listen, "agent": agent }),
  )
  .await?;
  serde_json::from_value(value).map_err(|e| e.to_string())
}

pub async fn stop_api_server() -> Result<ServerStatus, String> {
  let value = invoke("stop_api_server", json!({})).await?;
  serde_json::from_value(value).map_err(|e| e.to_string())
}

fn load_prefs_local_storage() -> Result<UserPreferences, String> {
  let window = web_sys::window().ok_or_else(|| "no window".to_string())?;
  let storage = window
    .local_storage()
    .map_err(|_| "localStorage unavailable".to_string())?
    .ok_or_else(|| "localStorage unavailable".to_string())?;
  match storage
    .get_item(LS_PREFS_KEY)
    .map_err(|_| "localStorage get failed".to_string())?
  {
    Some(text) => serde_json::from_str(&text).map_err(|e| e.to_string()),
    None => Ok(UserPreferences::default()),
  }
}

fn save_prefs_local_storage(prefs: &UserPreferences) -> Result<(), String> {
  let window = web_sys::window().ok_or_else(|| "no window".to_string())?;
  let storage = window
    .local_storage()
    .map_err(|_| "localStorage unavailable".to_string())?
    .ok_or_else(|| "localStorage unavailable".to_string())?;
  let text = serde_json::to_string(prefs).map_err(|e| e.to_string())?;
  storage
    .set_item(LS_PREFS_KEY, &text)
    .map_err(|_| "localStorage set failed".to_string())
}
