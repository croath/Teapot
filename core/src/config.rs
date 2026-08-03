//! Server and agent configuration.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Top-level configuration for the API server and agent backends.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
  /// HTTP listen address (host:port).
  pub listen: String,
  /// Optional API key. When set, clients must send `Authorization: Bearer <key>`
  /// or `x-api-key: <key>`.
  pub api_key: Option<String>,
  /// Default agent used when the request model cannot be mapped.
  pub default_agent: String,
  /// When set, only this agent CLI is used: models list is filtered to it, and
  /// it becomes the default routing target. Set via CLI `--agent` / UI select.
  pub active_agent: Option<String>,
  /// Map of agent name -> agent definition.
  pub agents: HashMap<String, AgentConfig>,
  /// Optional model alias map: request model id -> agent name.
  pub model_map: HashMap<String, String>,
}

impl Default for Config {
  fn default() -> Self {
    Self {
      listen: "127.0.0.1:8080".into(),
      api_key: None,
      default_agent: "codex".into(),
      active_agent: None,
      agents: default_agents(),
      model_map: default_model_map(),
    }
  }
}

/// Configuration for a single agent CLI backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentConfig {
  /// Display name.
  pub name: String,
  /// Executable to run (looked up on PATH if relative).
  pub command: String,
  /// Argument template. `{prompt}` is replaced with the user prompt.
  /// `{system}` is replaced with the optional system prompt (may be empty).
  pub args: Vec<String>,
  /// Working directory for the process (optional).
  pub cwd: Option<PathBuf>,
  /// Extra environment variables.
  pub env: HashMap<String, String>,
  /// When true, pass the prompt on stdin instead of as an argument.
  pub prompt_via_stdin: bool,
  /// Timeout in seconds (0 = no timeout).
  pub timeout_secs: u64,
  /// Description shown in `/models` listings.
  pub description: String,
  /// Optional argv used to list models from this CLI (stdout parsed for model ids).
  /// Example: `["models"]` → runs `codex models`.
  /// Empty means skip CLI probe and use built-in / alias catalogs only.
  #[serde(default)]
  pub list_models_args: Vec<String>,
}

impl Default for AgentConfig {
  fn default() -> Self {
    Self {
      name: "unnamed".into(),
      command: "echo".into(),
      args: vec!["{prompt}".into()],
      cwd: None,
      env: HashMap::new(),
      prompt_via_stdin: false,
      timeout_secs: 600,
      description: String::new(),
      list_models_args: Vec::new(),
    }
  }
}

fn default_agents() -> HashMap<String, AgentConfig> {
  let mut agents = HashMap::new();

  agents.insert(
    "codex".into(),
    AgentConfig {
      name: "codex".into(),
      command: "codex".into(),
      args: vec![
        "exec".into(),
        "--skip-git-repo-check".into(),
        "{prompt}".into(),
      ],
      prompt_via_stdin: false,
      timeout_secs: 900,
      description: "OpenAI Codex CLI agent".into(),
      // Best-effort; ignored when the subcommand is unavailable.
      list_models_args: vec!["models".into()],
      ..Default::default()
    },
  );

  agents.insert(
    "claude".into(),
    AgentConfig {
      name: "claude".into(),
      command: "claude".into(),
      args: vec![
        "-p".into(),
        "--output-format".into(),
        "text".into(),
        "{prompt}".into(),
      ],
      prompt_via_stdin: false,
      timeout_secs: 900,
      description: "Anthropic Claude Code CLI agent".into(),
      list_models_args: vec![],
      ..Default::default()
    },
  );

  agents.insert(
    "grok".into(),
    AgentConfig {
      name: "grok".into(),
      command: "grok".into(),
      args: vec!["{prompt}".into()],
      prompt_via_stdin: false,
      timeout_secs: 900,
      description: "xAI Grok CLI agent".into(),
      list_models_args: vec![],
      ..Default::default()
    },
  );

  agents.insert(
    "antigravity".into(),
    AgentConfig {
      name: "antigravity".into(),
      command: "antigravity-cli".into(),
      args: vec!["{prompt}".into()],
      prompt_via_stdin: false,
      timeout_secs: 900,
      description: "Antigravity CLI agent".into(),
      list_models_args: vec![],
      ..Default::default()
    },
  );

  agents.insert(
    "antigravity-cli".into(),
    AgentConfig {
      name: "antigravity-cli".into(),
      command: "antigravity-cli".into(),
      args: vec!["{prompt}".into()],
      prompt_via_stdin: false,
      timeout_secs: 900,
      description: "Antigravity CLI agent (alias)".into(),
      list_models_args: vec![],
      ..Default::default()
    },
  );

  agents
}

