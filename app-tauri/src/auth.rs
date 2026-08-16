//! Provider auth status and interactive login for the desktop UI.

use std::path::PathBuf;
use std::sync::mpsc;

use serde::Serialize;
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;
use teapot_core::{
  AuthMethod, AuthStore, LoginOptions, ProviderAuth, ProviderKind, import_service_account,
  provider_for,
};

use crate::server::ServerRuntime;

pub const DEFAULT_PROVIDER: &str = "codex";

struct LoginGuard<'a>(&'a ServerRuntime);

impl Drop for LoginGuard<'_> {
  fn drop(&mut self) {
    self.0.end_login();
  }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthStatus {
  pub provider: String,
  pub authenticated: bool,
  pub account: Option<String>,
  pub auth_method: String,
}

pub fn parse_provider(name: &str) -> Result<ProviderKind, String> {
  let name = name.trim();
  let name = if name.is_empty() {
    DEFAULT_PROVIDER
  } else {
    name
  };
  ProviderKind::parse(name).map_err(|e| e.to_string())
}

pub fn auth_method_id(method: AuthMethod) -> &'static str {
  match method {
    AuthMethod::None => "none",
    AuthMethod::BrowserOAuth => "browser-oauth",
    AuthMethod::DeviceCode => "device-code",
    AuthMethod::CredentialImport => "credential-import",
  }
}

pub fn status_for(kind: ProviderKind) -> Result<AuthStatus, String> {
  let store = AuthStore::local().map_err(|e| e.to_string())?;
  let provider = provider_for(kind);
  let entries = provider.load_auth(&store).map_err(|e| e.to_string())?;
  let first = entries.into_iter().next();
  Ok(AuthStatus {
    provider: kind.as_str().to_string(),
    authenticated: first.is_some(),
    account: first.as_ref().map(|entry| {
      entry
        .email()
        .filter(|s| !s.is_empty())
        .unwrap_or(&entry.account_key())
        .to_string()
    }),
    auth_method: auth_method_id(provider.auth_method()).into(),
  })
}

#[tauri::command]
pub fn get_auth_status(provider: String) -> Result<AuthStatus, String> {
  status_for(parse_provider(&provider)?)
}

#[tauri::command]
pub async fn login_provider(
  app: AppHandle,
  state: State<'_, ServerRuntime>,
  provider: String,
) -> Result<AuthStatus, String> {
  let kind = parse_provider(&provider)?;
  let Some(cancel_rx) = state.try_begin_login() else {
    return Err("Sign-in is already in progress.".into());
  };
  let _guard = LoginGuard(&state);

  state.log(&app, format!("[teapot] signing in to {}…", kind.as_str()));
  let result = tokio::select! {
    result = run_login(&app, kind) => result,
    _ = cancel_rx => Err("Sign-in cancelled.".into()),
  };

  match &result {
    Ok(status) => {
      let who = status.account.as_deref().unwrap_or("account");
      state.log(
        &app,
        format!("[teapot] signed in to {} as {who}", kind.as_str()),
      );
    }
    Err(err) if err == "Sign-in cancelled." => {}
    Err(err) => {
      state.log(
        &app,
        format!("[teapot] sign-in to {} failed: {err}", kind.as_str()),
      );
    }
  }

  result
}

#[tauri::command]
pub fn cancel_login(app: AppHandle, state: State<'_, ServerRuntime>) -> Result<(), String> {
  if state.request_cancel_login() {
    state.log(&app, "[teapot] sign-in cancelled".into());
  }
  Ok(())
}

async fn run_login(app: &AppHandle, kind: ProviderKind) -> Result<AuthStatus, String> {
  let store = AuthStore::local().map_err(|e| e.to_string())?;
  let provider = provider_for(kind);

  let entry = match provider.auth_method() {
    AuthMethod::CredentialImport => {
      let path = pick_json_file(app).await?;
      import_service_account(&store, &path, None, None)
        .await
        .map_err(|e| e.to_string())?
    }
    AuthMethod::None => {
      return Err(format!(
        "provider `{}` does not support auth",
        kind.as_str()
      ));
    }
    AuthMethod::BrowserOAuth | AuthMethod::DeviceCode => provider
      .login(&store, LoginOptions::default())
      .await
      .map_err(|e| e.to_string())?,
  };

  // Desktop UI is one active account per provider; drop leftovers so serve
  // and status pick the account that just signed in.
  keep_only_account(&*provider, &store, &entry.account_key())?;

  status_for(kind)
}

fn keep_only_account(
  provider: &dyn ProviderAuth,
  store: &AuthStore,
  account: &str,
) -> Result<(), String> {
  let entries = provider.load_auth(store).map_err(|e| e.to_string())?;
  for entry in entries {
    if entry.account_key() != account {
      provider
        .clear_auth(store, Some(&entry.account_key()))
        .map_err(|e| e.to_string())?;
    }
  }
  Ok(())
}

async fn pick_json_file(app: &AppHandle) -> Result<PathBuf, String> {
  let dialog = app.dialog().file().add_filter("JSON", &["json"]);
  let (tx, rx) = mpsc::channel();
  dialog.pick_file(move |file| {
    let _ = tx.send(file);
  });

  let picked = tauri::async_runtime::spawn_blocking(move || rx.recv())
    .await
    .map_err(|e| format!("file dialog: {e}"))?
    .map_err(|e| format!("file dialog: {e}"))?;

  match picked {
    Some(path) => path.into_path().map_err(|e| e.to_string()),
    None => Err("Sign-in cancelled.".into()),
  }
}
