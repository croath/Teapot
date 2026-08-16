//! Lightweight Leptos CSR i18n: locale context, persistence, and typed copy.

mod locale;
mod msg;

use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::tauri_bridge::{self, SetLocaleArgs};

pub use locale::Locale;
pub use msg::Msg;

const STORAGE_KEY: &str = "teapot.locale";

/// Reactive locale handle provided at the app root.
#[derive(Clone, Copy)]
pub struct I18n {
  locale: RwSignal<Locale>,
}

impl I18n {
  pub fn locale(self) -> RwSignal<Locale> {
    self.locale
  }

  pub fn t(self, msg: Msg) -> Signal<&'static str> {
    let locale = self.locale;
    Signal::derive(move || locale.get().t(msg))
  }

  pub fn set(self, next: Locale) {
    if self.locale.get_untracked() == next {
      persist(next);
      apply_document_lang(next);
      sync_tauri_locale(next);
      return;
    }
    self.locale.set(next);
    persist(next);
    apply_document_lang(next);
    sync_tauri_locale(next);
  }
}

pub fn provide_i18n() {
  let initial = resolve_initial_locale();
  let i18n = I18n {
    locale: RwSignal::new(initial),
  };
  apply_document_lang(initial);
  persist(initial);
  sync_tauri_locale(initial);
  provide_context(i18n);
}

pub fn use_i18n() -> I18n {
  expect_context::<I18n>()
}

fn resolve_initial_locale() -> Locale {
  if let Some(stored) = load_stored() {
    return stored;
  }
  detect_navigator()
}

fn load_stored() -> Option<Locale> {
  let storage = web_sys::window()?.local_storage().ok().flatten()?;
  let value = storage.get_item(STORAGE_KEY).ok().flatten()?;
  Locale::parse(&value)
}

fn persist(locale: Locale) {
  let Some(window) = web_sys::window() else {
    return;
  };
  let Ok(Some(storage)) = window.local_storage() else {
    return;
  };
  let _ = storage.set_item(STORAGE_KEY, locale.id());
}

fn detect_navigator() -> Locale {
  let Some(window) = web_sys::window() else {
    return Locale::En;
  };
  match window.navigator().language() {
    Some(tag) if !tag.is_empty() => Locale::from_tag(&tag),
    _ => Locale::En,
  }
}

fn apply_document_lang(locale: Locale) {
  let Some(window) = web_sys::window() else {
    return;
  };
  let Some(document) = window.document() else {
    return;
  };
  let Some(root) = document.document_element() else {
    return;
  };
  let _ = root.set_attribute("lang", locale.html_lang());
}

fn sync_tauri_locale(locale: Locale) {
  if !tauri_bridge::is_tauri() {
    return;
  }
  spawn_local(async move {
    let _ = tauri_bridge::invoke::<(), _>(
      "set_locale",
      SetLocaleArgs {
        locale: locale.id().to_string(),
      },
    )
    .await;
  });
}
