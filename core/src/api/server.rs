//! Axum server bootstrap.

use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum::middleware;
use axum::routing::get;
use tokio::sync::oneshot;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing::info;

use super::auth::require_api_key;
use super::state::AppState;
use super::surface::{ApiSurface, default_surface_list, mount_surfaces};
use crate::auth::AuthStore;
use crate::config::Config;
use crate::error::AppError;
use crate::providers::{PinnedProvider, ProviderKind, ProviderRuntime};

/// Build the full application router from default API surfaces.
pub fn build_router(state: AppState) -> Router {
  build_router_with_surfaces(state, &default_surface_list())
}

/// Build the application router with an explicit set of [`ApiSurface`]s.
pub fn build_router_with_surfaces(state: AppState, surfaces: &[Box<dyn ApiSurface>]) -> Router {
  let api = mount_surfaces(Router::new(), surfaces).layer(middleware::from_fn_with_state(
    state.clone(),
    require_api_key,
  ));

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

#[derive(serde::Serialize)]
struct HealthBody {
  status: &'static str,
  service: &'static str,
}

async fn health() -> axum::Json<HealthBody> {
  axum::Json(HealthBody {
    status: "ok",
    service: "teapot",
  })
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
  let provider_name = config.provider.clone().ok_or_else(|| {
    anyhow::anyhow!("provider is required; start with `teapotx serve -p <provider>`")
  })?;
  let kind = ProviderKind::parse(&provider_name).map_err(|e| anyhow::anyhow!("{e}"))?;

  let auth_store =
    Arc::new(AuthStore::local().map_err(|e| anyhow::anyhow!("open auth store: {e}"))?);

  // Create the provider once from the CLI/config pin and share it via AppState.
  let provider = Arc::new(PinnedProvider::from_kind(kind));
  info!(provider = %kind, "bootstrapping access_token into memory");
  let runtime = ProviderRuntime::bootstrap_with_provider(provider.clone(), auth_store.clone())
    .await
    .map_err(|e| anyhow::anyhow!("provider bootstrap failed: {e}"))?;

  let models = runtime.models().list().await;
  info!(provider = %kind, models = models.len(), "models ready");

  let state = AppState::new(config, auth_store, provider, runtime);
  let surfaces = default_surface_list();
  let app = build_router_with_surfaces(state, &surfaces);

  let addr: SocketAddr = listen
    .parse()
    .map_err(|e| anyhow::anyhow!("invalid listen address {listen}: {e}"))?;

  info!(%addr, "Teapot listening");
  info!(provider = %kind, "pinned provider instance ready");
  for surface in &surfaces {
    info!(
      surface = surface.name(),
      prefix = surface.prefix(),
      "API surface ready"
    );
  }
  info!("ChatGPT API:  http://{addr}/chatgpt/v1/chat/completions");
  info!(
    "Responses API: http://{addr}/chatgpt/v1/responses (+ get/delete/cancel/input_items/compact/input_tokens)"
  );
  info!("Models API:   http://{addr}/chatgpt/v1/models");
  info!("Claude API:   http://{addr}/claude/v1/messages");
  info!("Claude models: http://{addr}/claude/v1/models");

  let listener = tokio::net::TcpListener::bind(addr).await?;
  axum::serve(listener, app)
    .with_graceful_shutdown(shutdown)
    .await?;
  info!(%addr, "Teapot stopped");
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

/// Convert [`AppError`] into anyhow for bootstrap call sites that need it.
#[allow(dead_code)]
fn app_err(e: AppError) -> anyhow::Error {
  anyhow::anyhow!("{e}")
}
