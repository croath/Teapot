//! Grok CLI execute via `grok agent stdio` ACP JSON-RPC.

use tokio::sync::mpsc;

use crate::error::{AppError, AppResult};
use crate::models::openai::Usage;
use crate::providers::execute::{
  ExecRequest, ExecResponse, ExecStream, ExecStreamEvent, exec_stream_channel,
};
use crate::providers::flatten_messages;

use super::GrokCliProvider;
use super::stdio::{
  AcpSession, StreamEvent, is_session_update, notification_session_id, parse_session_update,
  result_stop_reason, result_text, session_id_from, usage_tokens,
};

impl GrokCliProvider {
  pub async fn execute(&self, req: &ExecRequest) -> AppResult<ExecResponse> {
    let mut guard = self.lock_session().await?;
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
      let mut guard = match this.lock_session().await {
        Ok(g) => g,
        Err(e) => {
          let _ = tx.send(Err(e)).await;
          return;
        }
      };
      let session = match GrokCliProvider::session_mut(&mut guard) {
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
  session: &mut AcpSession,
  req: &ExecRequest,
  stream: Option<&mpsc::Sender<AppResult<ExecStreamEvent>>>,
) -> AppResult<CollectedTurn> {
  let (system, prompt) = prompt_from_request(req);
  if prompt.trim().is_empty() {
    return Err(AppError::BadRequest(
      "grok-cli: empty prompt after flattening messages".into(),
    ));
  }

  while session.try_recv_notification().is_some() {}

  let created = session
    .new_session(system.as_deref(), Some(req.model.as_str()))
    .await?;
  let session_id = session_id_from(&created)
    .ok_or_else(|| AppError::ProviderFailed("grok-cli: session/new missing sessionId".into()))?;
  session
    .apply_model(&session_id, &req.model, &created)
    .await?;
  let waiter = session.start_prompt(&session_id, &prompt).await?;
  let wait_fut = waiter.wait();
  tokio::pin!(wait_fut);

  let mut content = String::new();
  let mut streamed_chars = 0usize;
  let mut prompt_tokens = 0u32;
  let mut completion_tokens = 0u32;

  let outcome: AppResult<CollectedTurn> = loop {
    tokio::select! {
      note = session.recv_notification() => {
        let Some(note) = note else {
          break Err(AppError::ProviderFailed(
            "grok-cli: agent closed before session/prompt result (run `grok login` if needed)"
              .into(),
          ));
        };
        if let Some(sid) = notification_session_id(&note) {
          if sid != session_id {
            continue;
          }
        }
        if !is_session_update(&note.method) {
          continue;
        }
        match parse_session_update(&note.params) {
          StreamEvent::TextDelta(delta) => {
            content.push_str(&delta);
            streamed_chars += delta.len();
            if let Some(tx) = stream {
              if tx
                .send(Ok(ExecStreamEvent::ContentDelta { text: delta }))
                .await
                .is_err()
              {
                break Ok(CollectedTurn {
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
          StreamEvent::Usage {
            prompt_tokens: p,
            completion_tokens: c,
          } => {
            if p > 0 {
              prompt_tokens = p;
            }
            if c > 0 {
              completion_tokens = c;
            }
          }
          StreamEvent::Ignored => {}
        }
      }
      result = &mut wait_fut => {
        let result = match result {
          Ok(v) => v,
          Err(e) => break Err(e),
        };
        let (p, c) = usage_tokens(&result);
        if p > 0 {
          prompt_tokens = p;
        }
        if c > 0 {
          completion_tokens = c;
        }
        if let Some(text) = result_text(&result) {
          if content.is_empty() {
            content = text;
          }
        }
        break match result_stop_reason(&result) {
          Some("refusal") => Err(AppError::ProviderFailed(
            "grok-cli: Grok refused to continue".into(),
          )),
          Some("cancelled") if content.is_empty() => Err(AppError::ProviderFailed(
            "grok-cli: turn cancelled before any assistant text".into(),
          )),
          _ => Ok(CollectedTurn {
            content,
            streamed_chars,
            prompt_tokens,
            completion_tokens,
          }),
        };
      }
    }
  };

  let _ = session.close_session(&session_id).await;
  outcome
}

fn prompt_from_request(req: &ExecRequest) -> (Option<String>, String) {
  let (system, turns) = req.system_and_turns();
  if turns.is_empty() {
    return (None, system.unwrap_or_default());
  }
  flatten_messages(system.as_deref(), &turns)
}
