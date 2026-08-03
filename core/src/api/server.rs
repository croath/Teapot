//! Axum server bootstrap.

use std::future::Future;
use std::net::SocketAddr;

use axum::Router;
use axum::middleware;
use axum::routing::get;
use tokio::sync::oneshot;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing::info;

use super::auth::require_api_key;
use super::chatgpt;
use super::claude;
use super::state::AppState;
use crate::config::Config;

/// Build the full application router.
pub fn build_router(state: AppState) -> Router {
  // Authenticated API surface
  let api = Router::new()
    .nest("/chatgpt/v1", chatgpt::router())
    .nest("/claude/v1", claude::router())
    .layer(middleware::from_fn_with_state(
      state.clone(),
      require_api_key,
    ));

  // Public health checks (no API key)
  let public = Router::new()
    .route("/health", get(health))
    .route("/healthz", get(health));

  Router::new()
    .merge(public)
    .merge(api)
    .layer(
      CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any),
    )
    .layer(TraceLayer::new_for_http())
    .with_state(state)
}

async fn health() -> axum::Json<serde_json::Value> {
  axum::Json(serde_json::json!({
    "status": "ok",
    "service": "teaport"
  }))
}

/// Bind and serve until the process is stopped (Ctrl-C / kill).
pub async fn serve(config: Config) -> anyhow::Result<()> {
  serve_with_shutdown(config, std::future::pending::<()>()).await
}

/// Bind and serve until `shutdown` completes, then drain connections.
pub async fn serve_with_shutdown<F>(config: Config, shutdown: F) -> anyhow::Result<()>
where
  F: Future<Output = ()> + Send + 'static,
{
  let listen = config.listen.clone();
  let active_agent = config.active_agent.clone();
  let state = AppState::new(config);
  let app = build_router(state);

  let addr: SocketAddr = listen
    .parse()
    .map_err(|e| anyhow::anyhow!("invalid listen address {listen}: {e}"))?;

  info!(%addr, "Teaport listening");
  if let Some(agent) = &active_agent {
    info!(%agent, "active agent CLI (models + default routing pinned)");
  }
  info!("ChatGPT API:  http://{addr}/chatgpt/v1/chat/completions");
  info!("Responses API: http://{addr}/chatgpt/v1/responses");
  info!("Models API:   http://{addr}/chatgpt/v1/models");
  info!("Claude API:   http://{addr}/claude/v1/messages");
  info!("Claude models: http://{addr}/claude/v1/models");

  let listener = tokio::net::TcpListener::bind(addr).await?;
  axum::serve(listener, app)
    .with_graceful_shutdown(shutdown)
    .await?;
  info!(%addr, "Teaport stopped");
  Ok(())
}

/// Handle used to stop a server that was started with [`start_server`].
pub struct ServerHandle {
  shutdown: Option<oneshot::Sender<()>>,
  pub listen: String,
}

impl ServerHandle {
  /// Request graceful shutdown. Safe to call more than once.
  pub fn stop(&mut self) {
    if let Some(tx) = self.shutdown.take() {
      let _ = tx.send(());
    }
  }
}

impl Drop for ServerHandle {
  fn drop(&mut self) {
    self.stop();
  }
}

/// Start the server on the current Tokio runtime and return a stop handle.
///
/// The server runs as a background task on the runtime that calls this function.
pub fn start_server(config: Config) -> ServerHandle {
  let listen = config.listen.clone();
  let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

  tokio::spawn(async move {
    let shutdown = async {
      let _ = shutdown_rx.await;
    };
    if let Err(e) = serve_with_shutdown(config, shutdown).await {
      tracing::error!(error = %e, "API server exited with error");
    }
  });

  ServerHandle {
    shutdown: Some(shutdown_tx),
    listen,
  }
}
