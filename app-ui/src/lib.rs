//! Leptos CSR UI for Teapot desktop (liquid glass).

mod app;
mod components;
mod pages;
mod tauri_bridge;

pub use app::App;

/// WASM entrypoint used by Trunk (`data-target-name="teapot_ui"`).
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() {
  console_error_panic_hook::set_once();
  leptos::mount::mount_to_body(App);
}
