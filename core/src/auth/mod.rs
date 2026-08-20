//! Shared auth infrastructure (not provider protocol logic).
//!
//! | Module | Role |
//! |--------|------|
//! | [`store`] | **One JSON file per provider**; typed load/save of native structs |
//! | [`credential`] | Thin trait/CLI types |
//! | [`pkce`] | OAuth PKCE + CSRF state |
//! | [`browser`] | Open system browser |
//!
//! OAuth **callback servers**, HTTP clients / headers, JWT parsers, and on-disk
//! field schemas live under `providers/<name>/` (each provider owns its
//! `StoredAuth` struct and serializes it unchanged).
//!
//! ```text
//! {data_local}/auth/
//!   codex.json        # account map of Codex StoredAuth
//!   claude.json       # account map of Claude StoredAuth
//!   …
//! ```

mod browser;
mod credential;
mod pkce;
mod store;

pub use browser::open_url;
pub use credential::{AuthMethod, LoginOptions};
pub use pkce::{PkceCodes, generate_pkce, generate_state};
pub use store::{AuthStore, default_auth_dir, default_auth_path, sanitize_account_segment};

/// Abort a spawned task when the owner is dropped (OAuth callback servers).
pub(crate) struct AbortOnDrop(pub tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
  fn drop(&mut self) {
    self.0.abort();
  }
}
