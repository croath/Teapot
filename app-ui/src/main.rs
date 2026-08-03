//! Native / non-WASM entry (not used by Trunk; see `lib.rs` for the WASM start).
//!
//! Kept so `cargo check -p teapot-ui` works without a wasm target.

fn main() {
  eprintln!(
    "teapot-ui is a WASM frontend. Build with Trunk:\n  cd app-ui && bun run dev\n  # or: trunk serve"
  );
}
