//! Thin bridge to Tauri IPC (`window.__TAURI__`) from Leptos WASM.

use js_sys::{Function, Object, Promise, Reflect};
use serde::Serialize;
use serde::de::DeserializeOwned;
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_futures::spawn_local;

/// True when running inside the Tauri webview with global API injected.
pub fn is_tauri() -> bool {
  web_sys::window()
    .and_then(|w| Reflect::get(&w, &JsValue::from_str("__TAURI__")).ok())
    .map(|v| !v.is_undefined() && !v.is_null())
    .unwrap_or(false)
}

fn invoke_fn() -> Result<Function, String> {
  let window = web_sys::window().ok_or_else(|| "no window".to_string())?;
  let tauri = Reflect::get(&window, &JsValue::from_str("__TAURI__"))
    .map_err(|_| "Tauri global missing".to_string())?;
  if tauri.is_undefined() || tauri.is_null() {
    return Err("Not running inside Teapot desktop app".into());
  }
  let core = Reflect::get(&tauri, &JsValue::from_str("core"))
    .map_err(|_| "Tauri.core missing".to_string())?;
  let invoke = Reflect::get(&core, &JsValue::from_str("invoke"))
    .map_err(|_| "Tauri.core.invoke missing".to_string())?;
  invoke
    .dyn_into::<Function>()
    .map_err(|_| "invoke is not a function".to_string())
}

/// Call a Tauri command and deserialize the JSON-compatible result.
pub async fn invoke<T, A>(cmd: &str, args: A) -> Result<T, String>
where
  T: DeserializeOwned,
  A: Serialize,
{
  let json = serde_json::to_string(&args).map_err(|e| e.to_string())?;
  let args_js = js_sys::JSON::parse(&json).map_err(|e| format!("serialize args: {e:?}"))?;
  invoke_js(cmd, args_js).await
}

/// Invoke with no args (empty object).
pub async fn invoke0<T: DeserializeOwned>(cmd: &str) -> Result<T, String> {
  invoke_js(cmd, Object::new().into()).await
}

async fn invoke_js<T: DeserializeOwned>(cmd: &str, args_js: JsValue) -> Result<T, String> {
  let invoke = invoke_fn()?;
  let this = JsValue::NULL;
  let promise = invoke
    .call2(&this, &JsValue::from_str(cmd), &args_js)
    .map_err(|e| format!("invoke call failed: {e:?}"))?;
  let promise = Promise::from(promise);
  let result = JsFuture::from(promise).await.map_err(js_error_to_string)?;

  if result.is_undefined() || result.is_null() {
    // Unit / empty success
    return serde_json::from_str("null").map_err(|e| e.to_string());
  }

  let json = js_sys::JSON::stringify(&result)
    .map_err(|e| format!("stringify result: {e:?}"))?
    .as_string()
    .ok_or_else(|| "result is not a string".to_string())?;
  serde_json::from_str(&json).map_err(|e| format!("decode result: {e}"))
}

fn js_error_to_string(err: JsValue) -> String {
  if let Some(s) = err.as_string() {
    return s;
  }
  if let Ok(s) = js_sys::JSON::stringify(&err) {
    if let Some(text) = s.as_string() {
      return text;
    }
  }
  format!("{err:?}")
}

/// Listen for a Tauri event and ignore the payload.
/// The handler is kept for the lifetime of the page (`forget`).
pub fn listen(event: &str, handler: impl Fn() + 'static) {
  if !is_tauri() {
    return;
  }
  let event = event.to_string();
  spawn_local(async move {
    let Ok(listen) = event_listen_fn() else {
      return;
    };
    let cb = Closure::wrap(Box::new(move |_ev: JsValue| {
      handler();
    }) as Box<dyn FnMut(JsValue)>);
    let this = JsValue::NULL;
    let result = listen.call2(
      &this,
      &JsValue::from_str(&event),
      cb.as_ref().unchecked_ref(),
    );
    cb.forget();
    if let Ok(promise) = result {
      let _ = JsFuture::from(Promise::from(promise)).await;
    }
  });
}

