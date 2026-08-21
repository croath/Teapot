//! Provider CLI backends and authentication.
//!
//! Each provider implements [`traits::Provider`] and stores its own auth struct
//! under a per-provider file `auth/{provider}.json` (see [`AuthEntry`] / [`ProviderKind`]).

pub mod antigravity;
pub mod auth_entry;
pub mod claude;
pub mod codex;
pub mod codex_cli;
pub mod compact;
pub mod execute;
pub mod kind;
pub mod model_info;
pub mod models_cache;
pub mod models_store;
pub mod pinned;
pub mod runtime;
pub mod traits;
pub mod vertex;
pub mod xai;

pub use antigravity::{AntigravityProvider, StoredAuth as AntigravityAuth};
pub use auth_entry::AuthEntry;
pub use claude::{ClaudeProvider, StoredAuth as ClaudeAuth};
pub use codex::{CodexProvider, StoredAuth as CodexAuth};
pub use codex_cli::{CodexCliModel, CodexCliProvider};
pub use compact::{ExecCompactRequest, ExecCompactResponse};
pub use execute::{ExecRequest, ExecResponse, ExecStream, ExecStreamEvent};
pub use kind::ProviderKind;
pub use model_info::{ModelInfo, ProviderModel};
pub use models_cache::{ModelsCache, NativeModelCatalog};
pub use models_store::{ModelsStore, default_models_dir};
pub use pinned::{PinnedProvider, pinned_provider};
pub use runtime::ProviderRuntime;
pub use traits::{
  PromptRequest, Provider, ProviderEvent, ProviderExecutor, ProviderSession, SpawnSpec,
  StdoutCodec, augmented_path, expand_args, flatten_messages, resolve_binary, stdin_prompt,
};
pub use vertex::VertexSession;
pub use vertex::{
  ImportOptions as VertexImportOptions, StoredAuth as VertexAuth, VertexProvider,
  import_service_account,
};
pub use xai::{StoredAuth as XaiAuth, XaiProvider};

use std::sync::Arc;

/// Providers offered in CLI and the desktop UI.
///
/// Hidden backends (`codex`, `claude`) remain in [`ProviderKind::ALL`] and
/// [`provider_for`].
pub fn all_providers() -> Vec<Arc<dyn ProviderAuth>> {
  offered_providers()
}

/// Same as [`all_providers`]: kinds listed in CLI pickers and the UI.
pub fn offered_providers() -> Vec<Arc<dyn ProviderAuth>> {
  ProviderKind::OFFERED
    .iter()
    .copied()
    .map(provider_for)
    .collect()
}

/// Instantiate the builtin provider for a kind (auth / CLI surface).
pub fn provider_for(kind: ProviderKind) -> Arc<dyn ProviderAuth> {
  match kind {
    ProviderKind::Codex => Arc::new(CodexProvider::new()),
    ProviderKind::CodexCli => Arc::new(CodexCliProvider::new()),
    ProviderKind::Claude => Arc::new(ClaudeProvider::new()),
    ProviderKind::Xai => Arc::new(XaiProvider::new()),
    ProviderKind::Antigravity => Arc::new(AntigravityProvider::new()),
    ProviderKind::Vertex => Arc::new(VertexProvider::new()),
  }
}

/// Resolve a config / CLI name (canonical id or alias) to a provider.
pub fn provider_by_name(name: &str) -> Option<Arc<dyn ProviderAuth>> {
  ProviderKind::parse(name).ok().map(provider_for)
}

/// Map a config / request name to a [`ProviderKind`].
pub fn family_for_name(name: &str) -> Option<ProviderKind> {
  ProviderKind::parse(name).ok()
}

/// Object-safe auth surface used by the CLI and registry.
pub trait ProviderAuth: Send + Sync {
  fn kind(&self) -> ProviderKind;
  fn id(&self) -> &str {
    self.kind().as_str()
  }
  fn description(&self) -> &str;
  fn command(&self) -> &str;
  fn is_installed(&self) -> bool;
  fn auth_method(&self) -> crate::auth::AuthMethod;
  fn supports_auth(&self) -> bool;

  fn load_auth(&self, store: &crate::auth::AuthStore) -> crate::error::AppResult<Vec<AuthEntry>>;

  fn save_auth(
    &self,
    store: &crate::auth::AuthStore,
    entry: &AuthEntry,
  ) -> crate::error::AppResult<std::path::PathBuf>;

  fn clear_auth(
    &self,
    store: &crate::auth::AuthStore,
    account: Option<&str>,
  ) -> crate::error::AppResult<usize>;

  fn login<'a>(
    &'a self,
    store: &'a crate::auth::AuthStore,
    opts: crate::auth::LoginOptions,
  ) -> std::pin::Pin<
    Box<dyn std::future::Future<Output = crate::error::AppResult<AuthEntry>> + Send + 'a>,
  >;

  fn refresh_auth<'a>(
    &'a self,
    store: &'a crate::auth::AuthStore,
    entry: &'a AuthEntry,
  ) -> std::pin::Pin<
    Box<dyn std::future::Future<Output = crate::error::AppResult<AuthEntry>> + Send + 'a>,
  >;

  fn ensure_auth<'a>(
    &'a self,
    store: &'a crate::auth::AuthStore,
  ) -> std::pin::Pin<
    Box<dyn std::future::Future<Output = crate::error::AppResult<AuthEntry>> + Send + 'a>,
  >;
}

impl<T: Provider + 'static> ProviderAuth for T {
  fn kind(&self) -> ProviderKind {
    Provider::kind(self)
  }

  fn description(&self) -> &str {
    Provider::description(self)
  }

  fn command(&self) -> &str {
    Provider::command(self)
  }

  fn is_installed(&self) -> bool {
    Provider::is_installed(self)
  }

  fn auth_method(&self) -> crate::auth::AuthMethod {
    Provider::auth_method(self)
  }

  fn supports_auth(&self) -> bool {
    Provider::supports_auth(self)
  }

  fn load_auth(&self, store: &crate::auth::AuthStore) -> crate::error::AppResult<Vec<AuthEntry>> {
    Provider::load_auth(self, store)
  }

  fn save_auth(
    &self,
    store: &crate::auth::AuthStore,
    entry: &AuthEntry,
  ) -> crate::error::AppResult<std::path::PathBuf> {
    Provider::save_auth(self, store, entry)
  }

  fn clear_auth(
    &self,
    store: &crate::auth::AuthStore,
    account: Option<&str>,
  ) -> crate::error::AppResult<usize> {
    Provider::clear_auth(self, store, account)
  }

  fn login<'a>(
    &'a self,
    store: &'a crate::auth::AuthStore,
    opts: crate::auth::LoginOptions,
  ) -> std::pin::Pin<
    Box<dyn std::future::Future<Output = crate::error::AppResult<AuthEntry>> + Send + 'a>,
  > {
    Box::pin(Provider::login(self, store, opts))
  }

  fn refresh_auth<'a>(
    &'a self,
    store: &'a crate::auth::AuthStore,
    entry: &'a AuthEntry,
  ) -> std::pin::Pin<
    Box<dyn std::future::Future<Output = crate::error::AppResult<AuthEntry>> + Send + 'a>,
  > {
    Box::pin(Provider::refresh_auth(self, store, entry))
  }

  fn ensure_auth<'a>(
    &'a self,
    store: &'a crate::auth::AuthStore,
  ) -> std::pin::Pin<
    Box<dyn std::future::Future<Output = crate::error::AppResult<AuthEntry>> + Send + 'a>,
  > {
    Box::pin(Provider::ensure_auth(self, store))
  }
}
