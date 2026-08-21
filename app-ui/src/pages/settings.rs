//! macOS-style Settings: hub + General / Debug / About / Language panes.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use icons::{Bug, Check, ChevronRight, Info, Languages, ServerCog};
use leptos::prelude::*;
use leptos_router::components::A;
use wasm_bindgen_futures::spawn_local;

use crate::components::{Button, ButtonVariant, Input, InputType, Label, Progress};
use crate::i18n::{Locale, Msg, use_i18n};
use crate::tauri_bridge::{
  self, AppConfigDto, AppInfo, DownloadProgress, SaveConfigArgs, UpdateCheck,
};

// ─── Hub ───────────────────────────────────────────────────────────────────

/// Settings root: category list (macOS System Settings feel).
#[component]
pub fn SettingsPage() -> impl IntoView {
  let i18n = use_i18n();
  view! {
    <div class="settings-shell">
      <header class="settings-hero">
        <div class="settings-done">
          <A href="/">{i18n.t(Msg::Done)}</A>
        </div>
        <h1 class="settings-large-title">{i18n.t(Msg::Settings)}</h1>
        <p class="settings-hero-sub">{i18n.t(Msg::SettingsHeroSub)}</p>
      </header>

      <section class="settings-group" aria-label=move || i18n.t(Msg::SettingsCategories).get()>
        <SettingsRow
          href="/settings/generate"
          title=i18n.t(Msg::General)
          subtitle=i18n.t(Msg::GeneralSub)
          accent="generate"
        >
          <ServerCog class="size-5".to_string() />
        </SettingsRow>
        <SettingsRow
          href="/settings/debug"
          title=i18n.t(Msg::Debug)
          subtitle=i18n.t(Msg::DebugSub)
          accent="debug"
        >
          <Bug class="size-5".to_string() />
        </SettingsRow>
        <SettingsRow
          href="/settings/about"
          title=i18n.t(Msg::About)
          subtitle=i18n.t(Msg::AboutSub)
          accent="about"
        >
          <Info class="size-5".to_string() />
        </SettingsRow>
      </section>

      <section class="settings-group" aria-label=move || i18n.t(Msg::Language).get()>
        <SettingsRow
          href="/settings/language"
          title=i18n.t(Msg::Language)
          subtitle=Signal::derive(move || i18n.locale().get().native_name())
          accent="language"
        >
          <Languages class="size-5".to_string() />
        </SettingsRow>
      </section>
    </div>
  }
}

#[component]
fn SettingsRow(
  href: &'static str,
  #[prop(into)] title: Signal<&'static str>,
  #[prop(into)] subtitle: Signal<&'static str>,
  accent: &'static str,
  children: Children,
) -> impl IntoView {
  let icon_class = format!("settings-row-icon settings-row-icon-{accent}");
  view! {
    <A href=href>
      <span class=icon_class aria-hidden="true">
        {children()}
      </span>
      <span class="settings-row-text">
        <span class="settings-row-title">{move || title.get()}</span>
        <span class="settings-row-sub">{move || subtitle.get()}</span>
      </span>
      <ChevronRight class="settings-row-chevron size-4".to_string() />
    </A>
  }
}

// ─── Shared chrome for detail panes ────────────────────────────────────────

#[component]
fn SettingsDetail(
  #[prop(into)] title: Signal<&'static str>,
  #[prop(optional)] subtitle: Option<Signal<&'static str>>,
  children: Children,
) -> impl IntoView {
  let i18n = use_i18n();
  let has_sub = subtitle.is_some();
  view! {
    <div class="settings-shell settings-shell-detail">
      <header class="settings-detail-bar">
        <div class="settings-back">
          <A href="/settings">
            <span class="settings-back-chevron" aria-hidden="true">"‹"</span>
            <span>{i18n.t(Msg::Settings)}</span>
          </A>
        </div>
        <h1 class="settings-detail-title">{move || title.get()}</h1>
        <Show when=move || has_sub>
          <p class="settings-detail-sub">{move || subtitle.map(|s| s.get()).unwrap_or("")}</p>
        </Show>
      </header>
      <div class="settings-detail-body">
        {children()}
      </div>
    </div>
  }
}

// ─── General ───────────────────────────────────────────────────────────────

