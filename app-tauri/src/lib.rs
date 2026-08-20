//! Teapot desktop shell: liquid-glass UI host + bundled `teapotx` sidecar.

mod auth;
mod i18n;
mod paths;
mod server;
mod telemetry;
mod updater;

use std::sync::atomic::{AtomicBool, Ordering};

use i18n::{Locale, Msg, load_locale, save_locale};
use server::ServerRuntime;
use tauri::menu::{MenuBuilder, MenuItem, SubmenuBuilder};
use tauri::{Emitter, Manager};
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
use tracing_subscriber::EnvFilter;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
  let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
  let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();

  tauri::Builder::default()
    .plugin(tauri_plugin_shell::init())
    .plugin(tauri_plugin_dialog::init())
    .plugin(tauri_plugin_updater::Builder::new().build())
    .plugin(prevent_webview_defaults())
    .manage(ServerRuntime::new())
    .manage(updater::PendingUpdate::new())
    .setup(|app| {
      paths::ensure_migrated(app.handle());
      let locale = load_locale(app.handle());
      let menu = build_app_menu(app.handle(), locale)?;
      app.set_menu(menu)?;
      telemetry::spawn_launch_signals(app.handle());
      Ok(())
    })
    .on_menu_event(|app, event| match event.id().0.as_str() {
      "check-updates" => {
        let _ = app.emit("navigate", "/settings/about");
        let _ = app.emit("updater-check", ());
      }
      id => {
        let path = match id {
          "settings" => Some("/settings"),
          "about" => Some("/settings/about"),
          "debug" => Some("/settings/debug"),
          "home" => Some("/"),
          _ => None,
        };
        if let Some(path) = path {
          let _ = app.emit("navigate", path);
        }
      }
    })
    .invoke_handler(tauri::generate_handler![
      server::get_server_status,
      server::start_server,
      server::stop_server,
      server::get_config,
      server::save_config,
      server::set_provider,
      server::get_logs,
      server::clear_logs,
      server::get_app_info,
      auth::get_auth_status,
      auth::login_provider,
      auth::cancel_login,
      updater::check_for_update,
      updater::install_update,
      set_locale,
      get_locale,
    ])
    .on_window_event(|window, event| {
      if let tauri::WindowEvent::CloseRequested { api, .. } = event {
        if teapotx_is_running(window.app_handle()) {
          api.prevent_close();
          warn_shutdown_teapotx_first(window.app_handle());
        } else {
          window.app_handle().exit(0);
        }
      }
    })
    .build(tauri::generate_context!())
    .expect("error while building tauri application")
    .run(|app, event| {
      if let tauri::RunEvent::ExitRequested { api, .. } = event {
        if teapotx_is_running(app) {
          api.prevent_exit();
          warn_shutdown_teapotx_first(app);
        }
      }
    });
}

fn teapotx_is_running(app: &tauri::AppHandle) -> bool {
  app
    .try_state::<ServerRuntime>()
    .is_some_and(|state| state.is_running())
}

fn warn_shutdown_teapotx_first(app: &tauri::AppHandle) {
  static SHOWING: AtomicBool = AtomicBool::new(false);
  if SHOWING.swap(true, Ordering::SeqCst) {
    return;
  }
  let locale = load_locale(app);
  let mut builder = app
    .dialog()
    .message(locale.t(Msg::CloseGuardMessage))
    .title(locale.t(Msg::CloseGuardTitle))
    .kind(MessageDialogKind::Warning);
  if let Some(window) = app.get_webview_window("main") {
    builder = builder.parent(&window);
  }
  builder.show(|_| {
    SHOWING.store(false, Ordering::SeqCst);
  });
}

#[tauri::command]
fn set_locale(app: tauri::AppHandle, locale: String) -> Result<(), String> {
  let parsed = Locale::parse(&locale);
  save_locale(&app, parsed)?;
  let menu = build_app_menu(&app, parsed).map_err(|e| e.to_string())?;
  app.set_menu(menu).map_err(|e| e.to_string())?;
  Ok(())
}

