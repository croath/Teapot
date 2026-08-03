//! OpenAI Chat Completions: POST /chatgpt/v1/chat/completions

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

pub async fn chat_completions(
  State(state): State<AppState>,
  Json(req): Json<ChatCompletionRequest>,
) -> AppResult<Response> {
  if req.messages.is_empty() {
    return Err(AppError::BadRequest("messages must not be empty".into()));
  }

  let model = req.model.clone();
  let agent_model = req
    .agent
    .clone()
    .unwrap_or_else(|| model.clone());

  let msgs: Vec<(String, String)> = req
    .messages
    .iter()
    .map(|m| (m.role.clone(), m.content.as_text()))
    .collect();
  let (system, prompt) = flatten_messages(None, &msgs);

  if req.stream {
    Ok(stream_chat(state, agent_model, model, system, prompt).await?)
  } else {
    Ok(complete_chat(state, &agent_model, model, system.as_deref(), &prompt).await?)
  }
}

async fn complete_chat(
  state: AppState,
  agent_model: &str,
  model: String,
  system: Option<&str>,
  prompt: &str,
) -> AppResult<Response> {
  let text = state.runner.run_collect(agent_model, system, prompt).await?;
  let id = format!("chatcmpl-{}", Uuid::new_v4().simple());
  let created = Utc::now().timestamp();
  let completion_tokens = estimate_tokens(&text);
  let prompt_tokens = estimate_tokens(prompt);

  let body = ChatCompletionResponse {
    id,
    object: "chat.completion",
    created,
    model,
    choices: vec![ChatChoice {
      index: 0,
      message: ChatMessage {
        role: "assistant".into(),
        content: ChatContent::Text(text),
        name: None,
      },
      finish_reason: Some("stop"),
    }],
    usage: Usage {
      prompt_tokens,
      completion_tokens,
      total_tokens: prompt_tokens + completion_tokens,
    },
  };
  Ok(Json(body).into_response())
}

async fn stream_chat(
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

  let id = format!("chatcmpl-{}", Uuid::new_v4().simple());
  let created = Utc::now().timestamp();

  let s = stream! {
    // Role chunk first (OpenAI convention)
    let first = ChatCompletionChunk {
      id: id.clone(),
      object: "chat.completion.chunk",
      created,
      model: model.clone(),
      choices: vec![ChatChunkChoice {
        index: 0,
        delta: ChatDelta {
          role: Some("assistant"),
          content: None,
        },
        finish_reason: None,
      }],
    };
    if let Ok(json) = serde_json::to_string(&first) {
      yield Ok::<Event, Infallible>(openai_data_event(&json));
    }

    loop {
      match session.recv().await {
        Some(AgentEvent::Token(text)) => {
          let chunk = ChatCompletionChunk {
            id: id.clone(),
            object: "chat.completion.chunk",
            created,
            model: model.clone(),
            choices: vec![ChatChunkChoice {
              index: 0,
              delta: ChatDelta {
                role: None,
                content: Some(text),
              },
              finish_reason: None,
            }],
          };
          if let Ok(json) = serde_json::to_string(&chunk) {
            yield Ok(openai_data_event(&json));
          }
        }
        Some(AgentEvent::Stderr(_)) => {}
        Some(AgentEvent::Done { .. }) => {
          let last = ChatCompletionChunk {
            id: id.clone(),
            object: "chat.completion.chunk",
            created,
            model: model.clone(),
            choices: vec![ChatChunkChoice {
              index: 0,
              delta: ChatDelta::default(),
              finish_reason: Some("stop"),
            }],
          };
          if let Ok(json) = serde_json::to_string(&last) {
            yield Ok(openai_data_event(&json));
          }
          yield Ok(openai_done_event());
          break;
        }
        Some(AgentEvent::Failed(msg)) => {
          let err = serde_json::json!({
            "error": { "message": msg, "type": "api_error" }
          });
          yield Ok(openai_data_event(&err.to_string()));
          yield Ok(openai_done_event());
          break;
        }
        None => {
          yield Ok(openai_done_event());
          break;
        }
      }
    }
  };

  Ok(sse_response(s).into_response())
}

fn estimate_tokens(s: &str) -> u32 {
  // Rough heuristic: ~4 chars per token
  ((s.len() as u32) / 4).max(1)
}
