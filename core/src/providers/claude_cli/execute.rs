//! Claude CLI execute via `claude -p` stream-json stdio.

use tokio::sync::mpsc;

use crate::error::{AppError, AppResult};
use crate::models::openai::Usage;
use crate::providers::execute::{
  ExecRequest, ExecResponse, ExecStream, ExecStreamEvent, exec_stream_channel,
};
use crate::providers::flatten_messages;

use super::ClaudeCliProvider;
use super::stdio::{StreamEvent, StreamJsonSession};

impl ClaudeCliProvider {
  pub async fn execute(&self, req: &ExecRequest) -> AppResult<ExecResponse> {
    let (system, _) = prompt_from_request(req);
    let mut guard = self.lock_session(&req.model, system.as_deref()).await?;
    let session = Self::session_mut(&mut guard)?;
    let collected = run_prompt(session, req, None).await?;
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
      let (system, _) = prompt_from_request(&req);
      let mut guard = match this.lock_session(&req.model, system.as_deref()).await {
        Ok(g) => g,
        Err(e) => {
          let _ = tx.send(Err(e)).await;
          return;
        }
      };
      let session = match ClaudeCliProvider::session_mut(&mut guard) {
        Ok(s) => s,
        Err(e) => {
          let _ = tx.send(Err(e)).await;
          return;
        }
      };

      match run_prompt(session, &req, Some(&tx)).await {
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

async fn run_prompt(
  session: &mut StreamJsonSession,
  req: &ExecRequest,
  stream: Option<&mpsc::Sender<AppResult<ExecStreamEvent>>>,
) -> AppResult<CollectedTurn> {
  let (_system, prompt) = prompt_from_request(req);
  if prompt.trim().is_empty() {
    return Err(AppError::BadRequest(
      "claude-cli: empty prompt after flattening messages".into(),
    ));
  }

  session.send_user(&prompt).await?;

  let mut content = String::new();
  let mut streamed_chars = 0usize;
  let mut prompt_tokens = 0u32;
  let mut completion_tokens = 0u32;

  loop {
    let Some(event) = session.recv_event().await else {
      return Err(AppError::ProviderFailed(
        "claude-cli: process closed before stream-json result (run `claude auth login` if needed)"
          .into(),
      ));
    };
    match event {
      StreamEvent::TextDelta(delta) => {
        content.push_str(&delta);
        streamed_chars += delta.len();
        if let Some(tx) = stream {
          if tx
            .send(Ok(ExecStreamEvent::ContentDelta { text: delta }))
            .await
            .is_err()
          {
            session.mark_turn_complete();
            return Ok(CollectedTurn {
              content,
              streamed_chars,
              prompt_tokens,
              completion_tokens,
            });
          }
        }
      }
      StreamEvent::ThinkingDelta(delta) => {
        if let Some(tx) = stream {
          let _ = tx
            .send(Ok(ExecStreamEvent::ReasoningDelta { text: delta }))
            .await;
        }
      }
      StreamEvent::AssistantText(text) => {
        if text.len() > content.len() {
          content = text;
        }
      }
      StreamEvent::Result {
        content: result_text,
        is_error,
        error,
        prompt_tokens: p,
        completion_tokens: c,
      } => {
        prompt_tokens = p;
        completion_tokens = c;
        if is_error {
          session.mark_turn_complete();
          return Err(AppError::ProviderFailed(error.unwrap_or_else(|| {
            "claude-cli: Claude Code returned an error (run `claude auth login` if needed)".into()
          })));
        }
        if content.is_empty() && !result_text.is_empty() {
          content = result_text;
        }
        break;
      }
      StreamEvent::Init { .. } | StreamEvent::ControlRequest { .. } | StreamEvent::Ignored => {}
    }
  }

  session.mark_turn_complete();
  Ok(CollectedTurn {
    content,
    streamed_chars,
    prompt_tokens,
    completion_tokens,
  })
}

fn prompt_from_request(req: &ExecRequest) -> (Option<String>, String) {
  let (system, turns) = req.system_and_turns();
  if turns.is_empty() {
    return (None, system.unwrap_or_default());
  }
  flatten_messages(system.as_deref(), &turns)
}
