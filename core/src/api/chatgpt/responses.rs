//! OpenAI Responses API under `/chatgpt/v1/responses`.
//!
//! - `POST /responses`         — create (stream + non-stream)
//! - `POST /responses/compact` — compact conversation (JSON or stream)

use axum::extract::State;
use axum::response::{IntoResponse, Response};
use chrono::Utc;
use tokio_stream::StreamExt;
use uuid::Uuid;

use super::json::OpenAiJson;
use crate::api::state::AppState;
use crate::error::{AppError, OpenAiResult};
use crate::models::openai::*;
use crate::providers::compact::ExecCompactRequest;
use crate::providers::execute::{ExecRequest, ExecStreamEvent};
use crate::stream::{openai_named_event, send_sse, sse_channel, sse_response};

// ---------------------------------------------------------------------------
// POST /responses
// ---------------------------------------------------------------------------

pub async fn create_response(
  State(state): State<AppState>,
  OpenAiJson(req): OpenAiJson<ResponsesRequest>,
) -> OpenAiResult<Response> {
  if req.background {
    return Err(
      AppError::BadRequest(
        "background=true is not supported yet; only background=false is available".into(),
      )
      .into(),
    );
  }

  tracing::info!(
    model = %req.model,
    stream = req.stream,
    "responses"
  );

  let exec_req = ExecRequest::from_responses(&req);
  if exec_req.messages.is_empty() {
    return Err(AppError::BadRequest("input must not be empty".into()).into());
  }

  state.runtime.models().require(&req.model).await?;
  state.runtime.refresh_access_token_if_needed().await?;

  let metadata = req.metadata.clone();

  if req.stream {
    stream_response(state, exec_req, metadata).await
  } else {
    complete_response(state, exec_req, metadata).await
  }
}

fn build_completed(
  id: String,
  model: String,
  created: i64,
  text: String,
  usage: ResponseUsage,
  metadata: Option<serde_json::Value>,
) -> ResponsesResponse {
  let msg_id = format!("msg_{}", Uuid::new_v4().simple());
  ResponsesResponse {
    id,
    object: "response".into(),
    created_at: created,
    status: "completed".into(),
    model,
    output: vec![ResponseOutputItem {
      id: msg_id,
      item_type: "message".into(),
      role: "assistant".into(),
      content: vec![ResponseOutputContent {
        content_type: "output_text".into(),
        text,
      }],
      status: "completed".into(),
    }],
    usage,
    error: None,
    metadata,
  }
}

fn usage_from_exec(u: &Usage) -> ResponseUsage {
  ResponseUsage::from(u)
}

fn estimate_usage(prompt_hint: &str, output: &str) -> ResponseUsage {
  let input_tokens = estimate_tokens(prompt_hint);
  let output_tokens = estimate_tokens(output);
  ResponseUsage {
    input_tokens,
    output_tokens,
    total_tokens: input_tokens.saturating_add(output_tokens),
  }
}

fn prompt_hint(req: &ExecRequest) -> String {
  req
    .messages
    .iter()
    .map(|m| m.content.as_text())
    .filter(|t| !t.is_empty())
    .collect::<Vec<_>>()
    .join("\n")
}

async fn complete_response(
  state: AppState,
  exec_req: ExecRequest,
  metadata: Option<serde_json::Value>,
) -> OpenAiResult<Response> {
  let hint = prompt_hint(&exec_req);
  let result = state.provider.execute(&exec_req).await?;
  let id = if result.id.starts_with("resp_") {
    result.id.clone()
  } else {
    format!("resp_{}", Uuid::new_v4().simple())
  };
  let created = Utc::now().timestamp();
  let usage = if result.usage.total_tokens > 0 {
    usage_from_exec(&result.usage)
  } else {
    estimate_usage(&hint, &result.content)
  };
  tracing::info!(
    id = %id,
    model = %result.model,
    input_tokens = usage.input_tokens,
    output_tokens = usage.output_tokens,
    "responses done"
  );
  let body = build_completed(id, result.model, created, result.content, usage, metadata);
  Ok(OpenAiJson(body).into_response())
}

