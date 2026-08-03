//! Binary entry for Trunk / local WASM builds.

fn main() {
  console_error_panic_hook::set_once();
  leptos::mount::mount_to_body(teaport_ui::App);
}
