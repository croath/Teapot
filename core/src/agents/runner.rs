//! Spawns agent CLI processes and streams stdout as token events.

use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use crate::config::{AgentConfig, Config};
use crate::error::{AppError, AppResult};

/// Events emitted while an agent CLI is running.
#[derive(Debug, Clone)]
pub enum AgentEvent {
  /// A chunk of stdout text (treated as model tokens / content delta).
  Token(String),
  /// A line from stderr (diagnostic; not sent as model content).
  Stderr(String),
  /// Process finished successfully.
  Done { exit_code: i32 },
  /// Process failed or timed out.
  Failed(String),
}

/// Shared agent runner backed by workspace configuration.
#[derive(Clone)]
pub struct AgentRunner {
  config: Arc<Config>,
}

impl AgentRunner {
  pub fn new(config: Arc<Config>) -> Self {
    Self { config }
  }

  pub fn config(&self) -> &Config {
    &self.config
  }

  /// Resolve agent config for a model id and ensure the binary exists on PATH.
  pub fn resolve(&self, model: &str) -> AppResult<(String, AgentConfig)> {
    let agent_name = self.config.resolve_agent(model).to_string();
    let agent = self
      .config
      .agent(&agent_name)
      .cloned()
      .ok_or_else(|| AppError::AgentNotFound(agent_name.clone()))?;

    if which::which(&agent.command).is_err() {
      return Err(AppError::AgentBinaryMissing(format!(
        "{} (command: {})",
        agent_name, agent.command
      )));
    }

    Ok((agent_name, agent))
  }

  /// Run an agent with the given prompts and stream events on a channel.
  pub async fn run(
    &self,
    model: &str,
    system: Option<&str>,
    prompt: &str,
  ) -> AppResult<AgentSession> {
    let (agent_name, agent) = self.resolve(model)?;
    info!(agent = %agent_name, command = %agent.command, "starting agent CLI");

    let system = system.unwrap_or("");
    let args: Vec<String> = agent
      .args
      .iter()
      .map(|a| a.replace("{prompt}", prompt).replace("{system}", system))
      .collect();

    let mut cmd = Command::new(&agent.command);
    cmd.args(&args)
      .stdout(Stdio::piped())
      .stderr(Stdio::piped())
      .kill_on_drop(true);

    if agent.prompt_via_stdin {
      cmd.stdin(Stdio::piped());
    } else {
      cmd.stdin(Stdio::null());
    }

    if let Some(cwd) = &agent.cwd {
      cmd.current_dir(cwd);
    }
    for (k, v) in &agent.env {
      cmd.env(k, v);
    }

    debug!(?args, "agent argv");

    let mut child = cmd.spawn().map_err(|e| {
      AppError::AgentFailed(format!("failed to spawn {}: {e}", agent.command))
    })?;

    if agent.prompt_via_stdin {
      if let Some(mut stdin) = child.stdin.take() {
        let mut payload = String::new();
        if !system.is_empty() {
          payload.push_str(system);
          payload.push_str("\n\n");
        }
        payload.push_str(prompt);
        if let Err(e) = stdin.write_all(payload.as_bytes()).await {
          warn!(error = %e, "failed to write prompt to agent stdin");
        }
        let _ = stdin.shutdown().await;
      }
    }

    let stdout = child
      .stdout
      .take()
      .ok_or_else(|| AppError::Internal("agent stdout missing".into()))?;
    let stderr = child
      .stderr
      .take()
      .ok_or_else(|| AppError::Internal("agent stderr missing".into()))?;

    let (tx, rx) = mpsc::channel::<AgentEvent>(256);
    let timeout = if agent.timeout_secs == 0 {
      None
    } else {
      Some(Duration::from_secs(agent.timeout_secs))
    };

    tokio::spawn(async move {
      let tx_out = tx.clone();
      let out_task = tokio::spawn(async move {
        let mut reader = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = reader.next_line().await {
          // Stream line-by-line; append newline so formatting is preserved.
          let chunk = format!("{line}\n");
          if tx_out.send(AgentEvent::Token(chunk)).await.is_err() {
            break;
          }
        }
      });

      let tx_err = tx.clone();
      let err_task = tokio::spawn(async move {
        let mut reader = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = reader.next_line().await {
          debug!(stderr = %line, "agent stderr");
          if tx_err.send(AgentEvent::Stderr(line)).await.is_err() {
            break;
          }
        }
      });

      let wait_fut = async {
        let _ = out_task.await;
        let _ = err_task.await;
        child.wait().await
      };

      let status = if let Some(dur) = timeout {
        match tokio::time::timeout(dur, wait_fut).await {
          Ok(Ok(status)) => status,
          Ok(Err(e)) => {
            let _ = tx
              .send(AgentEvent::Failed(format!("wait failed: {e}")))
              .await;
            return;
          }
          Err(_) => {
            error!("agent timed out after {dur:?}");
            let _ = child.kill().await;
            let _ = tx
              .send(AgentEvent::Failed(format!(
                "agent timed out after {}s",
                dur.as_secs()
              )))
              .await;
            return;
          }
        }
      } else {
        match wait_fut.await {
          Ok(status) => status,
          Err(e) => {
            let _ = tx
              .send(AgentEvent::Failed(format!("wait failed: {e}")))
              .await;
            return;
          }
        }
      };

      let code = status.code().unwrap_or(-1);
      if status.success() {
        let _ = tx.send(AgentEvent::Done { exit_code: code }).await;
      } else {
        let _ = tx
          .send(AgentEvent::Failed(format!(
            "agent exited with status {code}"
          )))
          .await;
      }
    });

    Ok(AgentSession {
      agent_name,
      receiver: rx,
    })
  }

