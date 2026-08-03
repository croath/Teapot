//! Teapot desktop shell: liquid-glass UI host + bundled `teapotx` sidecar.

mod server;

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
    .manage(ServerRuntime::new())
    .setup(|app| {
      let menu = build_app_menu(app.handle())?;
      app.set_menu(menu)?;
      Ok(())
    })
    .on_menu_event(|app, event| {
      let path = match event.id().0.as_str() {
        "settings" => Some("/settings"),
        "about" => Some("/settings/about"),
        "debug" => Some("/settings/debug"),
        "home" => Some("/"),
        _ => None,
      };
      if let Some(path) = path {
        let _ = app.emit("navigate", path);
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
      server::get_config_path,
    ])
    .run(tauri::generate_context!())
    .expect("error while running tauri application");
}

fn build_app_menu(app: &tauri::AppHandle) -> tauri::Result<tauri::menu::Menu<tauri::Wry>> {
  let pkg_name = app.package_info().name.clone();

  let settings_item = MenuItem::with_id(app, "settings", "Settings…", true, Some("CmdOrCtrl+,"))?;
  let about_item = MenuItem::with_id(app, "about", "About Teapot", true, None::<&str>)?;
  let debug_item = MenuItem::with_id(app, "debug", "Debug Logs", true, None::<&str>)?;

  #[cfg(target_os = "macos")]
  let app_submenu = SubmenuBuilder::new(app, &pkg_name)
    .item(&about_item)
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
  let file_submenu = SubmenuBuilder::new(app, "File")
    .item(&settings_item)
    .separator()
    .quit()
    .build()?;

  let edit_submenu = SubmenuBuilder::new(app, "Edit")
    .undo()
    .redo()
    .separator()
    .cut()
    .copy()
    .paste()
    .select_all()
    .build()?;

  let window_submenu = SubmenuBuilder::new(app, "Window")
    .minimize()
    .maximize()
    .separator()
    .close_window()
    .build()?;

  let help_submenu = {
    #[cfg(target_os = "macos")]
    {
      SubmenuBuilder::new(app, "Help").item(&debug_item).build()?
    }
    #[cfg(not(target_os = "macos"))]
    {
      SubmenuBuilder::new(app, "Help")
        .item(&about_item)
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
