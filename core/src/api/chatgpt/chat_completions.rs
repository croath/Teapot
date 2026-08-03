//! OpenAI Chat Completions: POST /chatgpt/v1/chat/completions
//!
//! Supports non-stream JSON and true SSE streaming (`stream: true`) via each
//! provider's `execute_stream`.

use axum::extract::State;
use axum::response::{IntoResponse, Response};
use chrono::Utc;
use tokio_stream::StreamExt;

use super::json::OpenAiJson;
use crate::api::state::AppState;
use crate::error::{AppError, OpenAiResult};
use crate::models::openai::{
  ChatChoice, ChatChunkChoice, ChatCompletionChunk, ChatCompletionRequest, ChatCompletionResponse,
  ChatContent, ChatDelta, ChatMessage,
};
use crate::providers::execute::{ExecRequest, ExecStreamEvent};
use crate::stream::{openai_data_event, openai_done_event, send_sse, sse_channel, sse_response};

pub async fn chat_completions(
  State(state): State<AppState>,
  OpenAiJson(req): OpenAiJson<ChatCompletionRequest>,
) -> OpenAiResult<Response> {
  if req.messages.is_empty() {
    return Err(AppError::BadRequest("messages must not be empty".into()).into());
  }

  tracing::info!(
    model = %req.model,
    stream = req.stream,
    messages = req.messages.len(),
    "chat/completions"
  );

  // No automatic provider/model adaptation: model must be in the pinned catalog.
  state.runtime.models().require(&req.model).await?;

  // Keep provider's own session fresh before use (does not run inside execute).
  state.runtime.refresh_access_token_if_needed().await?;

  let exec_req = ExecRequest::from_chat(&req);

  if req.stream {
    stream_completions(state, exec_req).await
  } else {
    let result = state.provider.execute(&exec_req).await?;
    tracing::info!(
      id = %result.id,
      model = %result.model,
      prompt_tokens = result.usage.prompt_tokens,
      completion_tokens = result.usage.completion_tokens,
      "chat/completions done"
    );
    let body = ChatCompletionResponse {
      id: result.id,
      object: "chat.completion",
      created: Utc::now().timestamp(),
      model: result.model,
      choices: vec![ChatChoice {
        index: 0,
        message: ChatMessage {
          role: "assistant".into(),
          content: ChatContent::Text(result.content),
          name: None,
        },
        finish_reason: Some("stop"),
      }],
      usage: result.usage,
    };
    Ok(OpenAiJson(body).into_response())
  }
}

