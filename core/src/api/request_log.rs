//! HTTP request access log for API surfaces.
//!
//! Logs method, path, status, and latency at info level. Does not log bodies
//! or prompts (see AGENTS security notes).

use std::time::Instant;

use axum::extract::{OriginalUri, Request};
use axum::middleware::Next;
use axum::response::Response;
use tracing::info;

/// Axum middleware: log one line per HTTP request after the handler returns.
///
/// For streaming responses, latency is time-to-headers (first byte), not end of
/// body — that matches typical reverse-proxy access logs.
pub async fn log_request(request: Request, next: Next) -> Response {
  let method = request.method().clone();
  let path = request
    .extensions()
    .get::<OriginalUri>()
    .map(|u| u.0.path().to_string())
    .unwrap_or_else(|| request.uri().path().to_string());
  let query = request
    .extensions()
    .get::<OriginalUri>()
    .and_then(|u| u.0.query().map(str::to_owned))
    .or_else(|| request.uri().query().map(str::to_owned));

  let start = Instant::now();
  let response = next.run(request).await;
  let status = response.status().as_u16();
  let latency_ms = start.elapsed().as_millis();

  if let Some(q) = query.as_deref().filter(|s| !s.is_empty()) {
    info!(
      method = %method,
      path = %path,
      query = %q,
      status,
      latency_ms,
      "request"
    );
  } else {
    info!(
      method = %method,
      path = %path,
      status,
      latency_ms,
      "request"
    );
  }

  response
}
