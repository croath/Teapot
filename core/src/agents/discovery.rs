//! Discover models exposed by **installed** agent CLIs.
//!
//! Only agents whose executable is present on `PATH` (or as an absolute path)
//! contribute models. Discovery order for each installed agent:
//!
//! 1. Optional CLI probe via [`AgentConfig::list_models_args`]
//! 2. Built-in known model ids for that agent (if any)
//! 3. The agent name itself as a model id
//! 4. Config `model_map` aliases that resolve to the agent

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use tokio::process::Command;
use tracing::{debug, warn};

use crate::config::{AgentConfig, Config};

/// A model that can be requested through Teaport APIs.
#[derive(Debug, Clone)]
pub struct DiscoveredModel {
  /// Model id clients pass as `model`.
  pub id: String,
  /// Backing agent name in config.
  pub agent: String,
  /// Human-readable label.
  pub display_name: String,
  /// Absolute path to the agent binary when known.
  pub binary: Option<String>,
  /// Source of the model entry.
  pub source: ModelSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelSource {
  /// Agent key itself (e.g. `codex`).
  Agent,
  /// Entry from `model_map` pointing at an installed agent.
  Alias,
  /// Parsed from the agent CLI's list-models output.
  CliProbe,
  /// Built-in catalog for a known agent family.
  Builtin,
}

/// Resolve the agent binary path if the command is available.
pub fn resolve_binary(command: &str) -> Option<String> {
  let path = Path::new(command);
  if path.is_absolute() || command.contains('/') || command.contains('\\') {
    if path.is_file() {
      return Some(path.display().to_string());
    }
    return None;
  }
  which::which(command).ok().map(|p| p.display().to_string())
}

/// Whether this agent CLI is installed and invokable.
pub fn is_agent_installed(agent: &AgentConfig) -> bool {
  resolve_binary(&agent.command).is_some()
}

/// List every model id supported by installed agent CLIs.
///
/// When [`Config::active_agent`] is set, only that agent is considered.
pub async fn discover_models(config: &Config) -> Vec<DiscoveredModel> {
  // agent_name -> binary path
  let mut installed: BTreeMap<String, (AgentConfig, String)> = BTreeMap::new();
  let wanted = config.discovery_agent_names();

  for name in &wanted {
    let Some(agent) = config.agents.get(name) else {
      debug!(agent = %name, "active agent not found in config");
      continue;
    };
    if let Some(bin) = resolve_binary(&agent.command) {
      installed.insert(name.clone(), (agent.clone(), bin));
    } else {
      debug!(agent = %name, command = %agent.command, "agent CLI not installed, skipping");
    }
  }

  if let Some(active) = &config.active_agent {
    if installed.is_empty() {
      warn!(
        agent = %active,
        "active agent is set but its CLI is not installed; models list will be empty"
      );
    }
  }

  // Prefer canonical agent names: if both "antigravity" and "antigravity-cli"
  // share the same binary, still list both keys when present in config.
  let mut by_id: BTreeMap<String, DiscoveredModel> = BTreeMap::new();

  for (agent_name, (agent, binary)) in &installed {
    // 1) CLI probe
    let probed = probe_cli_models(agent).await;
    for model_id in probed {
      insert_model(
        &mut by_id,
        DiscoveredModel {
          id: model_id.clone(),
          agent: agent_name.clone(),
          display_name: format!("{model_id} ({agent_name})"),
          binary: Some(binary.clone()),
          source: ModelSource::CliProbe,
        },
      );
    }

    // 2) Built-in catalog for this agent family
    for model_id in builtin_models_for(agent_name, &agent.command) {
      insert_model(
        &mut by_id,
        DiscoveredModel {
          id: model_id.clone(),
          agent: agent_name.clone(),
          display_name: display_name_for(&model_id, agent),
          binary: Some(binary.clone()),
          source: ModelSource::Builtin,
        },
      );
    }

    // 3) Agent name as model id
    insert_model(
      &mut by_id,
      DiscoveredModel {
        id: agent_name.clone(),
        agent: agent_name.clone(),
        display_name: if agent.description.is_empty() {
          agent_name.clone()
        } else {
          agent.description.clone()
        },
        binary: Some(binary.clone()),
        source: ModelSource::Agent,
      },
    );
  }

  // 4) model_map aliases only for installed (and selected) agents
  for (alias, agent_name) in &config.model_map {
    let Some((agent, binary)) = installed.get(agent_name) else {
      continue;
    };
    insert_model(
      &mut by_id,
      DiscoveredModel {
        id: alias.clone(),
        agent: agent_name.clone(),
        display_name: display_name_for(alias, agent),
        binary: Some(binary.clone()),
        source: ModelSource::Alias,
      },
    );
  }

  by_id.into_values().collect()
}

/// Summarize configured agents for UIs / CLI.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentInfo {
  pub name: String,
  pub command: String,
  pub description: String,
  pub installed: bool,
  pub binary: Option<String>,
}

/// List all configured agents with install status (not filtered by active_agent).
pub fn list_agent_infos(config: &Config) -> Vec<AgentInfo> {
  let mut names: Vec<_> = config.agents.keys().cloned().collect();
  names.sort();
  names
    .into_iter()
    .filter_map(|name| {
      let agent = config.agents.get(&name)?;
      let binary = resolve_binary(&agent.command);
      Some(AgentInfo {
        name,
        command: agent.command.clone(),
        description: agent.description.clone(),
        installed: binary.is_some(),
        binary,
      })
    })
    .collect()
}

fn insert_model(map: &mut BTreeMap<String, DiscoveredModel>, model: DiscoveredModel) {
  // Prefer richer sources if the same id appears twice.
  match map.get(&model.id) {
    Some(existing) if source_rank(existing.source) >= source_rank(model.source) => {}
    _ => {
      map.insert(model.id.clone(), model);
    }
  }
}

