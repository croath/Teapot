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
  AcpSession, StreamEvent, is_session_update, parse_session_update, result_stop_reason,
  result_text, usage_tokens,
};

impl GrokCliProvider {
  pub async fn execute(&self, req: &ExecRequest) -> AppResult<ExecResponse> {
    let collected = run_prompt(req, None).await?;
    Ok(
      ExecResponse::new(req.model.clone(), collected.content)
        .with_usage(collected.prompt_tokens, collected.completion_tokens),
    )
  }

  pub async fn execute_stream(&self, req: &ExecRequest) -> AppResult<ExecStream> {
    let model = req.model.clone();
    let req = req.clone();
    let (tx, rx) = exec_stream_channel(64);
    tokio::spawn(async move {
      match run_prompt(&req, Some(&tx)).await {
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
  req: &ExecRequest,
  stream: Option<&mpsc::Sender<AppResult<ExecStreamEvent>>>,
) -> AppResult<CollectedTurn> {
  let (system, prompt) = prompt_from_request(req);
  if prompt.trim().is_empty() {
    return Err(AppError::BadRequest(
      "grok-cli: empty prompt after flattening messages".into(),
    ));
  }

  let mut session = AcpSession::spawn(&req.model).await?;
  session.handshake().await?;
  let session_id = session.new_session(system.as_deref()).await?;
  let waiter = session.start_prompt(&session_id, &prompt).await?;
  let wait_fut = waiter.wait();
  tokio::pin!(wait_fut);

  let mut content = String::new();
  let mut streamed_chars = 0usize;
  let mut prompt_tokens = 0u32;
  let mut completion_tokens = 0u32;

  loop {
    tokio::select! {
      note = session.recv_notification() => {
        let Some(note) = note else {
          return Err(AppError::ProviderFailed(
            "grok-cli: agent closed before session/prompt result (run `grok login` if needed)"
              .into(),
          ));
        };
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
        let result = result?;
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
        match result_stop_reason(&result) {
          Some("refusal") => {
            return Err(AppError::ProviderFailed(
              "grok-cli: Grok refused to continue".into(),
            ));
          }
          Some("cancelled") if content.is_empty() => {
            return Err(AppError::ProviderFailed(
              "grok-cli: turn cancelled before any assistant text".into(),
            ));
          }
          _ => {}
        }
        break;
      }
    }
  }

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
