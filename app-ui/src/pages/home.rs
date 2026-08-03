//! Main page: status + switch for `teapotx serve`.

use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::components::{Badge, BadgeVariant, Switch};
use crate::tauri_bridge::{self, ServerStatus};

#[component]
pub fn HomePage() -> impl IntoView {
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
        "Open Teapot via the desktop app to control the server.".into(),
      ));
      return;
    }
    if busy.get_untracked() {
      return;
    }
    busy.set(true);
    error.set(None);
    spawn_local(async move {
      let result = if want_on {
        tauri_bridge::invoke0::<()>("start_server").await
      } else {
        tauri_bridge::invoke0::<()>("stop_server").await
      };
      match result {
        Ok(()) => running.set(want_on),
        Err(e) => {
          error.set(Some(e));
          if let Ok(status) = tauri_bridge::invoke0::<ServerStatus>("get_server_status").await {
            running.set(status.running);
          }
        }
      }
      busy.set(false);
    });
  });

  let switch_disabled = Signal::derive(move || busy.get() || !in_tauri);

  view! {
    <div class="page-center">
      <div class="home-simple">
        <div class="home-intro">
          <h1 class="home-name">"Teapot"</h1>
          <p class="home-blurb">
            "Turn local provider CLIs into OpenAI- and Anthropic-compatible APIs."
          </p>
        </div>

        <Show
          when=move || running.get()
          fallback=move || {
            view! {
              <Badge variant=BadgeVariant::Muted class="gap-1.5">
                <span class="status-dot status-dot-off"></span>
                "Stopped"
              </Badge>
            }
          }
        >
          <Badge variant=BadgeVariant::Default class="gap-1.5 bg-primary/20 text-primary">
            <span class="status-dot status-dot-on"></span>
            "Running"
          </Badge>
        </Show>

        <Switch
          checked=Signal::derive(move || running.get())
          on_change=on_toggle
          disabled=switch_disabled
          aria_label="Start or stop Teapot server".to_string()
          class="scale-125"
        />

        <Show when=move || error.get().is_some()>
          <p class="home-error home-error-compact">
            {move || error.get().unwrap_or_default()}
          </p>
        </Show>
      </div>
    </div>
  }
}
