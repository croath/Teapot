//! macOS-style Settings: hub + Generate / Debug / About panes.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use icons::{Bug, ChevronRight, Info, ServerCog};
use leptos::prelude::*;
use leptos_router::components::A;
use wasm_bindgen_futures::spawn_local;

use crate::components::{Button, ButtonVariant, Input, InputType, Label};
use crate::tauri_bridge::{self, AppConfigDto, AppInfo, SaveConfigArgs};

// ─── Hub ───────────────────────────────────────────────────────────────────

/// Settings root: category list (macOS System Settings feel).
#[component]
pub fn SettingsPage() -> impl IntoView {
  view! {
    <div class="settings-shell">
      <header class="settings-hero">
        <div class="settings-done">
          <A href="/">"Done"</A>
        </div>
        <h1 class="settings-large-title">"Settings"</h1>
        <p class="settings-hero-sub">"Configure teapotx and explore diagnostics."</p>
      </header>

      <section class="settings-group" aria-label="Settings categories">
        <SettingsRow
          href="/settings/generate"
          title="Generate"
          subtitle="Listen address & API key"
          accent="generate"
        >
          <ServerCog class="size-5".to_string() />
        </SettingsRow>
        <SettingsRow
          href="/settings/debug"
          title="Debug"
          subtitle="Live logs & export"
          accent="debug"
        >
          <Bug class="size-5".to_string() />
        </SettingsRow>
        <SettingsRow
          href="/settings/about"
          title="About"
          subtitle="Version & app info"
          accent="about"
        >
          <Info class="size-5".to_string() />
        </SettingsRow>
      </section>
    </div>
  }
}

#[component]
fn SettingsRow(
  href: &'static str,
  title: &'static str,
  subtitle: &'static str,
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
        <span class="settings-row-title">{title}</span>
        <span class="settings-row-sub">{subtitle}</span>
      </span>
      <ChevronRight class="settings-row-chevron size-4".to_string() />
    </A>
  }
}

// ─── Shared chrome for detail panes ────────────────────────────────────────

#[component]
fn SettingsDetail(
  title: &'static str,
  #[prop(optional)] subtitle: Option<&'static str>,
  children: Children,
) -> impl IntoView {
  let sub = subtitle.unwrap_or("");
  let has_sub = !sub.is_empty();
  view! {
    <div class="settings-shell settings-shell-detail">
      <header class="settings-detail-bar">
        <div class="settings-back">
          <A href="/settings">
            <span class="settings-back-chevron" aria-hidden="true">"‹"</span>
            <span>"Settings"</span>
          </A>
        </div>
        <h1 class="settings-detail-title">{title}</h1>
        <Show when=move || has_sub>
          <p class="settings-detail-sub">{sub}</p>
        </Show>
      </header>
      <div class="settings-detail-body">
        {children()}
      </div>
    </div>
  }
}

// ─── Generate ──────────────────────────────────────────────────────────────

#[component]
pub fn SettingsGeneratePage() -> impl IntoView {
  let listen = RwSignal::new("127.0.0.1:8080".to_string());
  let api_key = RwSignal::new(String::new());
  let config_path = RwSignal::new(String::new());
  let status = RwSignal::new(Option::<String>::None);
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
        }
        Err(e) => error.set(Some(e)),
      }
      if let Ok(path) = tauri_bridge::invoke0::<String>("get_config_path").await {
        config_path.set(path);
      }
    });
  });

  let on_save = move |_| {
    if !in_tauri {
      error.set(Some(
        "Open Teapot via the desktop app to save config.".into(),
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
    };
    spawn_local(async move {
      match tauri_bridge::invoke::<(), _>("save_config", SaveConfigArgs { config: cfg }).await {
        Ok(()) => status.set(Some("Config saved. Restart the server to apply.".into())),
        Err(e) => error.set(Some(e)),
      }
      saving.set(false);
    });
  };

  view! {
    <SettingsDetail title="Generate" subtitle="teapotx serve configuration">
      <div class="settings-group settings-group-form">
        <div class="settings-field">
          <Label>"Listen address"</Label>
          <Input
            r#type=InputType::Text
            bind_value=listen
            class="font-mono settings-input"
            placeholder="127.0.0.1:8080"
          />
          <p class="field-hint">"Host:port for the local API server."</p>
        </div>
        <div class="settings-field">
          <Label>"API key"</Label>
          <Input
            r#type=InputType::Password
            bind_value=api_key
            class="font-mono settings-input"
            placeholder="(empty — no auth)"
          />
          <p class="field-hint">
            "Optional. When set, clients need Bearer / x-api-key."
          </p>
        </div>
        <Show when=move || !config_path.get().is_empty()>
          <p class="settings-path font-mono">
            {move || config_path.get()}
          </p>
        </Show>
      </div>

      <div class="settings-footer-actions">
        <Button
          variant=ButtonVariant::Default
          on:click=on_save
          prop:disabled=move || saving.get() || !in_tauri
          class="settings-primary-btn"
        >
          {move || if saving.get() { "Saving…" } else { "Save config" }}
        </Button>
      </div>

      <Show when=move || status.get().is_some()>
        <p class="form-ok">{move || status.get().unwrap_or_default()}</p>
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
  let logs = RwSignal::new(String::new());
  let error = RwSignal::new(Option::<String>::None);
  let in_tauri = tauri_bridge::is_tauri();

  let refresh = move || {
    if !in_tauri {
      logs.set("Logs are available in the Teapot desktop app.".into());
      return;
    }
    spawn_local(async move {
      match tauri_bridge::invoke0::<String>("get_logs").await {
        Ok(text) => {
          logs.set(if text.is_empty() {
            "(no log lines yet)".into()
          } else {
            text
          });
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
    if content.is_empty() || content == "(no log lines yet)" {
      error.set(Some("Nothing to export.".into()));
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
      logs.set("(no log lines yet)".into());
    });
  };

  view! {
    <SettingsDetail title="Debug" subtitle="teapotx stdout / stderr">
      <div class="settings-toolbar">
        <Button variant=ButtonVariant::Outline on:click=on_clear class="settings-chip-btn">
          "Clear"
        </Button>
        <Button variant=ButtonVariant::Default on:click=on_export class="settings-chip-btn">
          "Export"
        </Button>
      </div>

      <div class="settings-group settings-log-card">
        <pre class="log-view">{move || logs.get()}</pre>
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
  let version = RwSignal::new(env!("CARGO_PKG_VERSION").to_string());
  let name = RwSignal::new("Teapot".to_string());
  let in_tauri = tauri_bridge::is_tauri();

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

  view! {
    <SettingsDetail title="About">
      <div class="settings-about">
        <div class="settings-about-hero">
          <div class="settings-about-glow" aria-hidden="true"></div>
          <img
            src="app-icon.png"
            alt=""
            class="settings-about-icon"
            width="96"
            height="96"
          />
          <h2 class="settings-about-name">{move || name.get()}</h2>
          <p class="settings-about-version">
            "Version "
            <span class="font-mono">{move || version.get()}</span>
          </p>
        </div>

        <div class="settings-group settings-about-blurb">
          <p>
            "Turn local provider CLIs into OpenAI- and Anthropic-compatible HTTP APIs."
          </p>
        </div>
      </div>
    </SettingsDetail>
  }
}
