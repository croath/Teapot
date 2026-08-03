//! Shared application state for Axum handlers.

use std::sync::Arc;

use crate::agents::AgentRunner;
use crate::config::Config;

#[derive(Clone)]
pub struct AppState {
  pub config: Arc<Config>,
  pub runner: AgentRunner,
}

impl AppState {
  pub fn new(config: Config) -> Self {
    let config = Arc::new(config);
    let runner = AgentRunner::new(Arc::clone(&config));
    Self { config, runner }
  }
}
