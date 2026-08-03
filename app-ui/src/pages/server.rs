//! Server control page: start / stop the local API, pick agent CLI, persist prefs.

use gloo_net::http::Request;
use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::tauri_api::{self, AgentInfo, ServerStatus, UserPreferences};

const DEFAULT_LISTEN: &str = "127.0.0.1:8080";

#[component]
pub fn ServerPage() -> impl IntoView {
  let listen = RwSignal::new(DEFAULT_LISTEN.to_string());
  let agent = RwSignal::new(String::new());
  let agents = RwSignal::new(Vec::<AgentInfo>::new());
  let status = RwSignal::new(ServerStatus {
    running: false,
    listen: None,
    base_url: None,
    agent: None,
  });
  let message = RwSignal::new(Option::<String>::None);
  let error = RwSignal::new(Option::<String>::None);
  let busy = RwSignal::new(false);
  let in_tauri = RwSignal::new(false);

  // Load prefs + agents + server status
  Effect::new(move |_| {
    in_tauri.set(tauri_api::is_tauri());
    spawn_local(async move {
      match tauri_api::list_agents().await {
        Ok(list) => {
          // Prefer installed agents first in the select
          let mut list = list;
          list.sort_by(|a, b| {
            b.installed
              .cmp(&a.installed)
              .then_with(|| a.name.cmp(&b.name))
          });
          agents.set(list);
        }
        Err(e) => error.set(Some(e)),
      }

      match tauri_api::load_preferences().await {
        Ok(prefs) => {
          if !prefs.listen.is_empty() {
            listen.set(prefs.listen);
          }
          if let Some(a) = prefs.agent {
            agent.set(a);
          } else if agent.get_untracked().is_empty() {
            // default to first installed agent
            if let Some(first) = agents
              .get_untracked()
              .into_iter()
              .find(|a| a.installed)
              .or_else(|| agents.get_untracked().into_iter().next())
            {
              agent.set(first.name);
            }
          }
        }
        Err(e) => error.set(Some(format!("preferences: {e}"))),
      }

      if tauri_api::is_tauri() {
        match tauri_api::get_server_status().await {
          Ok(s) => {
            if let Some(l) = s.listen.clone() {
              listen.set(l);
            }
            if let Some(a) = s.agent.clone() {
              agent.set(a);
            }
            status.set(s);
          }
          Err(e) => error.set(Some(e)),
        }
      }
    });
  });

  let persist_prefs = move || {
    let prefs = UserPreferences {
      listen: listen.get_untracked(),
      agent: {
        let a = agent.get_untracked();
        if a.is_empty() {
          None
        } else {
          Some(a)
        }
      },
    };
    spawn_local(async move {
      if let Err(e) = tauri_api::save_preferences(&prefs).await {
        error.set(Some(format!("save preferences: {e}")));
      }
    });
  };

  let on_agent_change = move |ev| {
    agent.set(event_target_value(&ev));
    persist_prefs();
  };

  let on_listen_change = move |ev| {
    listen.set(event_target_value(&ev));
  };

  let on_listen_blur = move |_| {
    persist_prefs();
  };

  let refresh = move |_| {
    busy.set(true);
    error.set(None);
    message.set(None);
    if tauri_api::is_tauri() {
      spawn_local(async move {
        match tauri_api::get_server_status().await {
          Ok(s) => {
            if let Some(l) = s.listen.clone() {
              listen.set(l);
            }
            if let Some(a) = s.agent.clone() {
              agent.set(a);
            }
            status.set(s);
            message.set(Some("Status refreshed".into()));
          }
          Err(e) => error.set(Some(e)),
        }
        busy.set(false);
      });
    } else {
      let addr = listen.get();
      let url = format!("http://{}/health", addr.trim_start_matches("http://"));
      spawn_local(async move {
        match Request::get(&url).send().await {
          Ok(resp) if resp.ok() => {
            status.set(ServerStatus {
              running: true,
              listen: Some(addr.clone()),
              base_url: Some(format!("http://{}", addr.trim_start_matches("http://"))),
              agent: None,
            });
            message.set(Some("Health check OK (external server)".into()));
          }
          Ok(resp) => {
            status.set(ServerStatus {
              running: false,
              listen: None,
              base_url: None,
              agent: None,
            });
            error.set(Some(format!("Health check HTTP {}", resp.status())));
          }
          Err(e) => {
            status.set(ServerStatus {
              running: false,
              listen: None,
              base_url: None,
              agent: None,
            });
            error.set(Some(format!("Not reachable: {e}")));
          }
        }
        busy.set(false);
      });
    }
  };

  let start = move |_| {
    if busy.get() {
      return;
    }
    if agent.get().trim().is_empty() {
      error.set(Some("Select an agent CLI before starting".into()));
      return;
    }
    busy.set(true);
    error.set(None);
    message.set(None);
    let addr = listen.get();
    let agent_name = agent.get();
    // Persist before start
    let prefs = UserPreferences {
      listen: addr.clone(),
      agent: Some(agent_name.clone()),
    };
    spawn_local(async move {
      let _ = tauri_api::save_preferences(&prefs).await;
      match tauri_api::start_api_server(Some(addr), Some(agent_name)).await {
        Ok(s) => {
          if let Some(l) = s.listen.clone() {
            listen.set(l);
          }
          if let Some(a) = s.agent.clone() {
            agent.set(a);
          }
          let url = s.base_url.clone().unwrap_or_default();
          let agent_label = s.agent.clone().unwrap_or_else(|| "default".into());
          status.set(s);
          message.set(Some(format!(
            "Server started at {url} (agent: {agent_label})"
          )));
        }
        Err(e) => error.set(Some(e)),
      }
      busy.set(false);
    });
  };

  let stop = move |_| {
    if busy.get() {
      return;
    }
    busy.set(true);
    error.set(None);
    message.set(None);
    spawn_local(async move {
      match tauri_api::stop_api_server().await {
        Ok(s) => {
          status.set(s);
          message.set(Some("Server stopped".into()));
        }
        Err(e) => error.set(Some(e)),
      }
      busy.set(false);
    });
  };

  view! {
    <section class="card space-y-5">
      <div>
        <h1 class="m-0 text-3xl font-semibold tracking-tight">"Teaport Server"</h1>
        <p class="mt-2 text-muted">
          "Choose an agent CLI, then start the local ChatGPT / Claude compatible API. "
          "Models are loaded from the selected agent only."
        </p>
      </div>

      <div class="flex flex-wrap items-center gap-3">
        <span
          class="inline-flex items-center gap-2 rounded-full border border-border px-3 py-1 text-sm"
          class:text-emerald-400=move || status.get().running
          class:text-muted=move || !status.get().running
        >
          <span
            class="inline-block h-2 w-2 rounded-full"
            class:bg-emerald-400=move || status.get().running
            class:bg-slate-500=move || !status.get().running
          ></span>
          {move || if status.get().running { "Running" } else { "Stopped" }}
        </span>
        <Show when=move || status.get().base_url.is_some()>
          <code class="text-sm">
            {move || status.get().base_url.unwrap_or_default()}
          </code>
        </Show>
        <Show when=move || status.get().agent.is_some()>
          <span class="text-sm text-muted">
            "agent: "
            <code>{move || status.get().agent.unwrap_or_default()}</code>
          </span>
        </Show>
      </div>

      <div class="grid grid-cols-1 gap-4 sm:grid-cols-2">
        <label class="field">
          "Agent CLI"
          <select
            class="field-input"
            prop:value=move || agent.get()
            prop:disabled=move || status.get().running || busy.get()
            on:change=on_agent_change
          >
            <option value="">"— select agent —"</option>
            {move || {
              agents
                .get()
                .into_iter()
                .map(|info| {
                  let label = if info.installed {
                    format!("{} — {}", info.name, info.description)
                  } else {
                    format!("{} — {} (not installed)", info.name, info.description)
                  };
                  let value = info.name.clone();
                  let selected = agent.get() == value;
                  view! {
                    <option value=value selected=selected disabled=!info.installed>
                      {label}
                    </option>
                  }
                })
                .collect_view()
            }}
          </select>
        </label>

        <label class="field">
          "Listen address"
          <input
            class="field-input"
            type="text"
            prop:value=move || listen.get()
            prop:disabled=move || status.get().running || busy.get()
            on:input=on_listen_change
            on:blur=on_listen_blur
          />
        </label>
      </div>

      <p class="m-0 text-xs text-muted">
        "Selection is saved to app local data (desktop) or browser localStorage."
      </p>

      <div class="flex flex-wrap gap-3">
        <button
          class="btn btn-primary"
          prop:disabled=move || {
            busy.get()
              || status.get().running
              || !in_tauri.get()
              || agent.get().trim().is_empty()
          }
          on:click=start
        >
          "Start"
        </button>
        <button
          class="btn"
          prop:disabled=move || busy.get() || !status.get().running || !in_tauri.get()
          on:click=stop
        >
          "Stop"
        </button>
        <button class="btn" prop:disabled=move || busy.get() on:click=refresh>
          "Refresh"
        </button>
        <a class="btn" href="/playground">"Open Playground"</a>
      </div>

      <Show when=move || !in_tauri.get()>
        <p class="rounded-lg border border-amber-500/40 bg-amber-500/10 px-3 py-2 text-sm text-amber-200">
          "Start/Stop is available in the Tauri desktop app. CLI equivalent: "
          <code>"cargo run -p teaport -- serve -a codex"</code>
          ". Preferences still save to localStorage in the browser."
        </p>
      </Show>

      <Show when=move || message.get().is_some()>
        <p class="text-sm text-emerald-300">{move || message.get().unwrap_or_default()}</p>
      </Show>
      <Show when=move || error.get().is_some()>
        <p class="text-sm text-red-400">{move || error.get().unwrap_or_default()}</p>
      </Show>

      <div class="space-y-2 border-t border-border pt-4 text-sm text-muted">
        <p class="m-0 font-medium text-slate-300">"Endpoints when running"</p>
        <ul class="list-disc space-y-1 pl-5">
          <li><code>"GET  /chatgpt/v1/models"</code>" (selected agent only)"</li>
          <li><code>"POST /chatgpt/v1/chat/completions"</code></li>
          <li><code>"POST /chatgpt/v1/responses"</code></li>
          <li><code>"GET  /claude/v1/models"</code></li>
          <li><code>"POST /claude/v1/messages"</code></li>
          <li><code>"GET  /health"</code></li>
        </ul>
      </div>
    </section>
  }
}
