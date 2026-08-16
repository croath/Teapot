//! Bake TelemetryDeck App ID / test mode and a git short hash at compile time.
//!
//! Precedence (first non-empty wins):
//! 1. Process environment
//! 2. `app-tauri/.env`
//! 3. workspace `.env`
//!
//! Empty `TELEMETRYDECK_APP_ID` disables sending (safe local / CI builds).

use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const APP_ID_KEY: &str = "TELEMETRYDECK_APP_ID";
const TEST_MODE_KEY: &str = "TELEMETRYDECK_TEST_MODE";

fn main() {
  tauri_build::build();

  let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
  let workspace_root = manifest_dir.parent().unwrap_or(&manifest_dir);
  let crate_env = manifest_dir.join(".env");
  let workspace_env = workspace_root.join(".env");
  let git_head = workspace_root.join(".git/HEAD");

  println!("cargo:rerun-if-changed={}", workspace_env.display());
  println!("cargo:rerun-if-changed={}", crate_env.display());
  println!("cargo:rerun-if-changed={}", git_head.display());
  println!("cargo:rerun-if-env-changed={APP_ID_KEY}");
  println!("cargo:rerun-if-env-changed={TEST_MODE_KEY}");
  println!("cargo:rerun-if-env-changed=GITHUB_SHA");

  let mut file_vars = HashMap::new();
  if let Some(vars) = load_dotenv(&workspace_env) {
    file_vars.extend(vars);
  }
  if let Some(vars) = load_dotenv(&crate_env) {
    file_vars.extend(vars);
  }

  let app_id = resolve_var(&file_vars, APP_ID_KEY).unwrap_or_default();
  println!("cargo:rustc-env={APP_ID_KEY}={app_id}");

  if let Some(test_mode) = resolve_var(&file_vars, TEST_MODE_KEY) {
    println!("cargo:rustc-env={TEST_MODE_KEY}={test_mode}");
  }

  println!("cargo:rustc-env=TEAPOT_GIT_HASH={}", git_short_hash());
}

fn git_short_hash() -> String {
  if let Ok(sha) = env::var("GITHUB_SHA") {
    let short: String = sha.trim().chars().take(7).collect();
    if !short.is_empty() {
      return short;
    }
  }

  Command::new("git")
    .args(["rev-parse", "--short", "HEAD"])
    .output()
    .ok()
    .and_then(|out| {
      if !out.status.success() {
        return None;
      }
      String::from_utf8(out.stdout).ok()
    })
    .map(|s| s.trim().to_string())
    .filter(|s| !s.is_empty())
    .unwrap_or_else(|| "unknown".into())
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
