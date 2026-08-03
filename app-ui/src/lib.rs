//! Leptos CSR UI for teaport (server control + playground).

mod app;
mod pages;
mod tauri_api;

pub use app::App;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::wasm_bindgen;

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen(start)]
pub fn start() {
  console_error_panic_hook::set_once();
  leptos::mount::mount_to_body(App);
}
