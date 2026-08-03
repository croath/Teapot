//! In-memory + disk models catalog for one pinned provider.
//!
//! Owns the provider-native catalog ([`NativeModelCatalog`]), the on-disk
//! [`ModelsStore`] file for that provider, and load/fetch helpers.
//! Independent from session/auth runtime.

use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::RwLock;
use tracing::info;

use crate::error::{AppError, AppResult};
use crate::providers::ProviderKind;
use crate::providers::antigravity::AntigravityModel;
use crate::providers::claude::ClaudeModel;
use crate::providers::codex::CodexModel;
use crate::providers::model_info::{ModelInfo, find_model_info, to_model_infos};
use crate::providers::models_store::ModelsStore;
use crate::providers::pinned::PinnedProvider;
use crate::providers::vertex::VertexModel;
use crate::providers::xai::XaiModel;

/// Provider-native model catalog kept in memory and on disk.
#[derive(Debug, Clone)]
pub enum NativeModelCatalog {
  Codex(Vec<CodexModel>),
  Claude(Vec<ClaudeModel>),
  Xai(Vec<XaiModel>),
  Antigravity(Vec<AntigravityModel>),
  Vertex(Vec<VertexModel>),
}

impl NativeModelCatalog {
  pub fn kind(&self) -> ProviderKind {
    match self {
      Self::Codex(_) => ProviderKind::Codex,
      Self::Claude(_) => ProviderKind::Claude,
      Self::Xai(_) => ProviderKind::Xai,
      Self::Antigravity(_) => ProviderKind::Antigravity,
      Self::Vertex(_) => ProviderKind::Vertex,
    }
  }

  pub fn is_empty(&self) -> bool {
    match self {
      Self::Codex(v) => v.is_empty(),
      Self::Claude(v) => v.is_empty(),
      Self::Xai(v) => v.is_empty(),
      Self::Antigravity(v) => v.is_empty(),
      Self::Vertex(v) => v.is_empty(),
    }
  }

  pub fn len(&self) -> usize {
    match self {
      Self::Codex(v) => v.len(),
      Self::Claude(v) => v.len(),
      Self::Xai(v) => v.len(),
      Self::Antigravity(v) => v.len(),
      Self::Vertex(v) => v.len(),
    }
  }

  /// Convert native entries to shared [`ModelInfo`] for API / validation.
  pub fn to_model_infos(&self) -> Vec<ModelInfo> {
    match self {
      Self::Codex(v) => to_model_infos(v),
      Self::Claude(v) => to_model_infos(v),
      Self::Xai(v) => to_model_infos(v),
      Self::Antigravity(v) => to_model_infos(v),
      Self::Vertex(v) => to_model_infos(v),
    }
  }

  pub fn find_model_info(&self, id: &str) -> Option<ModelInfo> {
    match self {
      Self::Codex(v) => find_model_info(v, id),
      Self::Claude(v) => find_model_info(v, id),
      Self::Xai(v) => find_model_info(v, id),
      Self::Antigravity(v) => find_model_info(v, id),
      Self::Vertex(v) => find_model_info(v, id),
    }
  }
}

/// Models cache for a single pinned provider.
///
/// - Disk: only `{kind}.json` via [`ModelsStore`]
/// - Memory: one [`NativeModelCatalog`] for this kind
/// - Fetch: uses the pinned [`PinnedProvider`] session (caller must refresh auth)
pub struct ModelsCache {
  kind: ProviderKind,
  store: ModelsStore,
  cache: RwLock<Option<NativeModelCatalog>>,
}

impl ModelsCache {
  /// Open the default models directory for `kind`.
  pub fn local(kind: ProviderKind) -> AppResult<Self> {
    Ok(Self {
      kind,
      store: ModelsStore::local()?,
      cache: RwLock::new(None),
    })
  }

  /// Construct with an explicit disk store (tests / custom data dir).
  pub fn new(kind: ProviderKind, store: ModelsStore) -> Self {
    Self {
      kind,
      store,
      cache: RwLock::new(None),
    }
  }

  pub fn kind(&self) -> ProviderKind {
    self.kind
  }

  /// Path of this provider's models file.
  pub fn path(&self) -> PathBuf {
    self.store.provider_path(self.kind)
  }

  /// Underlying per-provider disk store.
  pub fn store(&self) -> &ModelsStore {
    &self.store
  }

  /// Native catalog snapshot.
  pub async fn native(&self) -> Option<NativeModelCatalog> {
    self.cache.read().await.clone()
  }

  /// Shared [`ModelInfo`] list (converted from native storage on demand).
  pub async fn list(&self) -> Vec<ModelInfo> {
    self
      .cache
      .read()
      .await
      .as_ref()
      .map(|c| c.to_model_infos())
      .unwrap_or_default()
  }

