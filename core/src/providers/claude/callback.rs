//! Claude local OAuth redirect listener (`/callback`).

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::Query;
use axum::response::Html;
use axum::routing::get;
use serde::Deserialize;
use tokio::sync::{Mutex, oneshot};
use tracing::info;

use crate::error::{AppError, AppResult};

use super::ClaudeProvider;

/// Authorization code + state from Anthropic redirect.
#[derive(Debug, Clone)]
pub(super) struct CallbackResult {
  pub code: String,
  pub state: String,
}

#[derive(Debug, Deserialize)]
struct CallbackQuery {
  code: Option<String>,
  state: Option<String>,
  error: Option<String>,
  error_description: Option<String>,
}

impl ClaudeProvider {
  /// Listen on `127.0.0.1:port` for Claude redirect to the given path.
  pub(super) async fn wait_for_callback(
    &self,
    port: u16,
    path: &str,
    expected_state: Option<&str>,
    timeout: Duration,
  ) -> AppResult<CallbackResult> {
    let path = if path.starts_with('/') {
      path.to_string()
    } else {
      format!("/{path}")
    };

    let (tx, rx) = oneshot::channel::<Result<CallbackResult, String>>();
    let tx = Arc::new(Mutex::new(Some(tx)));
    let expected = expected_state.map(|s| s.to_string());

    let handler_tx = Arc::clone(&tx);
    let app = Router::new().route(
      &path,
      get(move |Query(q): Query<CallbackQuery>| {
        let handler_tx = Arc::clone(&handler_tx);
        let expected = expected.clone();
        async move {
          let result = process_query(q, expected.as_deref());
          let html = match &result {
            Ok(_) => SUCCESS_HTML.to_string(),
            Err(e) => format!(
              r#"<!DOCTYPE html><html><body style="font-family:system-ui;padding:2rem">
            <h1>Claude authentication failed</h1><p>{e}</p></body></html>"#
            ),
          };
          if let Some(sender) = handler_tx.lock().await.take() {
            let _ = sender.send(result);
          }
          Html(html)
        }
      }),
    );

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr)
      .await
      .map_err(|e| AppError::Internal(format!("claude: bind OAuth callback {addr}: {e}")))?;
    let bound = listener
      .local_addr()
      .map_err(|e| AppError::Internal(format!("local_addr: {e}")))?;
    info!(%bound, path = %path, "claude OAuth callback listening");

    let server = axum::serve(listener, app);
    let _server = crate::auth::AbortOnDrop(tokio::spawn(async move {
      let _ = server.await;
    }));

    let outcome = tokio::time::timeout(timeout, rx).await;

    match outcome {
      Ok(Ok(Ok(res))) => Ok(res),
      Ok(Ok(Err(msg))) => Err(AppError::Unauthorized(msg)),
      Ok(Err(_)) => Err(AppError::Internal(
        "claude: OAuth callback channel closed unexpectedly".into(),
      )),
      Err(_) => Err(AppError::Unauthorized(
        "claude: timed out waiting for OAuth callback".into(),
      )),
    }
  }
}

fn process_query(q: CallbackQuery, expected_state: Option<&str>) -> Result<CallbackResult, String> {
  if let Some(err) = q.error {
    let desc = q.error_description.unwrap_or_default();
    return Err(format!("provider error: {err} {desc}").trim().to_string());
  }
  let code = q
    .code
    .filter(|s| !s.trim().is_empty())
    .ok_or_else(|| "missing authorization code".to_string())?;
  let state = q.state.unwrap_or_default();
  if let Some(expected) = expected_state {
    if state != expected {
      return Err("state mismatch (possible CSRF)".into());
    }
  }
  Ok(CallbackResult { code, state })
}

const SUCCESS_HTML: &str = r#"<!DOCTYPE html>
<html><head><meta charset="utf-8"><title>Teapot — Claude</title></head>
<body style="font-family:system-ui;padding:2rem;max-width:32rem">
  <h1>Claude authentication complete</h1>
  <p>You can close this window and return to the terminal.</p>
</body></html>"#;