#[component]
pub fn SettingsGeneratePage() -> impl IntoView {
  let i18n = use_i18n();
  let listen = RwSignal::new("127.0.0.1:8080".to_string());
  let api_key = RwSignal::new(String::new());
  let provider = RwSignal::new("codex-cli".to_string());
  let status = RwSignal::new(Option::<Msg>::None);
  let error = RwSignal::new(Option::<String>::None);
  let saving = RwSignal::new(false);
  let in_tauri = tauri_bridge::is_tauri();

  Effect::new(move |_| {
    if !in_tauri {
      return;
    }
    spawn_local(async move {
      match tauri_bridge::invoke0::<AppConfigDto>("get_config").await {
        Ok(cfg) => {
          listen.set(cfg.listen);
          api_key.set(cfg.api_key);
          if !cfg.provider.trim().is_empty() {
            provider.set(cfg.provider);
          }
        }
        Err(e) => error.set(Some(e)),
      }
    });
  });

  let on_save = move |_| {
    if !in_tauri {
      error.set(Some(
        i18n.locale().get_untracked().t(Msg::DesktopOnlySave).into(),
      ));
      return;
    }
    if saving.get_untracked() {
      return;
    }
    saving.set(true);
    error.set(None);
    status.set(None);
    let cfg = AppConfigDto {
      listen: listen.get_untracked(),
      api_key: api_key.get_untracked(),
      provider: provider.get_untracked(),
    };
    spawn_local(async move {
      match tauri_bridge::invoke::<(), _>("save_config", SaveConfigArgs { config: cfg }).await {
        Ok(()) => status.set(Some(Msg::ConfigSaved)),
        Err(e) => error.set(Some(e)),
      }
      saving.set(false);
    });
  };

  view! {
    <SettingsDetail title=i18n.t(Msg::General) subtitle=i18n.t(Msg::GeneralDetailSub)>
      <div class="settings-group settings-group-form">
        <div class="settings-field">
          <Label>{i18n.t(Msg::ListenAddress)}</Label>
          <Input
            r#type=InputType::Text
            bind_value=listen
            class="font-mono settings-input"
            placeholder="127.0.0.1:8080"
          />
          <p class="field-hint">{i18n.t(Msg::ListenHint)}</p>
        </div>
        <div class="settings-field">
          <Label>{i18n.t(Msg::ApiKey)}</Label>
          <Input
            r#type=InputType::Password
            bind_value=api_key
            class="font-mono settings-input"
            placeholder=Signal::derive(move || i18n.t(Msg::ApiKeyPlaceholder).get().to_string())
          />
          <p class="field-hint">
            {i18n.t(Msg::ApiKeyHint)}
          </p>
        </div>
      </div>

      <div class="settings-footer-actions">
        <Button
          variant=ButtonVariant::Default
          on:click=on_save
          prop:disabled=move || saving.get() || !in_tauri
          class="settings-primary-btn"
        >
          {move || {
            if saving.get() {
              i18n.locale().get().t(Msg::Saving)
            } else {
              i18n.locale().get().t(Msg::SaveConfig)
            }
          }}
        </Button>
      </div>

      <Show when=move || status.get().is_some()>
        <p class="form-ok">
          {move || status.get().map(|m| i18n.locale().get().t(m)).unwrap_or_default()}
        </p>
      </Show>
      <Show when=move || error.get().is_some()>
        <p class="home-error">{move || error.get().unwrap_or_default()}</p>
      </Show>
    </SettingsDetail>
  }
}

// ─── Debug ─────────────────────────────────────────────────────────────────

