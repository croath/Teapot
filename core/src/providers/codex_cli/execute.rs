//! Codex CLI execute via app-server thread/turn JSON-RPC.

use serde_json::json;
use tokio::sync::mpsc;

use crate::error::{AppError, AppResult};
use crate::models::openai::Usage;
use crate::providers::execute::{
  ExecRequest, ExecResponse, ExecStream, ExecStreamEvent, exec_stream_channel,
};
use crate::providers::flatten_messages;

use super::CodexCliProvider;
use super::rpc::{
  AppServerSession, extract_agent_message_text, extract_text_delta, notification_thread_id,
  token_usage, turn_error_message, turn_status,
};

impl CodexCliProvider {
  pub async fn execute(&self, req: &ExecRequest) -> AppResult<ExecResponse> {
    let mut guard = self.lock_session().await?;
    let session = Self::session_mut(&mut guard)?;
    let collected = run_turn(session, req, None).await?;
    Ok(
      ExecResponse::new(req.model.clone(), collected.content)
        .with_usage(collected.prompt_tokens, collected.completion_tokens),
    )
  }

  pub async fn execute_stream(&self, req: &ExecRequest) -> AppResult<ExecStream> {
    let model = req.model.clone();
    let req = req.clone();
    let this = self.clone();
    let (tx, rx) = exec_stream_channel(64);
    tokio::spawn(async move {
      let mut guard = match this.lock_session().await {
        Ok(g) => g,
        Err(e) => {
          let _ = tx.send(Err(e)).await;
          return;
        }
      };
      let session = match CodexCliProvider::session_mut(&mut guard) {
        Ok(s) => s,
        Err(e) => {
          let _ = tx.send(Err(e)).await;
          return;
        }
      };

      match run_turn(session, &req, Some(&tx)).await {
        Ok(collected) => {
          let _ = tx
            .send(Ok(ExecStreamEvent::Meta {
              model: Some(model),
              id: None,
            }))
            .await;
          if collected.streamed_chars == 0 && !collected.content.is_empty() {
            let _ = tx
              .send(Ok(ExecStreamEvent::ContentDelta {
                text: collected.content,
              }))
              .await;
          }
          let _ = tx
            .send(Ok(ExecStreamEvent::Done {
              finish_reason: Some("stop"),
              usage: Some(Usage {
                prompt_tokens: collected.prompt_tokens,
                completion_tokens: collected.completion_tokens,
                total_tokens: collected
                  .prompt_tokens
                  .saturating_add(collected.completion_tokens),
              }),
            }))
            .await;
        }
        Err(e) => {
          let _ = tx.send(Err(e)).await;
        }
      }
    });
    Ok(rx)
  }
}

struct CollectedTurn {
  content: String,
  streamed_chars: usize,
  prompt_tokens: u32,
  completion_tokens: u32,
}

async fn run_turn(
  session: &mut AppServerSession,
  req: &ExecRequest,
  stream: Option<&mpsc::Sender<AppResult<ExecStreamEvent>>>,
) -> AppResult<CollectedTurn> {
  let prompt = prompt_from_request(req);
  if prompt.trim().is_empty() {
    return Err(AppError::BadRequest(
      "codex-cli: empty prompt after flattening messages".into(),
    ));
  }

  let cwd = std::env::current_dir().ok();
  let mut start_params = json!({
    "model": req.model,
    "ephemeral": true,
    "approvalPolicy": "never",
    "sandbox": "danger-full-access",
  });
  if let Some(dir) = cwd.as_ref().and_then(|p| p.to_str()) {
    start_params["cwd"] = json!(dir);
  }

  let thread = session.request("thread/start", start_params).await?;
  let thread_id = thread
    .get("thread")
    .and_then(|t| t.get("id"))
    .and_then(|v| v.as_str())
    .ok_or_else(|| AppError::ProviderFailed("codex-cli: thread/start missing thread.id".into()))?
    .to_string();

  let _turn = session
    .request(
      "turn/start",
      json!({
        "threadId": thread_id,
        "input": [{ "type": "text", "text": prompt }],
        "approvalPolicy": "never",
        "model": req.model,
      }),
    )
    .await?;

  let mut content = String::new();
  let mut streamed_chars = 0usize;
  let mut prompt_tokens = 0u32;
  let mut completion_tokens = 0u32;

  loop {
    let Some(note) = session.recv_notification().await else {
      return Err(AppError::ProviderFailed(
        "codex-cli: app-server closed before turn/completed".into(),
      ));
    };
    if let Some(tid) = notification_thread_id(&note) {
      if tid != thread_id {
        continue;
      }
    }

    match note.method.as_str() {
      "item/agentMessage/delta" => {
        if let Some(delta) = extract_text_delta(&note.params) {
          content.push_str(&delta);
          streamed_chars += delta.len();
          if let Some(tx) = stream {
            if tx
              .send(Ok(ExecStreamEvent::ContentDelta { text: delta }))
              .await
              .is_err()
            {
              return Ok(CollectedTurn {
                content,
                streamed_chars,
                prompt_tokens,
                completion_tokens,
              });
            }
          }
        }
      }
      "item/completed" | "item/started" => {
        if let Some(text) = extract_agent_message_text(&note.params) {
          if text.len() > content.len() {
            content = text;
          }
        }
      }
      "thread/tokenUsage/updated" => {
        if let Some((p, c)) = token_usage(&note.params) {
          prompt_tokens = p;
          completion_tokens = c;
        }
      }
      "turn/completed" => {
        if let Some((p, c)) = token_usage(&note.params) {
          prompt_tokens = p;
          completion_tokens = c;
        }
        let status = turn_status(&note.params).unwrap_or("completed");
        if status == "failed" {
          let msg =
            turn_error_message(&note.params).unwrap_or_else(|| "codex-cli: turn failed".into());
          return Err(AppError::ProviderFailed(msg));
        }
        if status == "interrupted" && content.is_empty() {
          return Err(AppError::ProviderFailed(
            "codex-cli: turn interrupted before any assistant text".into(),
          ));
        }
        break;
      }
      _ => {}
    }
  }

  let _ = session
    .request("thread/unsubscribe", json!({ "threadId": thread_id }))
    .await;

  Ok(CollectedTurn {
    content,
    streamed_chars,
    prompt_tokens,
    completion_tokens,
  })
}

fn prompt_from_request(req: &ExecRequest) -> String {
  let (system, turns) = req.system_and_turns();
  if turns.is_empty() {
    return system.unwrap_or_default();
  }
  let (_sys, body) = flatten_messages(system.as_deref(), &turns);
  body
}
