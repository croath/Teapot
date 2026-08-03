//! Agent CLI backends.
//!
//! Each agent is an external CLI process (`codex`, `claude`, `grok`, `antigravity-cli`)
//! that receives a prompt and streams text output on stdout.

pub mod discovery;
mod runner;

pub use discovery::{
  AgentInfo, DiscoveredModel, ModelSource, discover_models, is_agent_installed, list_agent_infos,
};
pub use runner::{AgentEvent, AgentRunner, AgentSession, flatten_messages};
