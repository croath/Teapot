//! Teapot desktop shell: liquid-glass UI host + bundled `teapotx` sidecar.

mod i18n;
mod server;
mod updater;

use i18n::{Locale, load_locale, save_locale};
use server::ServerRuntime;
use tauri::Emitter;
use tauri::menu::{MenuBuilder, MenuItem, SubmenuBuilder};
use tracing_subscriber::EnvFilter;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
  let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
  let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();

  tauri::Builder::default()
    .plugin(tauri_plugin_shell::init())
    .plugin(tauri_plugin_updater::Builder::new().build())
    .plugin(prevent_webview_defaults())
    .manage(ServerRuntime::new())
    .manage(updater::PendingUpdate::new())
    .setup(|app| {
      let locale = load_locale(app.handle());
      let menu = build_app_menu(app.handle(), locale)?;
      app.set_menu(menu)?;
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
      server::get_logs,
      server::clear_logs,
      server::get_app_info,
      updater::check_for_update,
      updater::install_update,
      set_locale,
      get_locale,
    ])
    .run(tauri::generate_context!())
    .expect("error while running tauri application");
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
  let copy = locale.menu();

  let settings_item = MenuItem::with_id(app, "settings", copy.settings, true, Some("CmdOrCtrl+,"))?;
  let about_item = MenuItem::with_id(app, "about", copy.about, true, None::<&str>)?;
  let debug_item = MenuItem::with_id(app, "debug", copy.debug, true, None::<&str>)?;
  let check_updates_item =
    MenuItem::with_id(app, "check-updates", copy.check_updates, true, None::<&str>)?;

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
  let file_submenu = SubmenuBuilder::new(app, copy.file)
    .item(&settings_item)
    .separator()
    .quit()
    .build()?;

  let edit_submenu = SubmenuBuilder::new(app, copy.edit)
    .undo()
    .redo()
    .separator()
    .cut()
    .copy()
    .paste()
    .select_all()
    .build()?;

  let window_submenu = SubmenuBuilder::new(app, copy.window)
    .minimize()
    .maximize()
    .separator()
    .close_window()
    .build()?;

  let help_submenu = {
    #[cfg(target_os = "macos")]
    {
      SubmenuBuilder::new(app, copy.help)
        .item(&debug_item)
        .build()?
    }
    #[cfg(not(target_os = "macos"))]
    {
      SubmenuBuilder::new(app, copy.help)
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
