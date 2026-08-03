//! Root application: Server control + Playground only.

use leptos::prelude::*;
use leptos_meta::{provide_meta_context, Stylesheet, Title};
use leptos_router::components::{Route, Router, Routes};
use leptos_router::path;

use crate::pages::{PlaygroundPage, ServerPage};

#[component]
pub fn App() -> impl IntoView {
  provide_meta_context();

  view! {
    <Stylesheet id="app-styles" href="styles/app.css" />
    <Title text="Teaport" />
    <Router>
      <div class="mx-auto max-w-4xl px-6 py-6">
        <header class="mb-6 flex flex-wrap items-center justify-between gap-4">
          <a
            class="text-lg font-bold text-slate-100 no-underline hover:text-white hover:no-underline"
            href="/"
          >
            "Teaport"
          </a>
          <nav class="flex gap-4 text-sm">
            <a class="text-muted hover:text-slate-100" href="/">"Server"</a>
            <a class="text-muted hover:text-slate-100" href="/playground">"Playground"</a>
          </nav>
        </header>
        <main>
          <Routes fallback=|| {
            view! {
              <p class="text-muted">"Page not found."</p>
            }
          }>
            <Route path=path!("/") view=ServerPage />
            <Route path=path!("/playground") view=PlaygroundPage />
          </Routes>
        </main>
      </div>
    </Router>
  }
}