  /// Collect the full agent response (non-streaming helper).
  pub async fn run_collect(
    &self,
    model: &str,
    system: Option<&str>,
    prompt: &str,
  ) -> AppResult<String> {
    let mut session = self.run(model, system, prompt).await?;
    let mut out = String::new();
    while let Some(event) = session.recv().await {
      match event {
        AgentEvent::Token(t) => out.push_str(&t),
        AgentEvent::Stderr(_) => {}
        AgentEvent::Done { .. } => break,
        AgentEvent::Failed(msg) => return Err(AppError::AgentFailed(msg)),
      }
    }
    Ok(out)
  }
}

/// Live session streaming agent events.
pub struct AgentSession {
  pub agent_name: String,
  receiver: mpsc::Receiver<AgentEvent>,
}

impl AgentSession {
  pub async fn recv(&mut self) -> Option<AgentEvent> {
    self.receiver.recv().await
  }

  pub fn agent_name(&self) -> &str {
    &self.agent_name
  }
}

/// Build a single prompt string from chat-style messages.
pub fn flatten_messages(
  system: Option<&str>,
  messages: &[(String, String)],
) -> (Option<String>, String) {
  let mut system_parts: Vec<String> = Vec::new();
  if let Some(s) = system {
    if !s.is_empty() {
      system_parts.push(s.to_string());
    }
  }

  let mut body = String::new();
  for (role, content) in messages {
    match role.as_str() {
      "system" => system_parts.push(content.clone()),
      "user" | "human" => {
        if !body.is_empty() {
          body.push_str("\n\n");
        }
        body.push_str("User: ");
        body.push_str(content);
      }
      "assistant" | "model" => {
        if !body.is_empty() {
          body.push_str("\n\n");
        }
        body.push_str("Assistant: ");
        body.push_str(content);
      }
      other => {
        if !body.is_empty() {
          body.push_str("\n\n");
        }
        body.push_str(other);
        body.push_str(": ");
        body.push_str(content);
      }
    }
  }

  // Encourage the agent to continue as assistant
  if !body.is_empty() && !body.ends_with("Assistant: ") {
    body.push_str("\n\nAssistant:");
  }

  let system = if system_parts.is_empty() {
    None
  } else {
    Some(system_parts.join("\n\n"))
  };
  (system, body)
}
