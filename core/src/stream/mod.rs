//! Helpers for Server-Sent Events (SSE) streaming responses.
//!
//! Event streams use **tokio-stream** (`ReceiverStream` + [`StreamExt`]).

use std::convert::Infallible;

use axum::response::sse::{Event, KeepAlive, Sse};
use tokio::sync::mpsc;
use tokio_stream::Stream;
use tokio_stream::wrappers::ReceiverStream;

/// One SSE frame as consumed by Axum's [`Sse`] body.
pub type SseItem = Result<Event, Infallible>;

/// Build an SSE response from a tokio/futures stream of events (with keep-alive).
pub fn sse_response<S>(stream: S) -> Sse<axum::response::sse::KeepAliveStream<S>>
where
  S: Stream<Item = SseItem> + Send + 'static,
{
  Sse::new(stream).keep_alive(KeepAlive::default())
}

/// Channel-backed SSE stream via [`ReceiverStream`].
///
/// Spawn a task that pushes frames with [`send_sse`], then pass the stream to
/// [`sse_response`].
pub fn sse_channel(buffer: usize) -> (mpsc::Sender<SseItem>, ReceiverStream<SseItem>) {
  let (tx, rx) = mpsc::channel(buffer);
  (tx, ReceiverStream::new(rx))
}

/// Push one SSE frame; ignores closed receivers (client disconnect).
pub async fn send_sse(tx: &mpsc::Sender<SseItem>, event: Event) {
  let _ = tx.send(Ok(event)).await;
}

/// Format one OpenAI-style SSE data line payload (JSON object as string).
///
/// Used by Chat Completions streaming (`data: {…}` only; no `event:` line).
pub fn openai_data_event(json: &str) -> Event {
  Event::default().data(json.to_string())
}

/// OpenAI Responses streaming frame: both `event:` and `data:` lines.
///
/// Event name matches the JSON `type` field (e.g. `response.output_text.delta`).
pub fn openai_named_event(event_type: &str, json: &str) -> Event {
  Event::default().event(event_type).data(json.to_string())
}

/// OpenAI Chat Completions stream terminator (`data: [DONE]`).
pub fn openai_done_event() -> Event {
  Event::default().data("[DONE]")
}

/// Anthropic-style named SSE event.
pub fn anthropic_event(event_type: &str, json: &str) -> Event {
  Event::default().event(event_type).data(json.to_string())
}