async fn stream_response(
  state: AppState,
  exec_req: ExecRequest,
  metadata: Option<serde_json::Value>,
) -> OpenAiResult<Response> {
  let mut upstream = state.provider.execute_stream(&exec_req).await?;

  let id = format!("resp_{}", Uuid::new_v4().simple());
  let msg_id = format!("msg_{}", Uuid::new_v4().simple());
  let mut model = exec_req.model.clone();
  let created = Utc::now().timestamp();
  let hint = prompt_hint(&exec_req);
  let (tx, rx) = sse_channel(64);

  tokio::spawn(async move {
    let created_ev = serde_json::json!({
      "type": "response.created",
      "response": {
        "id": id,
        "object": "response",
        "created_at": created,
        "status": "in_progress",
        "model": model,
        "output": []
      }
    });
    send_sse(
      &tx,
      openai_named_event("response.created", &created_ev.to_string()),
    )
    .await;

    let in_progress = serde_json::json!({
      "type": "response.in_progress",
      "response": {
        "id": id,
        "object": "response",
        "created_at": created,
        "status": "in_progress",
        "model": model,
        "output": []
      }
    });
    send_sse(
      &tx,
      openai_named_event("response.in_progress", &in_progress.to_string()),
    )
    .await;

    let item_added = serde_json::json!({
      "type": "response.output_item.added",
      "output_index": 0,
      "item": {
        "id": msg_id,
        "type": "message",
        "role": "assistant",
        "status": "in_progress",
        "content": []
      }
    });
    send_sse(
      &tx,
      openai_named_event("response.output_item.added", &item_added.to_string()),
    )
    .await;

    let part_added = serde_json::json!({
      "type": "response.content_part.added",
      "item_id": msg_id,
      "output_index": 0,
      "content_index": 0,
      "part": { "type": "output_text", "text": "" }
    });
    send_sse(
      &tx,
      openai_named_event("response.content_part.added", &part_added.to_string()),
    )
    .await;

    let mut full = String::new();
    let mut final_usage: Option<ResponseUsage> = None;

    while let Some(item) = upstream.next().await {
      match item {
        Ok(ExecStreamEvent::Meta { model: m, id: _ }) => {
          if let Some(m) = m.filter(|s| !s.is_empty()) {
            model = m;
          }
        }
        Ok(ExecStreamEvent::ContentDelta { text }) => {
          if text.is_empty() {
            continue;
          }
          full.push_str(&text);
          let delta = serde_json::json!({
            "type": "response.output_text.delta",
            "item_id": msg_id,
            "output_index": 0,
            "content_index": 0,
            "delta": text
          });
          send_sse(
            &tx,
            openai_named_event("response.output_text.delta", &delta.to_string()),
          )
          .await;
        }
        Ok(ExecStreamEvent::ReasoningDelta { .. }) => {}
        Ok(ExecStreamEvent::Done { usage, .. }) => {
          if let Some(u) = usage {
            final_usage = Some(usage_from_exec(&u));
          }
          finish_response_stream(
            &tx,
            &id,
            &msg_id,
            &model,
            created,
            &full,
            &hint,
            final_usage,
            metadata.clone(),
          )
          .await;
          return;
        }
        Err(e) => {
          tracing::warn!(error = %e, "responses stream error");
          let failed_body = ResponsesResponse {
            id: id.clone(),
            object: "response".into(),
            created_at: created,
            status: "failed".into(),
            model: model.clone(),
            output: vec![],
            usage: ResponseUsage::default(),
            error: Some(ResponseErrorBody {
              message: e.to_string(),
              code: Some("server_error".into()),
            }),
            metadata: metadata.clone(),
          };
          let failed = serde_json::json!({
            "type": "response.failed",
            "response": failed_body
          });
          send_sse(
            &tx,
            openai_named_event("response.failed", &failed.to_string()),
          )
          .await;
          return;
        }
      }
    }

    finish_response_stream(
      &tx,
      &id,
      &msg_id,
      &model,
      created,
      &full,
      &hint,
      final_usage,
      metadata,
    )
    .await;
  });

  Ok(sse_response(rx).into_response())
}

async fn finish_response_stream(
  tx: &tokio::sync::mpsc::Sender<crate::stream::SseItem>,
  id: &str,
  msg_id: &str,
  model: &str,
  created: i64,
  full: &str,
  prompt_hint: &str,
  usage: Option<ResponseUsage>,
  metadata: Option<serde_json::Value>,
) {
  let done_text = serde_json::json!({
    "type": "response.output_text.done",
    "item_id": msg_id,
    "output_index": 0,
    "content_index": 0,
    "text": full
  });
  send_sse(
    tx,
    openai_named_event("response.output_text.done", &done_text.to_string()),
  )
  .await;

  let part_done = serde_json::json!({
    "type": "response.content_part.done",
    "item_id": msg_id,
    "output_index": 0,
    "content_index": 0,
    "part": { "type": "output_text", "text": full }
  });
  send_sse(
    tx,
    openai_named_event("response.content_part.done", &part_done.to_string()),
  )
  .await;

  let item_done = serde_json::json!({
    "type": "response.output_item.done",
    "output_index": 0,
    "item": {
      "id": msg_id,
      "type": "message",
      "role": "assistant",
      "status": "completed",
      "content": [{ "type": "output_text", "text": full }]
    }
  });
  send_sse(
    tx,
    openai_named_event("response.output_item.done", &item_done.to_string()),
  )
  .await;

  let usage = usage.unwrap_or_else(|| estimate_usage(prompt_hint, full));
  let completed = build_completed(
    id.to_string(),
    model.to_string(),
    created,
    full.to_string(),
    usage,
    metadata,
  );

  let completed_ev = serde_json::json!({
    "type": "response.completed",
    "response": completed
  });
  send_sse(
    tx,
    openai_named_event("response.completed", &completed_ev.to_string()),
  )
  .await;
}