fn default_model_map() -> HashMap<String, String> {
  HashMap::from([
    ("codex".into(), "codex".into()),
    ("claude".into(), "claude".into()),
    ("grok".into(), "grok".into()),
    ("antigravity".into(), "antigravity".into()),
    ("antigravity-cli".into(), "antigravity".into()),
    ("gpt-4o".into(), "codex".into()),
    ("gpt-4.1".into(), "codex".into()),
    ("gpt-5".into(), "codex".into()),
    ("o3".into(), "codex".into()),
    ("o4-mini".into(), "codex".into()),
    ("claude-sonnet-4-20250514".into(), "claude".into()),
    ("claude-opus-4-20250514".into(), "claude".into()),
    ("claude-3-5-sonnet-latest".into(), "claude".into()),
    ("claude-3-7-sonnet-latest".into(), "claude".into()),
  ])
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

  /// Pin the server to a single agent CLI (also becomes `default_agent`).
  ///
  /// Returns an error if the name is not present in `agents`.
  pub fn set_active_agent(&mut self, name: impl Into<String>) -> anyhow::Result<()> {
    let name = name.into();
    if !self.agents.contains_key(&name) {
      let known: Vec<_> = self.agents.keys().cloned().collect();
      anyhow::bail!(
        "unknown agent `{name}`; configured agents: {}",
        known.join(", ")
      );
    }
    self.default_agent = name.clone();
    self.active_agent = Some(name);
    Ok(())
  }

  /// Effective default agent (active pin if set, else `default_agent`).
  pub fn effective_default_agent(&self) -> &str {
    self
      .active_agent
      .as_deref()
      .unwrap_or(self.default_agent.as_str())
  }

  /// Resolve which agent should handle a given model id.
  pub fn resolve_agent<'a>(&'a self, model: &'a str) -> &'a str {
    // When an active agent is pinned, always route to it unless the model
    // explicitly maps to the same agent family via model_map / agent name.
    if let Some(active) = self.active_agent.as_deref() {
      if let Some(mapped) = self.model_map.get(model) {
        if mapped == active {
          return mapped.as_str();
        }
      }
      if model == active || self.agents.contains_key(model) && model == active {
        return active;
      }
      // Prefix match still allowed when it points at the active agent
      if let Some((prefix, _)) = model.split_once(['/', ':']) {
        if prefix == active {
          return active;
        }
        if let Some(mapped) = self.model_map.get(prefix) {
          if mapped == active {
            return mapped.as_str();
          }
        }
      }
      // Any other model id is still served by the pinned agent
      return active;
    }

    if let Some(agent) = self.model_map.get(model) {
      return agent.as_str();
    }
    if self.agents.contains_key(model) {
      return model;
    }
    if let Some((prefix, _)) = model.split_once(['/', ':']) {
      if self.agents.contains_key(prefix) {
        return prefix;
      }
      if let Some(agent) = self.model_map.get(prefix) {
        return agent.as_str();
      }
    }
    self.default_agent.as_str()
  }

  /// Look up an agent configuration by name.
  pub fn agent(&self, name: &str) -> Option<&AgentConfig> {
    self.agents.get(name)
  }

  /// Names of agents to consider for model discovery (all, or only active).
  pub fn discovery_agent_names(&self) -> Vec<String> {
    if let Some(active) = &self.active_agent {
      vec![active.clone()]
    } else {
      let mut names: Vec<_> = self.agents.keys().cloned().collect();
      names.sort();
      names
    }
  }
}

/// Default path candidates for the config file.
pub fn default_config_paths() -> Vec<PathBuf> {
  let mut paths = Vec::new();
  paths.push(PathBuf::from("teaport.toml"));
  paths.push(PathBuf::from("config.toml"));
  if let Some(proj) = directories::ProjectDirs::from("dev", "teaport", "teaport") {
    paths.push(proj.config_dir().join("config.toml"));
  }
  paths
}
