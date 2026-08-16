use leptos::prelude::*;
use leptos_ui::clx;
use tw_merge::tw_merge;

mod components {
  use super::*;
  clx! {SwitchLabel, span, "text-sm font-medium"}
}

pub use components::*;

/// Liquid-glass toggle. Controlled via `checked` + `on_change`.
#[component]
pub fn Switch(
  #[prop(optional, into)] id: String,
  /// Current on/off state.
  #[prop(into)]
  checked: Signal<bool>,
  /// Fired when the user toggles the switch (not when `checked` changes programmatically).
  #[prop(into, optional)]
  on_change: Option<Callback<bool>>,
  #[prop(optional, into, default = Signal::stored(false))] disabled: Signal<bool>,
  #[prop(into, optional, default = Signal::stored("Toggle switch".to_string()))] aria_label: Signal<
    String,
  >,
  #[prop(into, optional)] class: String,
) -> impl IntoView {
  let state = move || {
    if checked.get() {
      "checked"
    } else {
      "unchecked"
    }
  };

  let track_class = tw_merge!(
    "inline-flex h-8 w-14 shrink-0 cursor-pointer items-center rounded-full border-2 border-transparent transition-[background-color,box-shadow,transform] duration-300 ease-out focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background disabled:cursor-not-allowed disabled:opacity-50 data-[state=checked]:bg-primary data-[state=unchecked]:bg-input/80 active:scale-[0.97]",
    class
  );

  view! {
    <button
      data-name="Switch"
      id=id
      type="button"
      role="switch"
      aria-checked=move || checked.get().to_string()
      aria-label=move || aria_label.get()
      data-state=state
      class=track_class
      prop:disabled=move || disabled.get()
      on:click=move |_| {
        if disabled.get_untracked() {
          return;
        }
        let next = !checked.get_untracked();
        if let Some(cb) = on_change {
          cb.run(next);
        }
      }
    >
      <span
        data-state=state
        class="pointer-events-none block size-6 rounded-full bg-background shadow-lg ring-0 transition-transform duration-300 ease-[cubic-bezier(0.22,1.4,0.36,1)] will-change-transform data-[state=checked]:translate-x-6 data-[state=unchecked]:translate-x-0.5"
      />
    </button>
  }
}
