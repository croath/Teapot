//! Helpers for Server-Sent Events (SSE) streaming responses.

use axum::response::sse::{Event, KeepAlive, Sse};
use futures::Stream;
use std::convert::Infallible;

/// Build an SSE response from a stream of events (with keep-alive pings).
pub fn sse_response<S>(stream: S) -> Sse<axum::response::sse::KeepAliveStream<S>>
where
  S: Stream<Item = Result<Event, Infallible>> + Send + 'static,
{
  Sse::new(stream).keep_alive(KeepAlive::default())
}

/// Format one OpenAI-style SSE data line payload (JSON object as string).
pub fn openai_data_event(json: &str) -> Event {
  Event::default().data(json.to_string())
}

/// OpenAI stream terminator.
pub fn openai_done_event() -> Event {
  Event::default().data("[DONE]")
}

/// Anthropic-style named SSE event.
pub fn anthropic_event(event_type: &str, json: &str) -> Event {
  Event::default().event(event_type).data(json.to_string())
}