#[tauri::command]
fn get_locale(app: tauri::AppHandle) -> String {
  load_locale(&app).id().to_string()
}

/// Hide browser-like webview chrome (Reload / Inspect / F5 / view-source) in
/// both `cargo tauri dev` and release builds.
fn prevent_webview_defaults() -> tauri::plugin::TauriPlugin<tauri::Wry> {
  use tauri_plugin_prevent_default::Flags;

  let builder = tauri_plugin_prevent_default::Builder::new().with_flags(
    Flags::CONTEXT_MENU
      | Flags::DEV_TOOLS
      | Flags::RELOAD
      | Flags::FIND
      | Flags::DOWNLOADS
      | Flags::SOURCE
      | Flags::OPEN
      | Flags::PRINT
      | Flags::CARET_BROWSING,
  );

  #[cfg(windows)]
  let builder = builder.platform(
    tauri_plugin_prevent_default::PlatformOptions::new()
      .default_context_menus(false)
      .browser_accelerator_keys(false),
  );

  builder.build()
}

fn build_app_menu(
  app: &tauri::AppHandle,
  locale: Locale,
) -> tauri::Result<tauri::menu::Menu<tauri::Wry>> {
  let pkg_name = app.package_info().name.clone();
  let t = |msg| locale.t(msg);

  let settings_item = MenuItem::with_id(
    app,
    "settings",
    t(Msg::MenuSettings),
    true,
    Some("CmdOrCtrl+,"),
  )?;
  let about_item = MenuItem::with_id(app, "about", t(Msg::MenuAbout), true, None::<&str>)?;
  let debug_item = MenuItem::with_id(app, "debug", t(Msg::MenuDebug), true, None::<&str>)?;
  let check_updates_item = MenuItem::with_id(
    app,
    "check-updates",
    t(Msg::MenuCheckUpdates),
    true,
    None::<&str>,
  )?;

  #[cfg(target_os = "macos")]
  let app_submenu = SubmenuBuilder::new(app, &pkg_name)
    .item(&about_item)
    .item(&check_updates_item)
    .separator()
    .item(&settings_item)
    .separator()
    .services()
    .separator()
    .hide()
    .hide_others()
    .separator()
    .quit()
    .build()?;

  #[cfg(not(target_os = "macos"))]
  let file_submenu = SubmenuBuilder::new(app, t(Msg::MenuFile))
    .item(&settings_item)
    .separator()
    .quit()
    .build()?;

  let edit_submenu = SubmenuBuilder::new(app, t(Msg::MenuEdit))
    .undo()
    .redo()
    .separator()
    .cut()
    .copy()
    .paste()
    .select_all()
    .build()?;

  let window_submenu = SubmenuBuilder::new(app, t(Msg::MenuWindow))
    .minimize()
    .maximize()
    .separator()
    .close_window()
    .build()?;

  let help_submenu = {
    #[cfg(target_os = "macos")]
    {
      SubmenuBuilder::new(app, t(Msg::MenuHelp))
        .item(&debug_item)
        .build()?
    }
    #[cfg(not(target_os = "macos"))]
    {
      SubmenuBuilder::new(app, t(Msg::MenuHelp))
        .item(&about_item)
        .item(&check_updates_item)
        .separator()
        .item(&debug_item)
        .build()?
    }
  };

  #[cfg(target_os = "macos")]
  {
    MenuBuilder::new(app)
      .item(&app_submenu)
      .item(&edit_submenu)
      .item(&window_submenu)
      .item(&help_submenu)
      .build()
  }

  #[cfg(not(target_os = "macos"))]
  {
    MenuBuilder::new(app)
      .item(&file_submenu)
      .item(&edit_submenu)
      .item(&window_submenu)
      .item(&help_submenu)
      .build()
  }
}
