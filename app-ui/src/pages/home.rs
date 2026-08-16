//! Main page: status + switch for `teapotx serve`.

use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::components::Switch;
use crate::i18n::{Msg, use_i18n};
use crate::tauri_bridge::{self, ServerStatus};

#[component]
pub fn HomePage() -> impl IntoView {
  let i18n = use_i18n();
  let running = RwSignal::new(false);
  let busy = RwSignal::new(false);
  let error = RwSignal::new(Option::<String>::None);
  let in_tauri = tauri_bridge::is_tauri();

  Effect::new(move |_| {
    if !in_tauri {
      return;
    }
    spawn_local(async move {
      match tauri_bridge::invoke0::<ServerStatus>("get_server_status").await {
        Ok(status) => running.set(status.running),
        Err(e) => error.set(Some(e)),
      }
    });
  });

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
    if busy.get_untracked() {
      return;
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

  let switch_disabled = Signal::derive(move || !in_tauri);
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
                let msg = match (busy.get(), running.get()) {
                  (true, true) => Msg::Starting,
                  (true, false) => Msg::Stopping,
                  (false, true) => Msg::Running,
                  (false, false) => Msg::Stopped,
                };
                i18n.locale().get().t(msg)
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