#[component]
pub fn SettingsDebugPage() -> impl IntoView {
  let i18n = use_i18n();
  let logs = RwSignal::new(String::new());
  let error = RwSignal::new(Option::<String>::None);
  let in_tauri = tauri_bridge::is_tauri();

  let refresh = move || {
    if !in_tauri {
      return;
    }
    spawn_local(async move {
      match tauri_bridge::invoke0::<String>("get_logs").await {
        Ok(text) => {
          logs.set(text);
          error.set(None);
        }
        Err(e) => error.set(Some(e)),
      }
    });
  };

  Effect::new(move |_| {
    refresh();
    let cancelled = Arc::new(AtomicBool::new(false));
    let cancelled_flag = cancelled.clone();
    spawn_local(async move {
      loop {
        gloo_timers::future::TimeoutFuture::new(800).await;
        if cancelled_flag.load(Ordering::Relaxed) {
          break;
        }
        refresh();
      }
    });
    on_cleanup(move || {
      cancelled.store(true, Ordering::Relaxed);
    });
  });

  let on_export = move |_| {
    let content = logs.get_untracked();
    if content.is_empty() {
      error.set(Some(
        i18n.locale().get_untracked().t(Msg::NothingToExport).into(),
      ));
      return;
    }
    let stamp = js_sys::Date::new_0()
      .to_iso_string()
      .as_string()
      .unwrap_or_else(|| "export".into())
      .replace(':', "-");
    let name = format!("teapotx-logs-{stamp}.txt");
    if let Err(e) = tauri_bridge::download_text(&name, &content) {
      error.set(Some(e));
    }
  };

  let on_clear = move |_| {
    if !in_tauri {
      return;
    }
    spawn_local(async move {
      let _ = tauri_bridge::invoke0::<()>("clear_logs").await;
      logs.set(String::new());
    });
  };

  view! {
    <SettingsDetail title=i18n.t(Msg::Debug) subtitle=i18n.t(Msg::DebugDetailSub)>
      <div class="settings-toolbar">
        <Button variant=ButtonVariant::Outline on:click=on_clear class="settings-chip-btn">
          {i18n.t(Msg::Clear)}
        </Button>
        <Button variant=ButtonVariant::Default on:click=on_export class="settings-chip-btn">
          {i18n.t(Msg::Export)}
        </Button>
      </div>

      <div class="settings-group settings-log-card">
        <pre class="log-view">
          {move || {
            if !in_tauri {
              return i18n.locale().get().t(Msg::LogsDesktopOnly).to_string();
            }
            let text = logs.get();
            if text.is_empty() {
              i18n.locale().get().t(Msg::NoLogs).to_string()
            } else {
              text
            }
          }}
        </pre>
      </div>

      <Show when=move || error.get().is_some()>
        <p class="home-error">{move || error.get().unwrap_or_default()}</p>
      </Show>
    </SettingsDetail>
  }
}

// ─── About ─────────────────────────────────────────────────────────────────

