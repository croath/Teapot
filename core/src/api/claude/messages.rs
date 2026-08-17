//! Anthropic Messages: POST /claude/v1/messages
//!
//! Supports non-stream JSON and true SSE streaming (`stream: true`) via each
//! provider's `execute_stream`. Event names and payloads follow the official
//! Messages stream (`message_start`, `content_block_*`, `message_delta`,
//! `message_stop`).

use axum::extract::State;
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use tokio_stream::StreamExt;

use super::json::ClaudeJson;
use crate::api::state::AppState;
use crate::error::{AnthropicErrorBody, AppError, ClaudeResult};
use crate::models::anthropic::{
  ContentBlock, ContentBlockDeltaEvent, ContentBlockStartEvent, ContentBlockStopEvent,
  MAX_MESSAGES, MessageDeltaEvent, MessageStartEvent, MessageStopEvent, MessagesRequest,
  MessagesResponse, anthropic_message_id, stop_reason_from_finish,
};
use crate::models::openai::Usage;
use crate::providers::execute::{ExecRequest, ExecStreamEvent};
use crate::stream::{SseItem, anthropic_event, send_sse, sse_channel, sse_response};

pub async fn create_message(
  State(state): State<AppState>,
  ClaudeJson(req): ClaudeJson<MessagesRequest>,
) -> ClaudeResult<Response> {
  validate_create_message(&req)?;

  tracing::info!(
    model = %req.model,
    stream = req.stream,
    messages = req.messages.len(),
    max_tokens = req.max_tokens,
    "messages.create"
  );

  // No automatic provider/model adaptation: model must be in the pinned catalog.
  state.runtime.models().require(&req.model).await?;

  // Keep provider's own session fresh before use (does not run inside execute).
  state.runtime.refresh_access_token_if_needed().await?;

  let exec_req = ExecRequest::from_messages(&req);

  if req.stream {
    stream_messages(state, exec_req).await
  } else {
    let result = state.provider.execute(&exec_req).await?;
    tracing::info!(
      id = %result.id,
      model = %result.model,
      prompt_tokens = result.usage.prompt_tokens,
      completion_tokens = result.usage.completion_tokens,
      "messages.create done"
    );
    let body = MessagesResponse::completed(
      anthropic_message_id(&result.id),
      result.model,
      result.content,
      &result.usage,
      stop_reason_from_finish(Some("stop")),
    );
    Ok(ClaudeJson(body).into_response())
  }
}

fn validate_create_message(req: &MessagesRequest) -> Result<(), AppError> {
  if req.messages.is_empty() {
    return Err(AppError::BadRequest("messages must not be empty".into()));
  }
  if req.messages.len() > MAX_MESSAGES {
    return Err(AppError::BadRequest(format!(
      "messages must not exceed {MAX_MESSAGES} items"
    )));
  }
  Ok(())
}

