//! Anthropic Messages API: POST /claude/v1/messages

use std::convert::Infallible;

use async_stream::stream;
use axum::extract::State;
use axum::response::sse::Event;
use axum::response::{IntoResponse, Response};
use axum::Json;
use uuid::Uuid;

use crate::agents::{flatten_messages, AgentEvent};
use crate::api::state::AppState;
use crate::error::{AppError, AppResult};
use crate::models::anthropic::*;
use crate::stream::{anthropic_event, sse_response};

pub async fn create_message(
  State(state): State<AppState>,
  Json(req): Json<MessagesRequest>,
) -> AppResult<Response> {
  if req.messages.is_empty() {
    return Err(AppError::BadRequest("messages must not be empty".into()));
  }

  let model = req.model.clone();
  let agent_model = req
    .agent
    .clone()
    .unwrap_or_else(|| model.clone());

  let system = req.system.as_ref().map(|s| s.as_text());
  let msgs: Vec<(String, String)> = req
    .messages
    .iter()
    .map(|m| (m.role.clone(), m.content.as_text()))
    .collect();
  let (system, prompt) = flatten_messages(system.as_deref(), &msgs);

  if req.stream {
    Ok(stream_message(state, agent_model, model, system, prompt).await?)
  } else {
    Ok(complete_message(state, &agent_model, model, system.as_deref(), &prompt).await?)
  }
}

async fn complete_message(
  state: AppState,
  agent_model: &str,
  model: String,
  system: Option<&str>,
  prompt: &str,
) -> AppResult<Response> {
  let text = state.runner.run_collect(agent_model, system, prompt).await?;
  let id = format!("msg_{}", Uuid::new_v4().simple());
  let output_tokens = estimate_tokens(&text);
  let input_tokens = estimate_tokens(prompt);

  let body = MessagesResponse {
    id,
    response_type: "message",
    role: "assistant",
    content: vec![ContentBlock::Text {
      text,
      citations: None,
    }],
    model,
    stop_reason: Some("end_turn"),
    stop_sequence: None,
    usage: AnthropicUsage {
      input_tokens,
      output_tokens,
    },
  };
  Ok(Json(body).into_response())
}

async fn stream_message(
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

  let id = format!("msg_{}", Uuid::new_v4().simple());
  let input_tokens = estimate_tokens(&prompt);

  let s = stream! {
    let start = MessagesResponse {
      id: id.clone(),
      response_type: "message",
      role: "assistant",
      content: vec![],
      model: model.clone(),
      stop_reason: None,
      stop_sequence: None,
      usage: AnthropicUsage {
        input_tokens,
        output_tokens: 0,
      },
    };
    let start_ev = StreamEvent {
      event_type: "message_start",
      message: Some(start),
      index: None,
      content_block: None,
      delta: None,
      usage: None,
    };
    if let Ok(json) = serde_json::to_string(&start_ev) {
      yield Ok::<Event, Infallible>(anthropic_event("message_start", &json));
    }

    let block_start = StreamEvent {
      event_type: "content_block_start",
      message: None,
      index: Some(0),
      content_block: Some(ContentBlock::Text {
        text: String::new(),
        citations: None,
      }),
      delta: None,
      usage: None,
    };
    if let Ok(json) = serde_json::to_string(&block_start) {
      yield Ok(anthropic_event("content_block_start", &json));
    }

    // Ping (clients ignore unknown events)
    yield Ok(anthropic_event(
      "ping",
      &serde_json::json!({ "type": "ping" }).to_string(),
    ));

    let mut output_tokens = 0u32;
    loop {
      match session.recv().await {
        Some(AgentEvent::Token(text)) => {
          output_tokens += estimate_tokens(&text);
          let delta = StreamEvent {
            event_type: "content_block_delta",
            message: None,
            index: Some(0),
            content_block: None,
            delta: Some(StreamDelta {
              delta_type: Some("text_delta"),
              text: Some(text),
              stop_reason: None,
            }),
            usage: None,
          };
          if let Ok(json) = serde_json::to_string(&delta) {
            yield Ok(anthropic_event("content_block_delta", &json));
          }
        }
        Some(AgentEvent::Stderr(_)) => {}
        Some(AgentEvent::Done { .. }) | None => {
          let block_stop = StreamEvent {
            event_type: "content_block_stop",
            message: None,
            index: Some(0),
            content_block: None,
            delta: None,
            usage: None,
          };
          if let Ok(json) = serde_json::to_string(&block_stop) {
            yield Ok(anthropic_event("content_block_stop", &json));
          }

          let delta_stop = StreamEvent {
            event_type: "message_delta",
            message: None,
            index: None,
            content_block: None,
            delta: Some(StreamDelta {
              delta_type: None,
              text: None,
              stop_reason: Some("end_turn"),
            }),
            usage: Some(AnthropicUsage {
              input_tokens: 0,
              output_tokens,
            }),
          };
          if let Ok(json) = serde_json::to_string(&delta_stop) {
            yield Ok(anthropic_event("message_delta", &json));
          }

          let stop = StreamEvent {
            event_type: "message_stop",
            message: None,
            index: None,
            content_block: None,
            delta: None,
            usage: None,
          };
          if let Ok(json) = serde_json::to_string(&stop) {
            yield Ok(anthropic_event("message_stop", &json));
          }
          break;
        }
        Some(AgentEvent::Failed(msg)) => {
          let err = serde_json::json!({
            "type": "error",
            "error": { "type": "api_error", "message": msg }
          });
          yield Ok(anthropic_event("error", &err.to_string()));
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