/// Listen for a Tauri event whose payload is JSON-compatible.
pub fn listen_json<T>(event: &str, handler: impl Fn(T) + 'static)
where
  T: DeserializeOwned + 'static,
{
  if !is_tauri() {
    return;
  }
  let event = event.to_string();
  spawn_local(async move {
    let Ok(listen) = event_listen_fn() else {
      return;
    };
    let cb = Closure::wrap(Box::new(move |ev: JsValue| {
      if let Ok(payload) = Reflect::get(&ev, &JsValue::from_str("payload")) {
        if let Ok(json) = js_sys::JSON::stringify(&payload) {
          if let Some(text) = json.as_string() {
            if let Ok(value) = serde_json::from_str::<T>(&text) {
              handler(value);
            }
          }
        }
      }
    }) as Box<dyn FnMut(JsValue)>);
    let this = JsValue::NULL;
    let result = listen.call2(
      &this,
      &JsValue::from_str(&event),
      cb.as_ref().unchecked_ref(),
    );
    cb.forget();
    if let Ok(promise) = result {
      let _ = JsFuture::from(Promise::from(promise)).await;
    }
  });
}

/// Listen for a Tauri event whose payload is a string (e.g. `"navigate"`).
/// The handler is kept for the lifetime of the page (`forget`).
pub fn listen_string(event: &str, handler: impl Fn(String) + 'static) {
  if !is_tauri() {
    return;
  }
  let event = event.to_string();
  spawn_local(async move {
    let Ok(listen) = event_listen_fn() else {
      return;
    };
    let cb = Closure::wrap(Box::new(move |ev: JsValue| {
      if let Ok(payload) = Reflect::get(&ev, &JsValue::from_str("payload")) {
        if let Some(s) = payload.as_string() {
          handler(s);
        }
      }
    }) as Box<dyn FnMut(JsValue)>);
    let this = JsValue::NULL;
    let result = listen.call2(
      &this,
      &JsValue::from_str(&event),
      cb.as_ref().unchecked_ref(),
    );
    // Keep the callback alive for the app lifetime.
    cb.forget();
    if let Ok(promise) = result {
      let _ = JsFuture::from(Promise::from(promise)).await;
    }
  });
}

fn event_listen_fn() -> Result<Function, String> {
  let window = web_sys::window().ok_or_else(|| "no window".to_string())?;
  let tauri = Reflect::get(&window, &JsValue::from_str("__TAURI__"))
    .map_err(|_| "Tauri global missing".to_string())?;
  if tauri.is_undefined() || tauri.is_null() {
    return Err("Not running inside Teapot desktop app".into());
  }
  let event_mod = Reflect::get(&tauri, &JsValue::from_str("event"))
    .map_err(|_| "Tauri.event missing".to_string())?;
  let listen = Reflect::get(&event_mod, &JsValue::from_str("listen"))
    .map_err(|_| "Tauri.event.listen missing".to_string())?;
  listen
    .dyn_into::<Function>()
    .map_err(|_| "listen is not a function".to_string())
}

/// Trigger a browser download for text content (log export).
pub fn download_text(filename: &str, content: &str) -> Result<(), String> {
  let window = web_sys::window().ok_or_else(|| "no window".to_string())?;
  let document = window.document().ok_or_else(|| "no document".to_string())?;

  let blob_parts = js_sys::Array::new();
  blob_parts.push(&JsValue::from_str(content));
  let blob_props = web_sys::BlobPropertyBag::new();
  blob_props.set_type("text/plain;charset=utf-8");
  let blob = web_sys::Blob::new_with_str_sequence_and_options(&blob_parts, &blob_props)
    .map_err(|e| format!("blob: {e:?}"))?;

  let url =
    web_sys::Url::create_object_url_with_blob(&blob).map_err(|e| format!("object url: {e:?}"))?;

  let a = document
    .create_element("a")
    .map_err(|e| format!("create a: {e:?}"))?
    .dyn_into::<web_sys::HtmlAnchorElement>()
    .map_err(|_| "not an anchor".to_string())?;
  a.set_href(&url);
  a.set_download(filename);
  a.click();
  let _ = web_sys::Url::revoke_object_url(&url);
  Ok(())
}

// --- DTO mirrors (camelCase, matching app-tauri) ---

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerStatus {
  pub running: bool,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AppConfigDto {
  pub listen: String,
  pub api_key: String,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
  pub name: String,
  pub version: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveConfigArgs {
  pub config: AppConfigDto,
}

#[derive(Debug, Serialize)]
pub struct SetLocaleArgs {
  pub locale: String,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheck {
  pub available: bool,
  pub current_version: String,
  pub version: Option<String>,
  pub notes: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DownloadProgress {
  pub downloaded: u64,
  pub content_length: Option<u64>,
}