async fn stream_messages(state: AppState, exec_req: ExecRequest) -> ClaudeResult<Response> {
  let mut upstream = state.provider.execute_stream(&exec_req).await?;

  let mut stream_id = format!("msg_{}", uuid::Uuid::new_v4().simple());
  let mut model = exec_req.model.clone();
  let (tx, rx) = sse_channel(64);

  tokio::spawn(async move {
    let mut started = false;
    let mut thinking_open: Option<u32> = None;
    let mut text_open: Option<u32> = None;
    let mut next_index: u32 = 0;

    while let Some(item) = upstream.next().await {
      match item {
        Ok(ExecStreamEvent::Meta { model: m, id: eid }) => {
          if let Some(m) = m.filter(|s| !s.is_empty()) {
            model = m;
          }
          if let Some(eid) = eid.filter(|s| !s.is_empty()) {
            stream_id = anthropic_message_id(&eid);
          }
          if !ensure_started(&tx, &mut started, &stream_id, &model).await {
            return;
          }
        }
        Ok(ExecStreamEvent::ReasoningDelta { text }) => {
          if text.is_empty() {
            continue;
          }
          if !ensure_started(&tx, &mut started, &stream_id, &model).await {
            return;
          }
          if !close_block(&tx, text_open.take()).await {
            return;
          }
          if thinking_open.is_none() {
            let idx = next_index;
            next_index = next_index.saturating_add(1);
            thinking_open = Some(idx);
            if !emit(
              &tx,
              "content_block_start",
              &ContentBlockStartEvent::new(idx, ContentBlock::empty_thinking()),
            )
            .await
            {
              return;
            }
          }
          let idx = thinking_open.unwrap();
          if !emit(
            &tx,
            "content_block_delta",
            &ContentBlockDeltaEvent::thinking(idx, text),
          )
          .await
          {
            return;
          }
        }
        Ok(ExecStreamEvent::ContentDelta { text }) => {
          if text.is_empty() {
            continue;
          }
          if !ensure_started(&tx, &mut started, &stream_id, &model).await {
            return;
          }
          if !close_block(&tx, thinking_open.take()).await {
            return;
          }
          if text_open.is_none() {
            let idx = next_index;
            next_index = next_index.saturating_add(1);
            text_open = Some(idx);
            if !emit(
              &tx,
              "content_block_start",
              &ContentBlockStartEvent::new(idx, ContentBlock::empty_text()),
            )
            .await
            {
              return;
            }
          }
          let idx = text_open.unwrap();
          if !emit(
            &tx,
            "content_block_delta",
            &ContentBlockDeltaEvent::text(idx, text),
          )
          .await
          {
            return;
          }
        }
        Ok(ExecStreamEvent::Done {
          finish_reason,
          usage,
        }) => {
          if !ensure_started(&tx, &mut started, &stream_id, &model).await {
            return;
          }
          finish_stream(
            &tx,
            thinking_open.take(),
            text_open.take(),
            finish_reason,
            usage.as_ref(),
          )
          .await;
          return;
        }
        Err(e) => {
          tracing::warn!(error = %e, "messages stream error");
          let body = AnthropicErrorBody::from_app_error(&e);
          let _ = emit(&tx, "error", &body).await;
          return;
        }
      }
    }

    // Upstream closed without Done.
    if ensure_started(&tx, &mut started, &stream_id, &model).await {
      finish_stream(
        &tx,
        thinking_open.take(),
        text_open.take(),
        Some("stop"),
        None,
      )
      .await;
    }
  });

  Ok(sse_response(rx).into_response())
}

async fn ensure_started(
  tx: &tokio::sync::mpsc::Sender<SseItem>,
  started: &mut bool,
  id: &str,
  model: &str,
) -> bool {
  if *started {
    return true;
  }
  *started = true;
  emit(
    tx,
    "message_start",
    &MessageStartEvent::new(MessagesResponse::stream_start(id, model)),
  )
  .await
}

async fn finish_stream(
  tx: &tokio::sync::mpsc::Sender<SseItem>,
  thinking_open: Option<u32>,
  text_open: Option<u32>,
  finish_reason: Option<&str>,
  usage: Option<&Usage>,
) {
  if !close_block(tx, thinking_open).await {
    return;
  }
  if !close_block(tx, text_open).await {
    return;
  }
  let output_tokens = usage.map(|u| u.completion_tokens).unwrap_or(0);
  let stop = stop_reason_from_finish(finish_reason);
  if !emit(
    tx,
    "message_delta",
    &MessageDeltaEvent::new(stop, output_tokens),
  )
  .await
  {
    return;
  }
  let _ = emit(tx, "message_stop", &MessageStopEvent::new()).await;
}

async fn close_block(tx: &tokio::sync::mpsc::Sender<SseItem>, index: Option<u32>) -> bool {
  match index {
    Some(idx) => emit(tx, "content_block_stop", &ContentBlockStopEvent::new(idx)).await,
    None => true,
  }
}

async fn emit<T: Serialize>(
  tx: &tokio::sync::mpsc::Sender<SseItem>,
  event_type: &str,
  body: &T,
) -> bool {
  match serde_json::to_string(body) {
    Ok(json) => {
      send_sse(tx, anthropic_event(event_type, &json)).await;
      true
    }
    Err(err) => {
      tracing::warn!(error = %err, event_type, "failed to serialize messages SSE event");
      false
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn rejects_empty_messages() {
    let req = MessagesRequest {
      model: "claude-sonnet-4-6".into(),
      messages: vec![],
      max_tokens: 1024,
      system: None,
      stream: false,
      temperature: None,
      top_p: None,
      top_k: None,
      stop_sequences: None,
      metadata: None,
      thinking: None,
      tool_choice: None,
      tools: None,
      output_config: None,
      service_tier: None,
      cache_control: None,
      container: None,
      inference_geo: None,
      agent: None,
    };
    let err = validate_create_message(&req).unwrap_err();
    assert!(err.to_string().contains("must not be empty"));
  }

  #[test]
  fn accepts_non_empty_messages() {
    let req: MessagesRequest = serde_json::from_str(
      r#"{
        "model": "claude-sonnet-4-6",
        "max_tokens": 1024,
        "messages": [{ "role": "user", "content": "Hello" }]
      }"#,
    )
    .unwrap();
    validate_create_message(&req).unwrap();
  }
}
