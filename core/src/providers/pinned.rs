//! Single provider instance pinned for a server process.
//!
//! Created once at bootstrap from the CLI/config provider name and reused for
//! auth, model listing, and chat execution (so HTTP clients stay warm).
//!
//! Each concrete provider owns its **own** in-memory credentials type
//! (`StoredAuth` or `VertexSession`) — there is no shared credential bag.

use std::sync::Arc;

use crate::auth::AuthStore;
use crate::error::AppResult;
use crate::providers::antigravity::AntigravityProvider;
use crate::providers::claude::ClaudeProvider;
use crate::providers::claude_cli::ClaudeCliProvider;
use crate::providers::codex::CodexProvider;
use crate::providers::codex_cli::CodexCliProvider;
use crate::providers::compact::{ExecCompactRequest, ExecCompactResponse};
use crate::providers::execute::{ExecRequest, ExecResponse, ExecStream};
use crate::providers::model_info::{ModelInfo, ProviderModel};
use crate::providers::models_cache::NativeModelCatalog;
use crate::providers::vertex::VertexProvider;
use crate::providers::xai::XaiProvider;
use crate::providers::{ProviderAuth, ProviderKind};

/// Concrete provider held for the lifetime of a pinned server process.
#[derive(Debug, Clone)]
pub enum PinnedProvider {
  Codex(CodexProvider),
  CodexCli(CodexCliProvider),
  Claude(ClaudeProvider),
  ClaudeCli(ClaudeCliProvider),
  Xai(XaiProvider),
  Antigravity(AntigravityProvider),
  Vertex(VertexProvider),
}

impl PinnedProvider {
  /// Build the builtin provider for `kind` (one instance per server).
  pub fn from_kind(kind: ProviderKind) -> Self {
    match kind {
      ProviderKind::Codex => Self::Codex(CodexProvider::new()),
      ProviderKind::CodexCli => Self::CodexCli(CodexCliProvider::new()),
      ProviderKind::Claude => Self::Claude(ClaudeProvider::new()),
      ProviderKind::ClaudeCli => Self::ClaudeCli(ClaudeCliProvider::new()),
      ProviderKind::Xai => Self::Xai(XaiProvider::new()),
      ProviderKind::Antigravity => Self::Antigravity(AntigravityProvider::new()),
      ProviderKind::Vertex => Self::Vertex(VertexProvider::new()),
    }
  }

  pub fn kind(&self) -> ProviderKind {
    match self {
      Self::Codex(_) => ProviderKind::Codex,
      Self::CodexCli(_) => ProviderKind::CodexCli,
      Self::Claude(_) => ProviderKind::Claude,
      Self::ClaudeCli(_) => ProviderKind::ClaudeCli,
      Self::Xai(_) => ProviderKind::Xai,
      Self::Antigravity(_) => ProviderKind::Antigravity,
      Self::Vertex(_) => ProviderKind::Vertex,
    }
  }

  /// Object-safe auth surface (login / ensure / refresh).
  pub fn as_auth(&self) -> &dyn ProviderAuth {
    match self {
      Self::Codex(p) => p,
      Self::CodexCli(p) => p,
      Self::Claude(p) => p,
      Self::ClaudeCli(p) => p,
      Self::Xai(p) => p,
      Self::Antigravity(p) => p,
      Self::Vertex(p) => p,
    }
  }

  /// Ensure credentials via this instance's auth implementation.
  pub async fn ensure_auth(&self, store: &AuthStore) -> AppResult<crate::providers::AuthEntry> {
    self.as_auth().ensure_auth(store).await
  }

  /// Whether this provider's **own** in-memory session needs a refresh.
  pub async fn session_needs_refresh(&self) -> bool {
    match self {
      Self::Codex(p) => p.session_needs_refresh().await,
      Self::CodexCli(p) => p.session_needs_refresh().await,
      Self::Claude(p) => p.session_needs_refresh().await,
      Self::ClaudeCli(p) => p.session_needs_refresh().await,
      Self::Xai(p) => p.session_needs_refresh().await,
      Self::Antigravity(p) => p.session_needs_refresh().await,
      Self::Vertex(p) => p.session_needs_refresh().await,
    }
  }

  /// Load / mint credentials into this provider's native in-memory session.
  pub async fn load_session(&self, store: &AuthStore) -> AppResult<()> {
    match self {
      Self::CodexCli(p) => {
        p.ensure_session().await?;
        return Ok(());
      }
      Self::ClaudeCli(p) => {
        p.ensure_session().await?;
        return Ok(());
      }
      _ => {}
    }
    let entry = self.ensure_auth(store).await?;
    match self {
      Self::CodexCli(_) | Self::ClaudeCli(_) => {}
      Self::Codex(p) => {
        p.set_session(entry.into_codex()?).await;
      }
      Self::Claude(p) => {
        p.set_session(entry.into_claude()?).await;
      }
      Self::Xai(p) => {
        p.set_session(entry.into_xai()?).await;
      }
      Self::Antigravity(p) => {
        p.set_session(entry.into_antigravity()?).await;
      }
      Self::Vertex(p) => {
        let stored = entry.into_vertex()?;
        let session = p.session_from_stored(&stored).await?;
        p.set_session(session).await;
      }
    }
    Ok(())
  }

