//! Shared auth options (not provider credential payloads).
//!
//! Provider-owned storage structs live under `providers/<name>/` and are
//! carried at the trait boundary as [`crate::providers::AuthEntry`].

/// How a provider obtains credentials.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthMethod {
  /// No interactive auth.
  None,
  /// Browser OAuth with local callback.
  BrowserOAuth,
  /// OAuth device-code flow.
  DeviceCode,
  /// Import a credentials file (e.g. service-account JSON).
  CredentialImport,
}

/// Shared options for browser / device OAuth login.
///
/// Import-style providers (Vertex) use their own options type under
/// `providers/vertex/`.
#[derive(Debug, Clone, Default)]
pub struct LoginOptions {
  /// Do not open a browser; print the URL for manual visit.
  pub no_browser: bool,
  /// Override the local OAuth callback port (`None` = provider default).
  pub callback_port: Option<u16>,
}
