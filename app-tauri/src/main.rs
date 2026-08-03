//! Tauri desktop binary entrypoint.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
  teaport_tauri_lib::run();
}
