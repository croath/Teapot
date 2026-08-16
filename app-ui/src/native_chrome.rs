//! Desktop-native chrome: labels are not selectable/copyable.

use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use web_sys::Event;

const EDITABLE: &str =
  "input, textarea, select, [contenteditable=\"\"], [contenteditable=\"true\"], .allow-select";

/// Block copy/cut/select of chrome text and hide the webview context menu.
pub fn install() {
  let Some(document) = web_sys::window().and_then(|w| w.document()) else {
    return;
  };

  listen(&document, "copy", |e| {
    if !is_editable(&e) {
      e.prevent_default();
    }
  });
  listen(&document, "cut", |e| {
    if !is_editable(&e) {
      e.prevent_default();
    }
  });
  listen(&document, "selectstart", |e| {
    if !is_editable(&e) {
      e.prevent_default();
    }
  });
  listen(&document, "dragstart", |e| {
    e.prevent_default();
  });

  // WKWebView / WebView2 default menu is Reload + Inspect Element.
  listen(&document, "contextmenu", |e| {
    e.prevent_default();
  });
}

fn is_editable(event: &Event) -> bool {
  event
    .target()
    .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
    .and_then(|el| el.closest(EDITABLE).ok().flatten())
    .is_some()
}

fn listen(document: &web_sys::Document, name: &str, handler: impl FnMut(Event) + 'static) {
  let closure = Closure::wrap(Box::new(handler) as Box<dyn FnMut(Event)>);
  let _ = document.add_event_listener_with_callback(name, closure.as_ref().unchecked_ref());
  closure.forget();
}
