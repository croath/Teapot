//! Root application: home switch + settings gear → macOS-style Settings.

use icons::Settings as SettingsIcon;
use leptos::prelude::*;
use leptos_meta::{Title, provide_meta_context};
use leptos_router::NavigateOptions;
use leptos_router::components::{A, Route, Router, Routes};
use leptos_router::hooks::{use_location, use_navigate};
use leptos_router::path;

use crate::pages::{
  HomePage, SettingsAboutPage, SettingsDebugPage, SettingsGeneratePage, SettingsPage,
};
use crate::tauri_bridge;

#[component]
pub fn App() -> impl IntoView {
  provide_meta_context();

  view! {
    <Title text="Teapot" />
    <Router>
      <div class="app-root">
        // macOS overlay titlebar drag region (traffic lights sit above content).
        <div class="titlebar-drag" data-tauri-drag-region></div>
        <AppChrome />

        <main class="app-main">
          <Routes fallback=|| {
            view! {
              <p class="text-muted-foreground text-sm p-6">"Page not found."</p>
            }
          }>
            <Route path=path!("/") view=HomePage />
            <Route path=path!("/settings") view=SettingsPage />
            <Route path=path!("/settings/generate") view=SettingsGeneratePage />
            <Route path=path!("/settings/debug") view=SettingsDebugPage />
            <Route path=path!("/settings/about") view=SettingsAboutPage />
            // Legacy redirects → settings panes
            <Route path=path!("/debug") view=RedirectToDebug />
            <Route path=path!("/about") view=RedirectToAbout />
          </Routes>
        </main>
      </div>
    </Router>
  }
}

#[component]
fn RedirectToDebug() -> impl IntoView {
  let navigate = use_navigate();
  Effect::new(move |_| {
    navigate("/settings/debug", NavigateOptions::default());
  });
  view! { <p class="text-muted-foreground text-sm p-6">"Opening Debug…"</p> }
}

#[component]
fn RedirectToAbout() -> impl IntoView {
  let navigate = use_navigate();
  Effect::new(move |_| {
    navigate("/settings/about", NavigateOptions::default());
  });
  view! { <p class="text-muted-foreground text-sm p-6">"Opening About…"</p> }
}

/// Settings gear (home) + bridge from native system menu → router.
#[component]
fn AppChrome() -> impl IntoView {
  let navigate = use_navigate();
  let location = use_location();

  Effect::new(move |_| {
    let navigate = navigate.clone();
    tauri_bridge::listen_string("navigate", move |path| {
      navigate(&path, NavigateOptions::default());
    });
  });

  // Hide gear while already inside Settings (toolbar has Done / back).
  let show_gear = move || {
    let path = location.pathname.get();
    !path.starts_with("/settings")
  };

  view! {
    <Show when=show_gear>
      <div class="app-settings-btn-wrap">
        <A href="/settings">
          <span class="app-settings-btn" role="img" aria-label="Open Settings">
            <SettingsIcon class="size-5".to_string() />
          </span>
        </A>
      </div>
    </Show>
  }
}
