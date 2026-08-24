//! Main page: provider picker + auth + switch for `teapotx serve`.

use leptos::ev;
use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::components::{Button, ButtonSize, ButtonVariant, Switch};
use crate::i18n::{Msg, use_i18n};
use crate::tauri_bridge::{
  self, AppConfigDto, AuthStatus, ProviderArgs, ProviderInfo, ServerStatus,
};

const DEFAULT_PROVIDER: &str = "codex-cli";

fn fallback_providers() -> Vec<ProviderInfo> {
  [
    ("codex-cli", "Codex CLI", false),
    ("claude-cli", "Claude CLI", false),
    ("grok-cli", "Grok CLI", false),
    ("antigravity", "Antigravity", true),
    ("vertex", "Vertex", true),
  ]
  .into_iter()
  .map(|(id, label, supports_auth)| ProviderInfo {
    id: id.into(),
    label: label.into(),
    description: String::new(),
    command: String::new(),
    installed: true,
    supports_auth,
    requires_local_cli: false,
    install_hint: None,
  })
  .collect()
}

#[component]
pub fn HomePage() -> impl IntoView {
  let i18n = use_i18n();
  let running = RwSignal::new(false);
  let busy = RwSignal::new(false);
  let signing_in = RwSignal::new(false);
  let provider = RwSignal::new(DEFAULT_PROVIDER.to_string());
  let providers = RwSignal::new(fallback_providers());
  let auth = RwSignal::new(Option::<AuthStatus>::None);
  let error = RwSignal::new(Option::<String>::None);
  let in_tauri = tauri_bridge::is_tauri();

  let refresh_auth = move |id: String| {
    if !in_tauri {
      return;
    }
    spawn_local(async move {
      match tauri_bridge::invoke::<AuthStatus, _>("get_auth_status", ProviderArgs { provider: id })
        .await
      {
        Ok(status) => {
          auth.set(Some(status));
        }
        Err(e) => {
          auth.set(None);
          error.set(Some(e));
        }
      }
    });
  };

  Effect::new(move |_| {
    if !in_tauri {
      return;
    }
    tauri_bridge::listen_json::<bool>("teapotx-status", move |on| {
      running.set(on);
    });
    spawn_local(async move {
      match tauri_bridge::invoke0::<ServerStatus>("get_server_status").await {
        Ok(status) => running.set(status.running),
        Err(e) => error.set(Some(e)),
      }
      match tauri_bridge::invoke0::<Vec<ProviderInfo>>("list_providers").await {
        Ok(list) if !list.is_empty() => providers.set(list),
        _ => {}
      }
      match tauri_bridge::invoke0::<AppConfigDto>("get_config").await {
        Ok(cfg) => {
          let id = if cfg.provider.trim().is_empty() {
            DEFAULT_PROVIDER.to_string()
          } else {
            cfg.provider
          };
          provider.set(id.clone());
          refresh_auth(id);
        }
        Err(e) => error.set(Some(e)),
      }
    });
  });

  let on_provider_change = move |ev: ev::Event| {
    let id = event_target_value(&ev);
    if id.is_empty() || id == provider.get_untracked() {
      return;
    }
    if running.get_untracked() || signing_in.get_untracked() {
      return;
    }
    provider.set(id.clone());
    auth.set(None);
    error.set(None);
    if !in_tauri {
      return;
    }
    spawn_local(async move {
      match tauri_bridge::invoke::<AppConfigDto, _>(
        "set_provider",
        ProviderArgs {
          provider: id.clone(),
        },
      )
      .await
      {
        Ok(cfg) => {
          provider.set(cfg.provider.clone());
          refresh_auth(cfg.provider);
        }
        Err(e) => {
          error.set(Some(e));
          refresh_auth(id);
        }
      }
    });
  };

  let on_sign_in = move |_| {
    if !in_tauri {
      error.set(Some(
        i18n
          .locale()
          .get_untracked()
          .t(Msg::DesktopOnlyServe)
          .into(),
      ));
      return;
    }
    if signing_in.get_untracked() || running.get_untracked() {
      return;
    }
    let id = provider.get_untracked();
    signing_in.set(true);
    error.set(None);
    spawn_local(async move {
      match tauri_bridge::invoke::<AuthStatus, _>(
        "login_provider",
        ProviderArgs {
          provider: id.clone(),
        },
      )
      .await
      {
        Ok(status) => {
          auth.set(Some(status));
        }
        Err(e) => {
          if e != "Sign-in cancelled." {
            error.set(Some(e));
          }
          refresh_auth(id);
        }
      }
      signing_in.set(false);
    });
  };

  let on_cancel_sign_in = move |_| {
    if !in_tauri || !signing_in.get_untracked() {
      return;
    }
    spawn_local(async move {
      if let Err(e) = tauri_bridge::invoke0::<()>("cancel_login").await {
        error.set(Some(e));
      }
    });
  };

  let on_toggle = Callback::new(move |want_on: bool| {
    if !in_tauri {
      error.set(Some(
        i18n
          .locale()
          .get_untracked()
          .t(Msg::DesktopOnlyServe)
          .into(),
      ));
      return;
    }
    if busy.get_untracked() || signing_in.get_untracked() {
      return;
    }
    let status = auth.get_untracked();
    if want_on && status.as_ref().is_some_and(|s| !s.installed) {
      let hint = status
        .as_ref()
        .and_then(|s| s.install_hint.clone())
        .unwrap_or_else(|| i18n.locale().get_untracked().t(Msg::CliMissing).into());
      error.set(Some(hint));
      return;
    }
    if want_on && !status.as_ref().is_some_and(|s| s.authenticated) {
      let needs_sign_in = status.as_ref().map(|s| s.supports_auth).unwrap_or(true);
      if needs_sign_in {
        error.set(Some(
          i18n.locale().get_untracked().t(Msg::AuthRequired).into(),
        ));
        return;
      }
    }
    running.set(want_on);
    busy.set(true);
    error.set(None);
    spawn_local(async move {
      let result = if want_on {
        tauri_bridge::invoke0::<()>("start_server").await
      } else {
        tauri_bridge::invoke0::<()>("stop_server").await
      };
      match result {
        Ok(()) => {}
        Err(e) => {
          error.set(Some(e));
          if let Ok(status) = tauri_bridge::invoke0::<ServerStatus>("get_server_status").await {
            running.set(status.running);
          } else {
            running.set(!want_on);
          }
        }
      }
      busy.set(false);
    });
  });

  let authenticated = Signal::derive(move || auth.get().is_some_and(|s| s.authenticated));
  let supports_auth = Signal::derive(move || auth.get().is_some_and(|s| s.supports_auth));
  let cli_ready = Signal::derive(move || auth.get().is_none_or(|s| s.installed));
  let switch_disabled = Signal::derive(move || {
    let needs_auth = supports_auth.get() && !authenticated.get();
    !in_tauri || needs_auth || signing_in.get() || !cli_ready.get()
  });
  let picker_disabled = Signal::derive(move || !in_tauri || running.get() || signing_in.get());
  let serve_aria = Signal::derive(move || i18n.t(Msg::ServeAria).get().to_string());

  view! {
    <div class="page-center">
      <div class="home-simple">
        <div class="home-intro">
          <div class="home-mark">
            <div class="home-mark-glow" aria-hidden="true"></div>
            <img
              src="/app-icon.png"
              alt="Teapot"
              class="home-mark-icon"
              width="64"
              height="64"
            />
          </div>
          <h1 class="home-name">"Teapot"</h1>
          <p class="home-blurb">
            {i18n.t(Msg::HomeBlurb)}
          </p>
        </div>

        <div class="provider-card">
          <label class="provider-picker" for="home-provider">
            <span class="provider-picker-label">{i18n.t(Msg::Provider)}</span>
            <span class="provider-select-wrap">
              <select
                id="home-provider"
                class="provider-select"
                prop:value=move || provider.get()
                prop:disabled=move || picker_disabled.get()
                aria-label=move || i18n.t(Msg::Provider).get()
                on:change=on_provider_change
              >
                {move || {
                  providers
                    .get()
                    .into_iter()
                    .map(|p| {
                      let id = p.id.clone();
                      let label = if p.installed {
                        p.label.clone()
                      } else {
                        format!("{} — not installed", p.label)
                      };
                      view! {
                        <option value=id.clone() selected=move || provider.get() == id>
                          {label}
                        </option>
                      }
                    })
                    .collect_view()
                }}
              </select>
            </span>
          </label>

          <div class="provider-auth">
            <Show when=move || {
              auth.get().is_some_and(|s| s.requires_local_cli && !s.installed)
            }>
              <p class="provider-auth-hint">
                {move || {
                  auth
                    .get()
                    .and_then(|s| s.install_hint)
                    .filter(|h| !h.is_empty())
                    .unwrap_or_else(|| {
                      if provider.get() == "codex-cli" {
                        i18n.locale().get().t(Msg::CodexCliInstallHint).to_string()
                      } else {
                        i18n.locale().get().t(Msg::CliMissing).to_string()
                      }
                    })
                }}
              </p>
            </Show>
            <Show when=move || {
              auth.get().is_some_and(|s| s.installed)
                && !supports_auth.get()
                && !signing_in.get()
            }>
              <p class="provider-auth-ok">
                <span class="status-dot status-dot-on"></span>
                <span>{i18n.t(Msg::AuthNotRequiredHint)}</span>
              </p>
            </Show>
            <Show when=move || supports_auth.get() && authenticated.get() && !signing_in.get()>
              <p class="provider-auth-ok">
                <span class="status-dot status-dot-on"></span>
                <span>
                  {i18n.t(Msg::AuthSignedIn)}
                  {move || {
                    auth
                      .get()
                      .and_then(|s| s.account)
                      .filter(|a| !a.is_empty())
                      .map(|a| format!(" {a}"))
                      .unwrap_or_default()
                  }}
                </span>
              </p>
              <Button
                variant=ButtonVariant::Outline
                size=ButtonSize::Sm
                class="provider-auth-btn"
                on:click=on_sign_in
                prop:disabled=move || !in_tauri || running.get()
              >
                {move || {
                  let import = provider.get() == "vertex"
                    || auth.get().is_some_and(|s| s.is_import());
                  if import {
                    i18n.locale().get().t(Msg::AuthImport)
                  } else {
                    i18n.locale().get().t(Msg::AuthSignInAgain)
                  }
                }}
              </Button>
            </Show>
            <Show when=move || supports_auth.get() && signing_in.get()>
              <p class="provider-auth-wait">{i18n.t(Msg::AuthWaitingBrowser)}</p>
              <Button
                variant=ButtonVariant::Outline
                size=ButtonSize::Sm
                class="provider-auth-btn"
                on:click=on_cancel_sign_in
              >
                {i18n.t(Msg::AuthCancel)}
              </Button>
            </Show>
            <Show when=move || {
              supports_auth.get() && !authenticated.get() && !signing_in.get()
            }>
              <p class="provider-auth-hint">{i18n.t(Msg::AuthRequiredHint)}</p>
              <Button
                variant=ButtonVariant::Default
                size=ButtonSize::Sm
                class="provider-auth-btn"
                on:click=on_sign_in
                prop:disabled=move || !in_tauri || running.get()
              >
                {move || {
                  let import = provider.get() == "vertex"
                    || auth.get().is_some_and(|s| s.is_import());
                  if import {
                    i18n.locale().get().t(Msg::AuthImport)
                  } else {
                    i18n.locale().get().t(Msg::AuthSignIn)
                  }
                }}
              </Button>
            </Show>
          </div>
        </div>

        <div
          class="serve-switch"
          data-state=move || if running.get() { "checked" } else { "unchecked" }
        >
          <div class="serve-switch-row">
            <span
              class=move || {
                if running.get() {
                  "serve-switch-label"
                } else {
                  "serve-switch-label serve-switch-label-on"
                }
              }
            >
              {i18n.t(Msg::Stop)}
            </span>
            <Switch
              checked=Signal::derive(move || running.get())
              on_change=on_toggle
              disabled=switch_disabled
              aria_label=serve_aria
            />
            <span
              class=move || {
                if running.get() {
                  "serve-switch-label serve-switch-label-on"
                } else {
                  "serve-switch-label"
                }
              }
            >
              {i18n.t(Msg::Start)}
            </span>
          </div>

          <p class="serve-switch-status">
            <span
              class=move || {
                if running.get() {
                  "status-dot status-dot-on"
                } else {
                  "status-dot status-dot-off"
                }
              }
            ></span>
            <span>
              {move || {
                let locale = i18n.locale().get();
                if !cli_ready.get() && !running.get() {
                  return locale.t(Msg::CliMissing);
                }
                if supports_auth.get() && !authenticated.get() && !running.get() {
                  return locale.t(Msg::AuthRequired);
                }
                let msg = match (busy.get(), running.get()) {
                  (true, true) => Msg::Starting,
                  (true, false) => Msg::Stopping,
                  (false, true) => Msg::Running,
                  (false, false) => Msg::Stopped,
                };
                locale.t(msg)
              }}
            </span>
          </p>
        </div>

        <Show when=move || error.get().is_some()>
          <p class="home-error home-error-compact">
            {move || error.get().unwrap_or_default()}
          </p>
        </Show>
      </div>
    </div>
  }
}