// ---------------------------------------------------------------------------
// POST /responses/compact
// ---------------------------------------------------------------------------

pub async fn compact_response(
  State(state): State<AppState>,
  OpenAiJson(req): OpenAiJson<CompactRequest>,
) -> OpenAiResult<Response> {
  tracing::info!(
    model = req.model.as_deref().unwrap_or(""),
    stream = req.stream,
    "responses/compact"
  );

  let mut text = req.input.as_flat_text();
  if let Some(instr) = req.instructions.as_deref() {
    if !instr.is_empty() {
      text = format!("{instr}\n\n{text}");
    }
  }

  if text.trim().is_empty() {
    if req.previous_response_id.is_some() {
      return Err(
        AppError::BadRequest(
          "previous_response_id is not supported without store; pass input".into(),
        )
        .into(),
      );
    }
    return Err(AppError::BadRequest("input must not be empty".into()).into());
  }

  if req.stream {
    stream_compact(state, req, text).await
  } else {
    complete_compact(state, req, text).await
  }
}

async fn complete_compact(
  state: AppState,
  req: CompactRequest,
  text: String,
) -> OpenAiResult<Response> {
  let base_output = req.input.to_input_item_list();
  let (id, summary, usage) = run_compact(&state, &req, &text).await?;
  Ok(OpenAiJson(build_compacted(id, base_output, summary, usage)).into_response())
}

async fn stream_compact(
  state: AppState,
  req: CompactRequest,
  text: String,
) -> OpenAiResult<Response> {
  let base_output = req.input.to_input_item_list();
  let created = Utc::now().timestamp();
  let model = req
    .model
    .clone()
    .filter(|m| !m.is_empty())
    .unwrap_or_else(|| "unknown".into());

  // No model: emit a single-shot synthetic stream of the passthrough text.
  let Some(model_id) = req.model.as_deref().filter(|m| !m.is_empty()) else {
    let tokens = estimate_tokens(&text);
    let usage = ResponseUsage {
      input_tokens: tokens,
      output_tokens: 0,
      total_tokens: tokens,
    };
    let id = format!("resp_{}", Uuid::new_v4().simple());
    let body = build_compacted(id, base_output, text, usage);
    return Ok(synthetic_compact_sse(body, created).into_response());
  };

  state.runtime.models().require(model_id).await?;
  state.runtime.refresh_access_token_if_needed().await?;

  let compact_req = ExecCompactRequest::from_compact(&req, model_id);
  let mut upstream = state.provider.execute_compact_stream(&compact_req).await?;

  let id = format!("resp_{}", Uuid::new_v4().simple());
  let cmp_id = format!("cmp_{}", Uuid::new_v4().simple());
  let (tx, rx) = sse_channel(64);

  tokio::spawn(async move {
    let created_ev = serde_json::json!({
      "type": "response.created",
      "response": {
        "id": id,
        "object": "response.compaction",
        "created_at": created,
        "status": "in_progress",
        "model": model,
        "output": []
      }
    });
    send_sse(
      &tx,
      openai_named_event("response.created", &created_ev.to_string()),
    )
    .await;

    let item_added = serde_json::json!({
      "type": "response.output_item.added",
      "output_index": base_output.len(),
      "item": {
        "id": cmp_id,
        "type": "compaction",
        "status": "in_progress"
      }
    });
    send_sse(
      &tx,
      openai_named_event("response.output_item.added", &item_added.to_string()),
    )
    .await;

    let mut full = String::new();
    let mut final_usage: Option<ResponseUsage> = None;
    let mut stream_id = id.clone();

    while let Some(item) = upstream.next().await {
      match item {
        Ok(ExecStreamEvent::Meta { id: mid, .. }) => {
          if let Some(mid) = mid.filter(|s| !s.is_empty()) {
            stream_id = mid;
          }
        }
        Ok(ExecStreamEvent::ContentDelta { text: delta }) => {
          if delta.is_empty() {
            continue;
          }
          full.push_str(&delta);
          let ev = serde_json::json!({
            "type": "response.compaction.delta",
            "item_id": cmp_id,
            "output_index": base_output.len(),
            "delta": delta
          });
          send_sse(
            &tx,
            openai_named_event("response.compaction.delta", &ev.to_string()),
          )
          .await;
        }
        Ok(ExecStreamEvent::ReasoningDelta { .. }) => {}
        Ok(ExecStreamEvent::Done { usage, .. }) => {
          if let Some(u) = usage {
            final_usage = Some(usage_from_exec(&u));
          }
          break;
        }
        Err(e) => {
          tracing::warn!(error = %e, "compact stream error");
          let failed = serde_json::json!({
            "type": "response.failed",
            "response": {
              "id": stream_id,
              "object": "response.compaction",
              "created_at": created,
              "status": "failed",
              "model": model,
              "output": [],
              "error": { "message": e.to_string(), "code": "server_error" }
            }
          });
          send_sse(
            &tx,
            openai_named_event("response.failed", &failed.to_string()),
          )
          .await;
          return;
        }
      }
    }

    let usage = final_usage.unwrap_or_else(|| estimate_usage(&text, &full));
    let completed = build_compacted(stream_id, base_output, full, usage);

    let item_done = serde_json::json!({
      "type": "response.output_item.done",
      "output_index": completed.output.len().saturating_sub(1),
      "item": completed.output.last()
    });
    send_sse(
      &tx,
      openai_named_event("response.output_item.done", &item_done.to_string()),
    )
    .await;

    let completed_ev = serde_json::json!({
      "type": "response.completed",
      "response": completed
    });
    send_sse(
      &tx,
      openai_named_event("response.completed", &completed_ev.to_string()),
    )
    .await;
  });

  Ok(sse_response(rx).into_response())
}

