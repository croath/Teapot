//! Server configuration (listen address, API key, optional provider pin).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Top-level configuration for the API server.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
  /// HTTP listen address (host:port).
  pub listen: String,
  /// Optional API key. When set, clients must send `Authorization: Bearer <key>`
  /// or `x-api-key: <key>`.
  pub api_key: Option<String>,
  /// Optional default provider (`codex-cli`, `claude-cli`, `xai`, `antigravity`, `vertex`).
  pub provider: Option<String>,
  /// When true (default), stream provider tool/command progress as optional
  /// `reasoning_content` / `status` fields on Chat Completions deltas.
  #[serde(default = "default_true")]
  pub include_progress: bool,
}

fn default_true() -> bool {
  true
}

impl Default for Config {
  fn default() -> Self {
    Self {
      listen: "127.0.0.1:8080".into(),
      api_key: None,
      provider: None,
      include_progress: true,
    }
  }
}

impl Config {
  /// Load configuration from a TOML file, falling back to defaults on missing file.
  pub fn load_from_path(path: impl AsRef<Path>) -> anyhow::Result<Self> {
    let path = path.as_ref();
    if !path.exists() {
      tracing::info!(?path, "config file not found, using defaults");
      return Ok(Self::default());
    }
    let text = std::fs::read_to_string(path)?;
    let cfg: Config = toml::from_str(&text)?;
    Ok(cfg)
  }

  /// Pin the server to a single provider key.
  pub fn set_provider(&mut self, name: impl Into<String>) {
    let name = name.into();
    self.provider = if name.trim().is_empty() {
      None
    } else {
      Some(name)
    };
  }

  /// The configured provider key, if any.
  pub fn provider_name(&self) -> Option<&str> {
    self.provider.as_deref()
  }
}

/// Default path candidates for the config file.
pub fn default_config_paths() -> Vec<PathBuf> {
  let mut paths = Vec::new();
  paths.push(PathBuf::from("teapot.toml"));
  paths.push(PathBuf::from("config.toml"));
  if let Ok(file) = crate::paths::default_config_file() {
    if !paths.contains(&file) {
      paths.push(file);
    }
  }
  // Linux: ProjectDirs config_dir (`~/.config/teapot`) differs from data_local.
  if let Some(proj) = crate::paths::project_dirs() {
    let cfg = proj.config_dir().join("config.toml");
    if !paths.contains(&cfg) {
      paths.push(cfg);
    }
  }
  paths
}
