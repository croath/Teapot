//! Trait-based API endpoint surfaces (ChatGPT, Claude, …).

use axum::Router;

use super::chatgpt;
use super::claude;
use super::state::AppState;

/// A mountable HTTP API surface under a path prefix.
///
/// Implementations own their route tables; the server nests them via
/// [`mount_surfaces`].
pub trait ApiSurface: Send + Sync {
  /// Short name for logs (e.g. `chatgpt`, `claude`).
  fn name(&self) -> &'static str;

  /// URL path prefix (e.g. `/chatgpt/v1`).
  fn prefix(&self) -> &'static str;

  /// Routes for this surface (relative to [`Self::prefix`]).
  fn routes(&self) -> Router<AppState>;
}

/// OpenAI / ChatGPT compatible API under `/chatgpt/v1`.
///
/// Includes chat completions, full Responses API, and OpenAI-compatible `GET /models`.
#[derive(Debug, Default, Clone, Copy)]
pub struct ChatGptSurface;

impl ApiSurface for ChatGptSurface {
  fn name(&self) -> &'static str {
    "chatgpt"
  }

  fn prefix(&self) -> &'static str {
    "/chatgpt/v1"
  }

  fn routes(&self) -> Router<AppState> {
    chatgpt::router()
  }
}

/// Anthropic / Claude compatible API under `/claude/v1`.
///
/// Includes messages and Anthropic-compatible `GET /models`.
#[derive(Debug, Default, Clone, Copy)]
pub struct ClaudeSurface;

impl ApiSurface for ClaudeSurface {
  fn name(&self) -> &'static str {
    "claude"
  }

  fn prefix(&self) -> &'static str {
    "/claude/v1"
  }

  fn routes(&self) -> Router<AppState> {
    claude::router()
  }
}

/// All default API surfaces as owned trait objects.
pub fn default_surface_list() -> Vec<Box<dyn ApiSurface>> {
  vec![Box::new(ChatGptSurface), Box::new(ClaudeSurface)]
}

/// Nest every surface onto `router` under its prefix.
pub fn mount_surfaces(
  mut router: Router<AppState>,
  surfaces: &[Box<dyn ApiSurface>],
) -> Router<AppState> {
  for surface in surfaces {
    tracing::debug!(
      surface = surface.name(),
      prefix = surface.prefix(),
      "mounting API surface"
    );
    router = router.nest(surface.prefix(), surface.routes());
  }
  router
}
