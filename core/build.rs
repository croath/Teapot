//! Bake Antigravity OAuth client credentials at compile time.
//!
//! Precedence (first non-empty wins):
//! 1. Process environment
//! 2. `core/.env`
//! 3. workspace `.env`
//!
//! Names: `ANTIGRAVITY_CLIENT_ID` / `ANTIGRAVITY_CLIENT_SECRET`.

use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

const CLIENT_ID_KEY: &str = "ANTIGRAVITY_CLIENT_ID";
const CLIENT_SECRET_KEY: &str = "ANTIGRAVITY_CLIENT_SECRET";

fn main() {
  let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
  let workspace_root = manifest_dir.parent().unwrap_or(&manifest_dir);
  let crate_env = manifest_dir.join(".env");
  let workspace_env = workspace_root.join(".env");

  println!("cargo:rerun-if-changed={}", workspace_env.display());
  println!("cargo:rerun-if-changed={}", crate_env.display());
  println!("cargo:rerun-if-env-changed={CLIENT_ID_KEY}");
  println!("cargo:rerun-if-env-changed={CLIENT_SECRET_KEY}");

  let mut file_vars = HashMap::new();
  if let Some(vars) = load_dotenv(&workspace_env) {
    file_vars.extend(vars);
  }
  if let Some(vars) = load_dotenv(&crate_env) {
    file_vars.extend(vars);
  }

  let client_id = require_var(&file_vars, CLIENT_ID_KEY);
  let client_secret = require_var(&file_vars, CLIENT_SECRET_KEY);

  println!("cargo:rustc-env={CLIENT_ID_KEY}={client_id}");
  println!("cargo:rustc-env={CLIENT_SECRET_KEY}={client_secret}");
}

fn require_var(file_vars: &HashMap<String, String>, name: &str) -> String {
  resolve_var(file_vars, name).unwrap_or_else(|| {
    panic!(
      "{name} is required at build time. Set it in the environment or in a `.env` file \
       at the workspace root (see `.env.example`)."
    )
  })
}

fn resolve_var(file_vars: &HashMap<String, String>, name: &str) -> Option<String> {
  if let Ok(v) = env::var(name) {
    let trimmed = v.trim();
    if !trimmed.is_empty() {
      return Some(trimmed.to_string());
    }
  }
  file_vars.get(name).and_then(|v| {
    let trimmed = v.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
  })
}

fn load_dotenv(path: &Path) -> Option<HashMap<String, String>> {
  let text = fs::read_to_string(path).ok()?;
  Some(parse_dotenv(&text))
}

fn parse_dotenv(text: &str) -> HashMap<String, String> {
  let mut out = HashMap::new();
  for raw in text.lines() {
    let line = raw.trim();
    if line.is_empty() || line.starts_with('#') {
      continue;
    }
    let line = line.strip_prefix("export ").unwrap_or(line).trim();
    let Some((key, value)) = line.split_once('=') else {
      continue;
    };
    let key = key.trim();
    if key.is_empty() {
      continue;
    }
    out.insert(key.to_string(), unquote(value.trim()));
  }
  out
}

fn unquote(value: &str) -> String {
  let bytes = value.as_bytes();
  if bytes.len() >= 2 {
    let quote = bytes[0];
    if (quote == b'"' || quote == b'\'') && bytes[bytes.len() - 1] == quote {
      return value[1..value.len() - 1].to_string();
    }
  }
  value.to_string()
}