async fn run_compact(
  state: &AppState,
  req: &CompactRequest,
  text: &str,
) -> OpenAiResult<(String, String, ResponseUsage)> {
  if let Some(model) = req.model.as_deref().filter(|m| !m.is_empty()) {
    state.runtime.models().require(model).await?;
    state.runtime.refresh_access_token_if_needed().await?;

    let compact_req = ExecCompactRequest::from_compact(req, model);
    let result = state.provider.execute_compact(&compact_req).await?;
    let usage = if result.usage.total_tokens > 0 {
      usage_from_exec(&result.usage)
    } else {
      estimate_usage(text, &result.content)
    };
    Ok((result.id, result.content, usage))
  } else {
    let tokens = estimate_tokens(text);
    Ok((
      format!("resp_{}", Uuid::new_v4().simple()),
      text.to_string(),
      ResponseUsage {
        input_tokens: tokens,
        output_tokens: 0,
        total_tokens: tokens,
      },
    ))
  }
}

fn build_compacted(
  id: String,
  mut output: Vec<ResponseListItem>,
  summary: String,
  usage: ResponseUsage,
) -> CompactedResponse {
  output.push(ResponseListItem {
    id: format!("cmp_{}", Uuid::new_v4().simple()),
    item_type: "compaction".into(),
    role: None,
    content: Some(ResponseContent::Text(summary.clone())),
    encrypted_content: Some(summary),
    status: Some("completed".into()),
  });
  CompactedResponse {
    id,
    created_at: Utc::now().timestamp(),
    object: "response.compaction",
    output,
    usage,
  }
}

/// Emit a one-shot SSE for non-provider compact (no model / passthrough).
fn synthetic_compact_sse(body: CompactedResponse, created: i64) -> impl IntoResponse {
  let (tx, rx) = sse_channel(8);
  tokio::spawn(async move {
    let created_ev = serde_json::json!({
      "type": "response.created",
      "response": {
        "id": body.id,
        "object": "response.compaction",
        "created_at": created,
        "status": "in_progress",
        "output": []
      }
    });
    send_sse(
      &tx,
      openai_named_event("response.created", &created_ev.to_string()),
    )
    .await;
    if let Some(item) = body.output.last() {
      if let Some(enc) = item.encrypted_content.as_ref() {
        let delta = serde_json::json!({
          "type": "response.compaction.delta",
          "item_id": item.id,
          "delta": enc
        });
        send_sse(
          &tx,
          openai_named_event("response.compaction.delta", &delta.to_string()),
        )
        .await;
      }
    }
    let completed_ev = serde_json::json!({
      "type": "response.completed",
      "response": body
    });
    send_sse(
      &tx,
      openai_named_event("response.completed", &completed_ev.to_string()),
    )
    .await;
  });
  sse_response(rx)
}

fn estimate_tokens(s: &str) -> u32 {
  if s.is_empty() {
    return 0;
  }
  ((s.len() as u32) / 4).max(1)
}