  pub async fn get(&self, id: &str) -> Option<ModelInfo> {
    self
      .cache
      .read()
      .await
      .as_ref()
      .and_then(|c| c.find_model_info(id))
  }

  pub async fn require(&self, id: &str) -> AppResult<ModelInfo> {
    self.get(id).await.ok_or_else(|| {
      AppError::BadRequest(format!(
        "model `{id}` is not supported by provider `{}`",
        self.kind
      ))
    })
  }

  /// Load from this provider's file if present; otherwise fetch, save, cache.
  pub async fn load_or_fetch(&self, provider: &PinnedProvider) -> AppResult<Vec<ModelInfo>> {
    if provider.kind() != self.kind {
      return Err(AppError::Internal(format!(
        "models cache is for `{}` but provider is `{}`",
        self.kind,
        provider.kind()
      )));
    }
    if let Some(catalog) = self.load_from_disk()? {
      let infos = catalog.to_model_infos();
      let count = catalog.len();
      let path = self.path();
      {
        let mut guard = self.cache.write().await;
        *guard = Some(catalog);
      }
      info!(
        provider = %self.kind,
        count,
        path = %path.display(),
        "native models loaded from provider cache file"
      );
      return Ok(infos);
    }
    self.fetch_and_store(provider).await
  }

  /// Fetch from upstream via `provider`, write only this provider's file, update memory.
  pub async fn fetch_and_store(&self, provider: &PinnedProvider) -> AppResult<Vec<ModelInfo>> {
    if provider.kind() != self.kind {
      return Err(AppError::Internal(format!(
        "models cache is for `{}` but provider is `{}`",
        self.kind,
        provider.kind()
      )));
    }
    let catalog = provider.fetch_models().await?;
    if catalog.is_empty() {
      return Err(AppError::ProviderFailed(format!(
        "{} models: empty catalog from upstream",
        self.kind
      )));
    }
    let path = self.save_to_disk(&catalog)?;
    let infos = catalog.to_model_infos();
    let count = catalog.len();
    {
      let mut guard = self.cache.write().await;
      *guard = Some(catalog);
    }
    info!(
      provider = %self.kind,
      count,
      path = %path.display(),
      "native models fetched and saved for provider"
    );
    Ok(infos)
  }

  /// Replace in-memory catalog (does not write disk).
  pub async fn set_memory(&self, catalog: NativeModelCatalog) -> AppResult<()> {
    if catalog.kind() != self.kind {
      return Err(AppError::Internal(format!(
        "refusing to cache {} models in `{}` ModelsCache",
        catalog.kind(),
        self.kind
      )));
    }
    let mut guard = self.cache.write().await;
    *guard = Some(catalog);
    Ok(())
  }

  fn load_from_disk(&self) -> AppResult<Option<NativeModelCatalog>> {
    let kind = self.kind;
    Ok(match kind {
      ProviderKind::Codex => self
        .store
        .load_models::<CodexModel>(kind)?
        .map(NativeModelCatalog::Codex),
      ProviderKind::Claude => self
        .store
        .load_models::<ClaudeModel>(kind)?
        .map(NativeModelCatalog::Claude),
      ProviderKind::Xai => self
        .store
        .load_models::<XaiModel>(kind)?
        .map(NativeModelCatalog::Xai),
      ProviderKind::Antigravity => self
        .store
        .load_models::<AntigravityModel>(kind)?
        .map(NativeModelCatalog::Antigravity),
      ProviderKind::Vertex => self
        .store
        .load_models::<VertexModel>(kind)?
        .map(NativeModelCatalog::Vertex),
    })
  }

  fn save_to_disk(&self, catalog: &NativeModelCatalog) -> AppResult<PathBuf> {
    if catalog.kind() != self.kind {
      return Err(AppError::Internal(format!(
        "refusing to save {} models while cache is for {}",
        catalog.kind(),
        self.kind
      )));
    }
    match catalog {
      NativeModelCatalog::Codex(m) => self.store.save_models(ProviderKind::Codex, m),
      NativeModelCatalog::Claude(m) => self.store.save_models(ProviderKind::Claude, m),
      NativeModelCatalog::Xai(m) => self.store.save_models(ProviderKind::Xai, m),
      NativeModelCatalog::Antigravity(m) => self.store.save_models(ProviderKind::Antigravity, m),
      NativeModelCatalog::Vertex(m) => self.store.save_models(ProviderKind::Vertex, m),
    }
  }
}

/// Shared handle used by API state / runtime.
pub type SharedModelsCache = Arc<ModelsCache>;
