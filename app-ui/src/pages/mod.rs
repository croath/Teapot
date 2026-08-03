//! UI pages: home (serve switch) + macOS-style settings hub.

mod home;
mod settings;

pub use home::HomePage;
pub use settings::{
  SettingsAboutPage, SettingsDebugPage, SettingsGeneratePage, SettingsPage,
};
