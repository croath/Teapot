//! API playground: send a request to the local teaport server.

use gloo_net::http::Request;
use leptos::prelude::*;
use serde_json::json;
use wasm_bindgen_futures::spawn_local;

use crate::tauri_api;

const DEFAULT_BASE: &str = "http://127.0.0.1:8080";

#[component]
pub fn PlaygroundPage() -> impl IntoView {
  let base_url = RwSignal::new(DEFAULT_BASE.to_string());
  let model = RwSignal::new("codex".to_string());
  let api_style = RwSignal::new("chatgpt".to_string());
  let prompt = RwSignal::new(String::new());
  let output = RwSignal::new(String::new());
  let busy = RwSignal::new(false);
  let error = RwSignal::new(Option::<String>::None);

  // Prefer base URL from the Tauri-managed server when available
  Effect::new(move |_| {
    if tauri_api::is_tauri() {
      spawn_local(async move {
        if let Ok(status) = tauri_api::get_server_status().await {
          if let Some(url) = status.base_url {
            base_url.set(url);
          }
        }
      });
    }
  });

  let on_send = move |_| {
    if busy.get() {
      return;
    }
    let base = base_url.get();
    let model_id = model.get();
    let style = api_style.get();
    let user_prompt = prompt.get();
    if user_prompt.trim().is_empty() {
      error.set(Some("Prompt is empty".into()));
      return;
    }

    busy.set(true);
    error.set(None);
    output.set(String::new());

    spawn_local(async move {
      let result = match style.as_str() {
        "claude" => call_claude(&base, &model_id, &user_prompt).await,
        _ => call_chatgpt(&base, &model_id, &user_prompt).await,
      };
      match result {
        Ok(text) => output.set(text),
        Err(e) => error.set(Some(e)),
      }
      busy.set(false);
    });
  };

  view! {
    <section class="card space-y-4">
      <div>
        <h1 class="m-0 text-3xl font-semibold tracking-tight">"Playground"</h1>
        <p class="mt-2 text-muted">
          "Send a non-streaming request. Start the server on the "
          <a href="/">"Server"</a>
          " page first."
        </p>
      </div>

      <div class="grid grid-cols-1 gap-4 sm:grid-cols-3">
        <label class="field">
          "Base URL"
          <input
            class="field-input"
            type="text"
            prop:value=move || base_url.get()
            on:input=move |ev| base_url.set(event_target_value(&ev))
          />
        </label>
        <label class="field">
          "API"
          <select
            class="field-input"
            prop:value=move || api_style.get()
            on:change=move |ev| api_style.set(event_target_value(&ev))
          >
            <option value="chatgpt">"ChatGPT /chat/completions"</option>
            <option value="claude">"Claude /messages"</option>
          </select>
        </label>
        <label class="field">
          "Model / Agent"
          <input
            class="field-input"
            type="text"
            prop:value=move || model.get()
            on:input=move |ev| model.set(event_target_value(&ev))
          />
        </label>
      </div>

      <label class="field">
        "Prompt"
        <textarea
          class="field-input min-h-32 w-full resize-y"
          rows="6"
          prop:value=move || prompt.get()
          on:input=move |ev| prompt.set(event_target_value(&ev))
        />
      </label>

      <div class="flex flex-wrap gap-3">
        <button class="btn btn-primary" prop:disabled=move || busy.get() on:click=on_send>
          {move || if busy.get() { "Running…" } else { "Send" }}
        </button>
        <a class="btn" href="/">"Server"</a>
      </div>

      <Show when=move || error.get().is_some()>
        <p class="text-sm text-red-400">{move || error.get().unwrap_or_default()}</p>
      </Show>

      <label class="field">
        "Response"
        <pre class="field-input max-h-[28rem] min-h-32 overflow-auto whitespace-pre-wrap break-words font-mono text-sm">
          {move || output.get()}
        </pre>
      </label>
    </section>
  }
}

async fn call_chatgpt(base: &str, model: &str, prompt: &str) -> Result<String, String> {
  let url = format!(
    "{}/chatgpt/v1/chat/completions",
    base.trim_end_matches('/')
  );
  let body = json!({
    "model": model,
    "stream": false,
    "messages": [
      { "role": "user", "content": prompt }
    ]
  });
  let resp = Request::post(&url)
    .header("Content-Type", "application/json")
    .json(&body)
    .map_err(|e| e.to_string())?
    .send()
    .await
    .map_err(|e| e.to_string())?;

  if !resp.ok() {
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    return Err(format!("HTTP {status}: {text}"));
  }

  let value: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
  if let Some(text) = value
    .pointer("/choices/0/message/content")
    .and_then(|v| v.as_str())
  {
    Ok(text.to_string())
  } else {
    Ok(value.to_string())
  }
}

async fn call_claude(base: &str, model: &str, prompt: &str) -> Result<String, String> {
  let url = format!("{}/claude/v1/messages", base.trim_end_matches('/'));
  let body = json!({
    "model": model,
    "max_tokens": 4096,
    "stream": false,
    "messages": [
      { "role": "user", "content": prompt }
    ]
  });
  let resp = Request::post(&url)
    .header("Content-Type", "application/json")
    .header("anthropic-version", "2023-06-01")
    .json(&body)
    .map_err(|e| e.to_string())?
    .send()
    .await
    .map_err(|e| e.to_string())?;

  if !resp.ok() {
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    return Err(format!("HTTP {status}: {text}"));
  }

  let value: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
  if let Some(text) = value.pointer("/content/0/text").and_then(|v| v.as_str()) {
    Ok(text.to_string())
  } else {
    Ok(value.to_string())
  }
}