  /// Refresh session if missing or near expiry.
  pub async fn refresh_session_if_needed(&self, store: &AuthStore) -> AppResult<()> {
    if self.session_needs_refresh().await {
      self.load_session(store).await?;
    }
    Ok(())
  }

  /// Upstream chat execute using the provider's own session credentials.
  pub async fn execute(&self, req: &ExecRequest) -> AppResult<ExecResponse> {
    match self {
      Self::Codex(p) => p.execute(req).await,
      Self::CodexCli(p) => p.execute(req).await,
      Self::Claude(p) => p.execute(req).await,
      Self::ClaudeCli(p) => p.execute(req).await,
      Self::Xai(p) => p.execute(req).await,
      Self::Antigravity(p) => p.execute(req).await,
      Self::Vertex(p) => p.execute(req).await,
    }
  }

  /// Upstream streaming execute (SSE deltas as provider [`ExecStream`] events).
  pub async fn execute_stream(&self, req: &ExecRequest) -> AppResult<ExecStream> {
    match self {
      Self::Codex(p) => p.execute_stream(req).await,
      Self::CodexCli(p) => p.execute_stream(req).await,
      Self::Claude(p) => p.execute_stream(req).await,
      Self::ClaudeCli(p) => p.execute_stream(req).await,
      Self::Xai(p) => p.execute_stream(req).await,
      Self::Antigravity(p) => p.execute_stream(req).await,
      Self::Vertex(p) => p.execute_stream(req).await,
    }
  }

  /// Compact conversation history (native `/responses/compact` or summarize workaround).
  pub async fn execute_compact(&self, req: &ExecCompactRequest) -> AppResult<ExecCompactResponse> {
    match self {
      Self::Codex(p) => p.execute_compact(req).await,
      Self::CodexCli(p) => p.execute_compact(req).await,
      Self::Claude(p) => p.execute_compact(req).await,
      Self::ClaudeCli(p) => p.execute_compact(req).await,
      Self::Xai(p) => p.execute_compact(req).await,
      Self::Antigravity(p) => p.execute_compact(req).await,
      Self::Vertex(p) => p.execute_compact(req).await,
    }
  }

  /// Streaming compact (deltas of summary / content, then done).
  pub async fn execute_compact_stream(&self, req: &ExecCompactRequest) -> AppResult<ExecStream> {
    match self {
      Self::Codex(p) => p.execute_compact_stream(req).await,
      Self::CodexCli(p) => p.execute_compact_stream(req).await,
      Self::Claude(p) => p.execute_compact_stream(req).await,
      Self::ClaudeCli(p) => p.execute_compact_stream(req).await,
      Self::Xai(p) => p.execute_compact_stream(req).await,
      Self::Antigravity(p) => p.execute_compact_stream(req).await,
      Self::Vertex(p) => p.execute_compact_stream(req).await,
    }
  }

  /// Fetch the native model catalog via this provider's own session.
  pub async fn fetch_models(&self) -> AppResult<NativeModelCatalog> {
    Ok(match self {
      Self::Codex(p) => NativeModelCatalog::Codex(p.models().await?),
      Self::CodexCli(p) => NativeModelCatalog::CodexCli(p.models().await?),
      Self::Claude(p) => NativeModelCatalog::Claude(p.models().await?),
      Self::ClaudeCli(p) => NativeModelCatalog::ClaudeCli(p.models().await?),
      Self::Xai(p) => NativeModelCatalog::Xai(p.models().await?),
      Self::Antigravity(p) => NativeModelCatalog::Antigravity(p.models().await?),
      Self::Vertex(p) => NativeModelCatalog::Vertex(p.models().await?),
    })
  }

  /// List models from the provider's upstream `models` API (converted to [`ModelInfo`]).
  pub async fn models(&self) -> AppResult<Vec<ModelInfo>> {
    Ok(self.fetch_models().await?.to_model_infos())
  }

  /// Retrieve one model via the provider's upstream `model` API (or list lookup).
  pub async fn model(&self, id: &str) -> AppResult<Option<ModelInfo>> {
    Ok(match self {
      Self::Codex(p) => p.model(id).await?.map(|m| m.to_model_info()),
      Self::CodexCli(p) => p.model(id).await?.map(|m| m.to_model_info()),
      Self::Claude(p) => p.model(id).await?.map(|m| m.to_model_info()),
      Self::ClaudeCli(p) => p.model(id).await?.map(|m| m.to_model_info()),
      Self::Xai(p) => p.model(id).await?.map(|m| m.to_model_info()),
      Self::Antigravity(p) => p.model(id).await?.map(|m| m.to_model_info()),
      Self::Vertex(p) => p.model(id).await?.map(|m| m.to_model_info()),
    })
  }
}

/// Convenience constructor used at server bootstrap.
pub fn pinned_provider(kind: ProviderKind) -> Arc<PinnedProvider> {
  Arc::new(PinnedProvider::from_kind(kind))
}