fn source_rank(source: ModelSource) -> u8 {
  match source {
    ModelSource::CliProbe => 4,
    ModelSource::Builtin => 3,
    ModelSource::Agent => 2,
    ModelSource::Alias => 1,
  }
}

fn display_name_for(id: &str, agent: &AgentConfig) -> String {
  if !agent.description.is_empty() && id == agent.name {
    return agent.description.clone();
  }
  id.to_string()
}

/// Known model ids for popular agent CLIs (used when CLI probe yields nothing).
fn builtin_models_for(agent_name: &str, command: &str) -> Vec<String> {
  let key = agent_name.to_ascii_lowercase();
  let cmd = command.to_ascii_lowercase();
  let family = if key.contains("claude") || cmd == "claude" {
    "claude"
  } else if key.contains("codex") || cmd == "codex" {
    "codex"
  } else if key.contains("grok") || cmd == "grok" {
    "grok"
  } else if key.contains("antigravity") || cmd.contains("antigravity") {
    "antigravity"
  } else {
    return Vec::new();
  };

  match family {
    "claude" => vec![
      "claude-sonnet-4-20250514".into(),
      "claude-opus-4-20250514".into(),
      "claude-3-7-sonnet-latest".into(),
      "claude-3-5-sonnet-latest".into(),
      "claude-3-5-haiku-latest".into(),
    ],
    "codex" => vec![
      "gpt-5".into(),
      "gpt-4.1".into(),
      "gpt-4o".into(),
      "o3".into(),
      "o4-mini".into(),
    ],
    "grok" => vec!["grok-3".into(), "grok-3-mini".into(), "grok-2".into()],
    "antigravity" => Vec::new(),
    _ => Vec::new(),
  }
}

/// Run the agent CLI with `list_models_args` and parse model ids from stdout.
async fn probe_cli_models(agent: &AgentConfig) -> Vec<String> {
  if agent.list_models_args.is_empty() {
    return Vec::new();
  }

  let mut cmd = Command::new(&agent.command);
  cmd
    .args(&agent.list_models_args)
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .stdin(Stdio::null())
    .kill_on_drop(true);

  if let Some(cwd) = &agent.cwd {
    cmd.current_dir(cwd);
  }
  for (k, v) in &agent.env {
    cmd.env(k, v);
  }

  let output = match tokio::time::timeout(Duration::from_secs(8), cmd.output()).await {
    Ok(Ok(out)) => out,
    Ok(Err(e)) => {
      warn!(command = %agent.command, error = %e, "list-models probe failed to spawn");
      return Vec::new();
    }
    Err(_) => {
      warn!(command = %agent.command, "list-models probe timed out");
      return Vec::new();
    }
  };

  if !output.status.success() {
    let stderr = String::from_utf8_lossy(&output.stderr);
    debug!(
      command = %agent.command,
      status = ?output.status.code(),
      stderr = %stderr.trim(),
      "list-models probe exited non-zero"
    );
    return Vec::new();
  }

  parse_model_lines(&String::from_utf8_lossy(&output.stdout))
}

/// Parse model ids from free-form CLI output (one id per line, or JSON-ish lists).
fn parse_model_lines(stdout: &str) -> Vec<String> {
  let mut ids = BTreeSet::new();

  // Try JSON array of strings or { "data": [ { "id": ... } ] }
  let trimmed = stdout.trim();
  if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
    collect_ids_from_json(&value, &mut ids);
    if !ids.is_empty() {
      return ids.into_iter().collect();
    }
  }

  for line in stdout.lines() {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
      continue;
    }
    // Skip obvious non-ids
    if line.contains(' ') && !line.contains('/') {
      // e.g. "Available models:" headers
      if line.ends_with(':') {
        continue;
      }
      // Take first token if it looks like an id
      if let Some(first) = line.split_whitespace().next() {
        if looks_like_model_id(first) {
          ids.insert(first.to_string());
        }
      }
      continue;
    }
    if looks_like_model_id(line) {
      ids.insert(line.to_string());
    }
  }

  ids.into_iter().collect()
}

fn collect_ids_from_json(value: &serde_json::Value, ids: &mut BTreeSet<String>) {
  match value {
    serde_json::Value::Array(items) => {
      for item in items {
        match item {
          serde_json::Value::String(s) if looks_like_model_id(s) => {
            ids.insert(s.clone());
          }
          serde_json::Value::Object(map) => {
            if let Some(serde_json::Value::String(s)) = map.get("id") {
              if looks_like_model_id(s) {
                ids.insert(s.clone());
              }
            }
          }
          _ => collect_ids_from_json(item, ids),
        }
      }
    }
    serde_json::Value::Object(map) => {
      if let Some(data) = map.get("data") {
        collect_ids_from_json(data, ids);
      }
      if let Some(models) = map.get("models") {
        collect_ids_from_json(models, ids);
      }
      if let Some(serde_json::Value::String(s)) = map.get("id") {
        if looks_like_model_id(s) {
          ids.insert(s.clone());
        }
      }
    }
    serde_json::Value::String(s) if looks_like_model_id(s) => {
      ids.insert(s.clone());
    }
    _ => {}
  }
}

fn looks_like_model_id(s: &str) -> bool {
  let s = s.trim().trim_matches(|c| c == '"' || c == '\'' || c == ',');
  if s.is_empty() || s.len() > 128 {
    return false;
  }
  // Must look like gpt-4o, claude-..., codex, o3, etc.
  s.chars()
    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' || c == ':' || c == '/')
    && s.chars().any(|c| c.is_ascii_alphanumeric())
}
