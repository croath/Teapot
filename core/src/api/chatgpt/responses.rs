//! OpenAI Responses API: POST /chatgpt/v1/responses

use std::convert::Infallible;

use async_stream::stream;
use axum::extract::State;
use axum::response::sse::Event;
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::Utc;
use uuid::Uuid;

use crate::agents::{flatten_messages, AgentEvent};
use crate::api::state::AppState;
use crate::error::{AppError, AppResult};
use crate::models::openai::*;
use crate::stream::{openai_data_event, openai_done_event, sse_response};

pub async fn create_response(
  State(state): State<AppState>,
  Json(req): Json<ResponsesRequest>,
) -> AppResult<Response> {
  let model = req.model.clone();
  let agent_model = req
    .agent
    .clone()
    .unwrap_or_else(|| model.clone());

  let (input_system, msgs) = req.input.as_prompt_messages();
  let system = match (req.instructions.as_ref(), input_system) {
    (Some(i), Some(s)) => Some(format!("{i}\n\n{s}")),
    (Some(i), None) => Some(i.clone()),
    (None, s) => s,
  };
  let (system, prompt) = flatten_messages(system.as_deref(), &msgs);

  if prompt.trim().is_empty() {
    return Err(AppError::BadRequest("input must not be empty".into()));
  }

  if req.stream {
    Ok(stream_response(state, agent_model, model, system, prompt).await?)
  } else {
    Ok(complete_response(state, &agent_model, model, system.as_deref(), &prompt).await?)
  }
}

async fn complete_response(
  state: AppState,
  agent_model: &str,
  model: String,
  system: Option<&str>,
  prompt: &str,
) -> AppResult<Response> {
  let text = state.runner.run_collect(agent_model, system, prompt).await?;
  let id = format!("resp_{}", Uuid::new_v4().simple());
  let msg_id = format!("msg_{}", Uuid::new_v4().simple());
  let created = Utc::now().timestamp();
  let output_tokens = estimate_tokens(&text);
  let input_tokens = estimate_tokens(prompt);

  let body = ResponsesResponse {
    id,
    object: "response",
    created_at: created,
    status: "completed",
    model,
    output: vec![ResponseOutputItem {
      id: msg_id,
      item_type: "message",
      role: "assistant",
      content: vec![ResponseOutputContent {
        content_type: "output_text",
        text,
      }],
      status: "completed",
    }],
    usage: ResponseUsage {
      input_tokens,
      output_tokens,
      total_tokens: input_tokens + output_tokens,
    },
  };
  Ok(Json(body).into_response())
}

async fn stream_response(
  state: AppState,
  agent_model: String,
  model: String,
  system: Option<String>,
  prompt: String,
) -> AppResult<Response> {
  let mut session = state
    .runner
    .run(&agent_model, system.as_deref(), &prompt)
    .await?;

  let id = format!("resp_{}", Uuid::new_v4().simple());
  let msg_id = format!("msg_{}", Uuid::new_v4().simple());
  let created = Utc::now().timestamp();

  let s = stream! {
    // response.created
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
    yield Ok::<Event, Infallible>(openai_data_event(&created_ev.to_string()));

    // response.output_item.added
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
    yield Ok(openai_data_event(&item_added.to_string()));

    // response.content_part.added
    let part_added = serde_json::json!({
      "type": "response.content_part.added",
      "item_id": msg_id,
      "output_index": 0,
      "content_index": 0,
      "part": { "type": "output_text", "text": "" }
    });
    yield Ok(openai_data_event(&part_added.to_string()));

    let mut full = String::new();
    loop {
      match session.recv().await {
        Some(AgentEvent::Token(text)) => {
          full.push_str(&text);
          let delta = serde_json::json!({
            "type": "response.output_text.delta",
            "item_id": msg_id,
            "output_index": 0,
            "content_index": 0,
            "delta": text
          });
          yield Ok(openai_data_event(&delta.to_string()));
        }
        Some(AgentEvent::Stderr(_)) => {}
        Some(AgentEvent::Done { .. }) | None => {
          let done_text = serde_json::json!({
            "type": "response.output_text.done",
            "item_id": msg_id,
            "output_index": 0,
            "content_index": 0,
            "text": full
          });
          yield Ok(openai_data_event(&done_text.to_string()));

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
          yield Ok(openai_data_event(&item_done.to_string()));

          let completed = serde_json::json!({
            "type": "response.completed",
            "response": {
              "id": id,
              "object": "response",
              "created_at": created,
              "status": "completed",
              "model": model,
              "output": [{
                "id": msg_id,
                "type": "message",
                "role": "assistant",
                "status": "completed",
                "content": [{ "type": "output_text", "text": full }]
              }]
            }
          });
          yield Ok(openai_data_event(&completed.to_string()));
          yield Ok(openai_done_event());
          break;
        }
        Some(AgentEvent::Failed(msg)) => {
          let failed = serde_json::json!({
            "type": "response.failed",
            "response": {
              "id": id,
              "status": "failed",
              "error": { "message": msg }
            }
          });
          yield Ok(openai_data_event(&failed.to_string()));
          yield Ok(openai_done_event());
          break;
        }
      }
    }
  };

  Ok(sse_response(s).into_response())
}

fn estimate_tokens(s: &str) -> u32 {
  ((s.len() as u32) / 4).max(1)
}