async fn stream_completions(state: AppState, exec_req: ExecRequest) -> OpenAiResult<Response> {
  let mut upstream = state.provider.execute_stream(&exec_req).await?;

  let id = format!("chatcmpl-{}", uuid::Uuid::new_v4().simple());
  let mut model = exec_req.model.clone();
  let created = Utc::now().timestamp();
  let (tx, rx) = sse_channel(64);

  tokio::spawn(async move {
    let mut sent_role = false;
    let mut stream_id = id.clone();

    while let Some(item) = upstream.next().await {
      match item {
        Ok(ExecStreamEvent::Meta { model: m, id: eid }) => {
          if let Some(m) = m.filter(|s| !s.is_empty()) {
            model = m;
          }
          if let Some(eid) = eid.filter(|s| !s.is_empty()) {
            stream_id = eid;
          }
        }
        Ok(ExecStreamEvent::ContentDelta { text }) => {
          if text.is_empty() {
            continue;
          }
          if !sent_role {
            sent_role = true;
            let first = ChatCompletionChunk {
              id: stream_id.clone(),
              object: "chat.completion.chunk",
              created,
              model: model.clone(),
              choices: vec![ChatChunkChoice {
                index: 0,
                delta: ChatDelta {
                  role: Some("assistant"),
                  content: Some(text),
                  reasoning_content: None,
                  status: None,
                },
                finish_reason: None,
              }],
            };
            if let Ok(json) = serde_json::to_string(&first) {
              send_sse(&tx, openai_data_event(&json)).await;
            }
          } else {
            let chunk = ChatCompletionChunk {
              id: stream_id.clone(),
              object: "chat.completion.chunk",
              created,
              model: model.clone(),
              choices: vec![ChatChunkChoice {
                index: 0,
                delta: ChatDelta {
                  role: None,
                  content: Some(text),
                  reasoning_content: None,
                  status: None,
                },
                finish_reason: None,
              }],
            };
            if let Ok(json) = serde_json::to_string(&chunk) {
              send_sse(&tx, openai_data_event(&json)).await;
            }
          }
        }
        Ok(ExecStreamEvent::ReasoningDelta { text }) => {
          if text.is_empty() {
            continue;
          }
          let chunk = ChatCompletionChunk {
            id: stream_id.clone(),
            object: "chat.completion.chunk",
            created,
            model: model.clone(),
            choices: vec![ChatChunkChoice {
              index: 0,
              delta: ChatDelta {
                role: None,
                content: None,
                reasoning_content: Some(text),
                status: None,
              },
              finish_reason: None,
            }],
          };
          if let Ok(json) = serde_json::to_string(&chunk) {
            send_sse(&tx, openai_data_event(&json)).await;
          }
        }
        Ok(ExecStreamEvent::Done { finish_reason, .. }) => {
          if !sent_role {
            // Empty completion: still emit a role-only chunk for client compatibility.
            let first = ChatCompletionChunk {
              id: stream_id.clone(),
              object: "chat.completion.chunk",
              created,
              model: model.clone(),
              choices: vec![ChatChunkChoice {
                index: 0,
                delta: ChatDelta {
                  role: Some("assistant"),
                  content: None,
                  reasoning_content: None,
                  status: None,
                },
                finish_reason: None,
              }],
            };
            if let Ok(json) = serde_json::to_string(&first) {
              send_sse(&tx, openai_data_event(&json)).await;
            }
          }
          let last = ChatCompletionChunk {
            id: stream_id.clone(),
            object: "chat.completion.chunk",
            created,
            model: model.clone(),
            choices: vec![ChatChunkChoice {
              index: 0,
              delta: ChatDelta::default(),
              finish_reason: finish_reason.or(Some("stop")),
            }],
          };
          if let Ok(json) = serde_json::to_string(&last) {
            send_sse(&tx, openai_data_event(&json)).await;
          }
          send_sse(&tx, openai_done_event()).await;
          return;
        }
        Err(e) => {
          tracing::warn!(error = %e, "chat completions stream error");
          // Best-effort error as a final content chunk, then close.
          let err_text = format!("[error] {e}");
          let chunk = ChatCompletionChunk {
            id: stream_id.clone(),
            object: "chat.completion.chunk",
            created,
            model: model.clone(),
            choices: vec![ChatChunkChoice {
              index: 0,
              delta: ChatDelta {
                role: if sent_role { None } else { Some("assistant") },
                content: Some(err_text),
                reasoning_content: None,
                status: None,
              },
              finish_reason: Some("stop"),
            }],
          };
          if let Ok(json) = serde_json::to_string(&chunk) {
            send_sse(&tx, openai_data_event(&json)).await;
          }
          send_sse(&tx, openai_done_event()).await;
          return;
        }
      }
    }

    // Upstream closed without Done.
    let last = ChatCompletionChunk {
      id: stream_id,
      object: "chat.completion.chunk",
      created,
      model,
      choices: vec![ChatChunkChoice {
        index: 0,
        delta: ChatDelta::default(),
        finish_reason: Some("stop"),
      }],
    };
    if let Ok(json) = serde_json::to_string(&last) {
      send_sse(&tx, openai_data_event(&json)).await;
    }
    send_sse(&tx, openai_done_event()).await;
  });

  Ok(sse_response(rx).into_response())
}
