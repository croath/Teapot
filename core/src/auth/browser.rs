//! Open a URL in the system browser (best-effort).
//!
//! Used by browser-based OAuth login flows (Codex, Claude, Antigravity).

use std::process::Command;

use tracing::warn;

/// Attempt to open `url` in the system browser. Returns `true` on spawn success.
pub fn open_url(url: &str) -> bool {
  let result = {
    #[cfg(target_os = "macos")]
    {
      Command::new("open").arg(url).spawn()
    }
    #[cfg(target_os = "windows")]
    {
      Command::new("cmd").args(["/C", "start", "", url]).spawn()
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
      Command::new("xdg-open").arg(url).spawn()
    }
  };
  match result {
    Ok(_) => true,
    Err(e) => {
      warn!(error = %e, "failed to open browser");
      false
    }
  }
}
