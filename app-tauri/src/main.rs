//! Tauri desktop binary entrypoint.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
  teapot_tauri_lib::run();
}