#[component]
pub fn SettingsAboutPage() -> impl IntoView {
  let i18n = use_i18n();
  let version = RwSignal::new(env!("CARGO_PKG_VERSION").to_string());
  let name = RwSignal::new("Teapot".to_string());
  let in_tauri = tauri_bridge::is_tauri();
  let checking = RwSignal::new(false);
  let installing = RwSignal::new(false);
  let update = RwSignal::new(Option::<UpdateCheck>::None);
  let error = RwSignal::new(Option::<String>::None);
  let downloaded = RwSignal::new(0u64);
  let content_length = RwSignal::new(Option::<u64>::None);

  Effect::new(move |_| {
    if !in_tauri {
      return;
    }
    spawn_local(async move {
      if let Ok(info) = tauri_bridge::invoke0::<AppInfo>("get_app_info").await {
        name.set(info.name);
        version.set(info.version);
      }
    });
  });

  let run_check = move || {
    if !in_tauri || checking.get_untracked() || installing.get_untracked() {
      return;
    }
    checking.set(true);
    error.set(None);
    spawn_local(async move {
      match tauri_bridge::invoke0::<UpdateCheck>("check_for_update").await {
        Ok(result) => {
          version.set(result.current_version.clone());
          update.set(Some(result));
        }
        Err(e) => {
          update.set(None);
          error.set(Some(e));
        }
      }
      checking.set(false);
    });
  };

  Effect::new(move |_| {
    if !in_tauri {
      return;
    }
    run_check();
    tauri_bridge::listen("updater-check", move || run_check());
    tauri_bridge::listen_json::<DownloadProgress>("updater-progress", move |progress| {
      downloaded.set(progress.downloaded);
      content_length.set(progress.content_length);
    });
    tauri_bridge::listen("updater-finished", move || {
      installing.set(true);
    });
  });

  let on_check = move |_| run_check();

  let on_install = move |_| {
    if !in_tauri || installing.get_untracked() {
      return;
    }
    installing.set(true);
    error.set(None);
    downloaded.set(0);
    content_length.set(None);
    spawn_local(async move {
      if let Err(e) = tauri_bridge::invoke0::<()>("install_update").await {
        error.set(Some(e));
        installing.set(false);
      }
    });
  };

  let progress_pct = Signal::derive(move || match content_length.get() {
    Some(total) if total > 0 => (downloaded.get() as f64 / total as f64) * 100.0,
    _ => 0.0,
  });

  let available = Signal::derive(move || update.get().map(|u| u.available).unwrap_or(false));

  view! {
    <SettingsDetail title=i18n.t(Msg::About)>
      <div class="settings-about">
        <div class="settings-about-hero">
          <div class="settings-about-glow" aria-hidden="true"></div>
          <img
            src="/app-icon.png"
            alt="Teapot"
            class="settings-about-icon"
            width="96"
            height="96"
          />
          <h2 class="settings-about-name">{move || name.get()}</h2>
          <p class="settings-about-version">
            {i18n.t(Msg::Version)}
            " "
            <span class="font-mono">{move || version.get()}</span>
          </p>
        </div>

        <div class="settings-group settings-updater">
          <Show when=move || !in_tauri>
            <p class="settings-updater-status">{i18n.t(Msg::UpdateDesktopOnly)}</p>
          </Show>
          <Show when=move || in_tauri>
            <p class="settings-updater-status">
              {move || {
                let locale = i18n.locale().get();
                if installing.get() {
                  if downloaded.get() > 0 || content_length.get().is_some() {
                    locale.t(Msg::DownloadingUpdate)
                  } else {
                    locale.t(Msg::InstallingUpdate)
                  }
                } else if checking.get() {
                  locale.t(Msg::CheckingForUpdates)
                } else if let Some(info) = update.get() {
                  if info.available {
                    locale.t(Msg::UpdateAvailable)
                  } else {
                    locale.t(Msg::UpToDate)
                  }
                } else {
                  ""
                }
              }}
            </p>
            <Show when=move || {
              update.get().and_then(|u| u.version.filter(|_| u.available)).is_some()
            }>
              <p class="settings-updater-version font-mono">
                {move || {
                  update
                    .get()
                    .and_then(|u| u.version)
                    .unwrap_or_default()
                }}
              </p>
            </Show>
            <Show when=move || {
              update
                .get()
                .and_then(|u| u.notes.filter(|n| !n.trim().is_empty()))
                .is_some()
            }>
              <p class="settings-updater-notes">
                {move || {
                  update
                    .get()
                    .and_then(|u| u.notes)
                    .unwrap_or_default()
                }}
              </p>
            </Show>
            <Show when=move || installing.get() && content_length.get().is_some()>
              <Progress class="settings-updater-progress".to_string() value=progress_pct />
            </Show>
            <div class="settings-updater-actions">
              <Show when=move || available.get() && !installing.get()>
                <Button
                  variant=ButtonVariant::Default
                  on:click=on_install
                  class="settings-primary-btn"
                >
                  {i18n.t(Msg::InstallAndRestart)}
                </Button>
              </Show>
              <Show when=move || !available.get() || !installing.get()>
                <Button
                  variant=ButtonVariant::Outline
                  on:click=on_check
                  prop:disabled=move || checking.get() || installing.get()
                  class="settings-chip-btn"
                >
                  {move || {
                    if checking.get() {
                      i18n.locale().get().t(Msg::CheckingForUpdates)
                    } else {
                      i18n.locale().get().t(Msg::CheckForUpdates)
                    }
                  }}
                </Button>
              </Show>
            </div>
          </Show>
          <Show when=move || error.get().is_some()>
            <p class="home-error">{move || error.get().unwrap_or_default()}</p>
          </Show>
        </div>

        <div class="settings-group settings-about-blurb">
          <p>
            {i18n.t(Msg::AboutBlurb)}
          </p>
        </div>
      </div>
    </SettingsDetail>
  }
}

// ─── Language ──────────────────────────────────────────────────────────────

#[component]
pub fn SettingsLanguagePage() -> impl IntoView {
  let i18n = use_i18n();
  view! {
    <SettingsDetail title=i18n.t(Msg::Language) subtitle=i18n.t(Msg::LanguageDetailSub)>
      <section
        class="settings-group"
        role="radiogroup"
        aria-label=move || i18n.t(Msg::Language).get()
      >
        {Locale::ALL
          .into_iter()
          .map(|locale| {
            view! {
              <button
                type="button"
                class="settings-lang-option"
                role="radio"
                aria-checked=move || (i18n.locale().get() == locale).to_string()
                on:click=move |_| i18n.set(locale)
              >
                <span class="settings-row-text">
                  <span class="settings-row-title">{locale.native_name()}</span>
                </span>
                <Show when=move || i18n.locale().get() == locale>
                  <Check class="settings-lang-check size-4".to_string() />
                </Show>
              </button>
            }
          })
          .collect_view()}
      </section>
    </SettingsDetail>
  }
}
