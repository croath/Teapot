//! TelemetryDeck Ingest API v2 client (install + DAU + session).
//!
//! Empty `TELEMETRYDECK_APP_ID` disables sending. Identity is a local UUID
//! hashed with SHA-256 before POST. Never send serial numbers or hardware UUIDs.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::Duration;

use serde::Serialize;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Manager};

const INGEST_URL: &str = "https://nom.telemetrydeck.com/v2/";
const CLIENT_ID_FILE: &str = "telemetry_client_id";
const DAILY_ACTIVE_FILE: &str = "telemetry_last_daily_active";
const DAILY_RECHECK: Duration = Duration::from_secs(30 * 60);
const HTTP_TIMEOUT: Duration = Duration::from_secs(15);
const SDK_NAME: &str = "teapot-rust";

#[derive(Serialize)]
struct IngestSignal {
  #[serde(rename = "appID")]
  app_id: String,
  #[serde(rename = "clientUser")]
  client_user: String,
  #[serde(rename = "sessionID")]
  session_id: String,
  #[serde(rename = "type")]
  signal_type: String,
  #[serde(rename = "isTestMode", skip_serializing_if = "is_false")]
  is_test_mode: bool,
  #[serde(rename = "telemetryClientVersion")]
  telemetry_client_version: String,
  payload: BTreeMap<String, String>,
}

fn is_false(value: &bool) -> bool {
  !*value
}

/// Fire-and-forget launch signals. Returns immediately; never blocks the UI.
pub fn spawn_launch_signals(app: &AppHandle) {
  let Some(app_id) = baked_app_id() else {
    tracing::debug!("TelemetryDeck disabled (empty TELEMETRYDECK_APP_ID)");
    return;
  };

  let data_dir = match app.path().app_data_dir() {
    Ok(dir) => dir,
    Err(error) => {
      tracing::warn!(%error, "TelemetryDeck: resolve app data dir failed");
      return;
    }
  };

  let locale = crate::i18n::load_locale(app).id().to_string();
  let payload = build_payload(&locale);

  if let Err(error) = thread::Builder::new()
    .name("telemetrydeck".into())
    .spawn(move || run_worker(app_id, data_dir, payload))
  {
    tracing::warn!(%error, "TelemetryDeck: spawn worker failed");
  }
}

fn baked_app_id() -> Option<String> {
  let id = option_env!("TELEMETRYDECK_APP_ID").unwrap_or("").trim();
  (!id.is_empty()).then(|| id.to_string())
}

fn resolve_test_mode() -> bool {
  match option_env!("TELEMETRYDECK_TEST_MODE").map(str::trim) {
    Some("true") | Some("1") => true,
    Some("false") | Some("0") => false,
    _ => cfg!(debug_assertions),
  }
}

fn run_worker(app_id: String, data_dir: PathBuf, payload: BTreeMap<String, String>) {
  if let Err(error) = fs::create_dir_all(&data_dir) {
    tracing::warn!(%error, "TelemetryDeck: create app data dir failed");
    return;
  }

  let session_id = uuid::Uuid::new_v4().to_string();
  let (raw_client, is_new_install) = load_or_create_client_id(&data_dir);
  let client_user = sha256_hex(&raw_client);
  let test_mode = resolve_test_mode();

  let mut types = vec!["TelemetryDeck.Session.started"];
  if is_new_install {
    types.insert(0, "App.installed");
  }
  if should_send_daily(&data_dir) {
    types.push("App.dailyActive");
  }

  let sent_daily = types.contains(&"App.dailyActive");
  if post_signals(
    &app_id,
    &client_user,
    &session_id,
    test_mode,
    &payload,
    &types,
  ) {
    if sent_daily {
      persist_daily_marker(&data_dir);
    }
    tracing::info!(http_status = 200, "TelemetryDeck launch signals accepted");
  }

  loop {
    thread::sleep(DAILY_RECHECK);
    if !should_send_daily(&data_dir) {
      continue;
    }
    if post_signals(
      &app_id,
      &client_user,
      &session_id,
      test_mode,
      &payload,
      &["App.dailyActive"],
    ) {
      persist_daily_marker(&data_dir);
      tracing::info!(http_status = 200, "TelemetryDeck daily-active accepted");
    }
  }
}

fn post_signals(
  app_id: &str,
  client_user: &str,
  session_id: &str,
  is_test_mode: bool,
  payload: &BTreeMap<String, String>,
  types: &[&str],
) -> bool {
  if types.is_empty() {
    return true;
  }

  let version = env!("CARGO_PKG_VERSION");
  let body: Vec<IngestSignal> = types
    .iter()
    .map(|signal_type| IngestSignal {
      app_id: app_id.to_string(),
      client_user: client_user.to_string(),
      session_id: session_id.to_string(),
      signal_type: (*signal_type).to_string(),
      is_test_mode,
      telemetry_client_version: format!("{SDK_NAME} {version}"),
      payload: payload.clone(),
    })
    .collect();

  let client = match reqwest::blocking::Client::builder()
    .timeout(HTTP_TIMEOUT)
    .user_agent(format!("{SDK_NAME}/{version}"))
    .build()
  {
    Ok(client) => client,
    Err(error) => {
      tracing::warn!(%error, "TelemetryDeck: http client failed");
      return false;
    }
  };

  match client
    .post(INGEST_URL)
    .header("Content-Type", "application/json; charset=utf-8")
    .json(&body)
    .send()
  {
    Ok(response) => {
      let status = response.status();
      if status.is_success() {
        true
      } else {
        tracing::warn!(
          http_status = status.as_u16(),
          "TelemetryDeck ingest rejected"
        );
        false
      }
    }
    Err(error) => {
      tracing::warn!(%error, "TelemetryDeck ingest failed");
      false
    }
  }
}

fn load_or_create_client_id(data_dir: &Path) -> (String, bool) {
  let path = data_dir.join(CLIENT_ID_FILE);
  if let Ok(existing) = fs::read_to_string(&path) {
    let trimmed = existing.trim();
    if !trimmed.is_empty() {
      return (trimmed.to_string(), false);
    }
  }

  let id = uuid::Uuid::new_v4().to_string();
  if let Err(error) = fs::write(&path, format!("{id}\n")) {
    tracing::warn!(%error, "TelemetryDeck: persist client id failed");
  }
  (id, true)
}

fn should_send_daily(data_dir: &Path) -> bool {
  let today = local_date();
  match fs::read_to_string(data_dir.join(DAILY_ACTIVE_FILE)) {
    Ok(prev) => prev.trim() != today,
    Err(_) => true,
  }
}

fn persist_daily_marker(data_dir: &Path) {
  let path = data_dir.join(DAILY_ACTIVE_FILE);
  if let Err(error) = fs::write(&path, format!("{}\n", local_date())) {
    tracing::warn!(%error, "TelemetryDeck: persist daily-active marker failed");
  }
}

fn local_date() -> String {
  chrono::Local::now().format("%Y-%m-%d").to_string()
}

fn sha256_hex(input: &str) -> String {
  let digest = Sha256::digest(input.as_bytes());
  format!("{digest:x}")
}

fn build_payload(app_locale: &str) -> BTreeMap<String, String> {
  let version = env!("CARGO_PKG_VERSION");
  let git_hash = option_env!("TEAPOT_GIT_HASH").unwrap_or("unknown");
  let version_and_build = format!("{version} ({git_hash})");
  let platform = operating_system();
  let arch = std::env::consts::ARCH;
  let is_debug = if cfg!(debug_assertions) {
    "true"
  } else {
    "false"
  };
  let build_type = if cfg!(debug_assertions) {
    "debug"
  } else {
    "release"
  };
  let (sys_language, sys_region) = system_lang_region();
  let sys_ver = system_version();

  let mut payload = BTreeMap::new();
  put(&mut payload, "TelemetryDeck.AppInfo.version", version);
  put(&mut payload, "TelemetryDeck.AppInfo.buildNumber", git_hash);
  put(
    &mut payload,
    "TelemetryDeck.AppInfo.versionAndBuildNumber",
    &version_and_build,
  );
  put(&mut payload, "appVersion", version);
  put(&mut payload, "buildNumber", git_hash);
  put(&mut payload, "Teapot.App.version", version);
  put(&mut payload, "Teapot.App.buildNumber", git_hash);
  put(&mut payload, "Teapot.App.gitHash", git_hash);
  put(&mut payload, "Teapot.App.locale", app_locale);

  put(&mut payload, "TelemetryDeck.Device.platform", platform);
  put(
    &mut payload,
    "TelemetryDeck.Device.operatingSystem",
    platform,
  );
  put(&mut payload, "TelemetryDeck.Device.architecture", arch);
  put(
    &mut payload,
    "TelemetryDeck.Device.systemFamily",
    system_family(),
  );
  put(&mut payload, "TelemetryDeck.Device.timeZone", time_zone());
  if let Some(brand) = device_brand() {
    put(&mut payload, "TelemetryDeck.Device.brand", brand);
  }
  if let Some(model) = model_identifier() {
    put(&mut payload, "TelemetryDeck.Device.modelName", &model);
    put(&mut payload, "Teapot.Device.modelIdentifier", &model);
  }
  if let Some(chip) = cpu_brand() {
    put(&mut payload, "Teapot.Device.chip", chip);
  }

  if let Some(ref ver) = sys_ver.display {
    put(&mut payload, "TelemetryDeck.Device.systemVersion", ver);
    put(&mut payload, "TelemetryDeck.Device.osVersion", ver);
  }
  if let Some(ref major) = sys_ver.major {
    put(
      &mut payload,
      "TelemetryDeck.Device.systemMajorVersion",
      major,
    );
  }
  if let Some(ref major_minor) = sys_ver.major_minor {
    put(
      &mut payload,
      "TelemetryDeck.Device.systemMajorMinorVersion",
      major_minor,
    );
  }
  if let Some(build) = sys_ver.build {
    put(&mut payload, "TelemetryDeck.Device.osBuild", build);
  }

  put(&mut payload, "TelemetryDeck.RunContext.isDebug", is_debug);
  put(
    &mut payload,
    "TelemetryDeck.RunContext.isSimulator",
    "false",
  );
  put(
    &mut payload,
    "TelemetryDeck.RunContext.targetEnvironment",
    "native",
  );
  put(&mut payload, "TelemetryDeck.RunContext.locale", app_locale);
  put(
    &mut payload,
    "TelemetryDeck.RunContext.language",
    app_locale,
  );
  if let Some(language) = sys_language {
    put(
      &mut payload,
      "TelemetryDeck.UserPreference.language",
      language,
    );
  }
  if let Some(region) = sys_region {
    put(&mut payload, "TelemetryDeck.UserPreference.region", region);
  }

  put(&mut payload, "TelemetryDeck.SDK.name", SDK_NAME);
  put(&mut payload, "TelemetryDeck.SDK.version", version);
  put(
    &mut payload,
    "TelemetryDeck.SDK.nameAndVersion",
    format!("{SDK_NAME} {version}"),
  );
  put(&mut payload, "TelemetryDeck.SDK.buildType", build_type);
  payload
}

fn put(map: &mut BTreeMap<String, String>, key: &str, value: impl Into<String>) {
  let value = value.into();
  if !value.is_empty() {
    map.insert(key.to_string(), value);
  }
}

fn operating_system() -> &'static str {
  match std::env::consts::OS {
    "macos" => "macOS",
    "windows" => "Windows",
    "linux" => "Linux",
    other => other,
  }
}

fn system_family() -> &'static str {
  match std::env::consts::OS {
    "macos" | "ios" => "darwin",
    "windows" => "windows",
    "linux" => "linux",
    other => other,
  }
}

fn device_brand() -> Option<&'static str> {
  match std::env::consts::OS {
    "macos" | "ios" => Some("Apple"),
    "windows" => Some("Microsoft"),
    _ => None,
  }
}

struct SystemVersion {
  display: Option<String>,
  major: Option<String>,
  major_minor: Option<String>,
  build: Option<String>,
}

fn system_version() -> SystemVersion {
  #[cfg(target_os = "macos")]
  {
    let product = command_stdout("sw_vers", &["-productVersion"]);
    let build = command_stdout("sw_vers", &["-buildVersion"]);
    let display = product.as_ref().map(|v| format!("macOS {v}"));
    split_os_version(display, product, build)
  }

  #[cfg(target_os = "windows")]
  {
    let raw = command_stdout("cmd", &["/C", "ver"]);
    let numeric = raw.as_deref().and_then(extract_windows_version);
    return split_os_version(raw, numeric, None);
  }

  #[cfg(target_os = "linux")]
  {
    let pretty = linux_pretty_name();
    return split_os_version(pretty.clone(), pretty, None);
  }

  #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
  {
    split_os_version(None, None, None)
  }
}

fn split_os_version(
  display: Option<String>,
  numeric: Option<String>,
  build: Option<String>,
) -> SystemVersion {
  let numeric = numeric.as_deref().and_then(|s| {
    s.split_whitespace()
      .last()
      .filter(|part| part.chars().next().is_some_and(|c| c.is_ascii_digit()))
      .map(str::to_string)
  });
  let (major, major_minor) = match numeric.as_deref() {
    Some(ver) => {
      let mut parts = ver.split('.');
      let major = parts.next().map(str::to_string);
      let minor = parts.next();
      let major_minor = match (major.as_deref(), minor) {
        (Some(major), Some(minor)) => Some(format!("{major}.{minor}")),
        _ => major.clone(),
      };
      (major, major_minor)
    }
    None => (None, None),
  };
  SystemVersion {
    display,
    major,
    major_minor,
    build,
  }
}

fn model_identifier() -> Option<String> {
  #[cfg(target_os = "macos")]
  {
    command_stdout("sysctl", &["-n", "hw.model"])
  }
  #[cfg(not(target_os = "macos"))]
  {
    None
  }
}

fn cpu_brand() -> Option<String> {
  #[cfg(target_os = "macos")]
  {
    command_stdout("sysctl", &["-n", "machdep.cpu.brand_string"])
  }
  #[cfg(not(target_os = "macos"))]
  {
    None
  }
}

fn time_zone() -> String {
  if let Ok(tz) = std::env::var("TZ") {
    let trimmed = tz.trim();
    if !trimmed.is_empty() {
      return trimmed.to_string();
    }
  }
  if let Ok(link) = fs::read_link("/etc/localtime")
    && let Some(name) = link.to_str().and_then(|p| p.split("/zoneinfo/").nth(1))
  {
    return name.to_string();
  }
  chrono::Local::now().format("%:z").to_string()
}

fn system_lang_region() -> (Option<String>, Option<String>) {
  let Ok(raw) = std::env::var("LC_ALL").or_else(|_| std::env::var("LANG")) else {
    return (None, None);
  };
  let tag = raw.split('.').next().unwrap_or(raw.as_str()).trim();
  if tag.is_empty() || tag == "C" || tag == "POSIX" {
    return (None, None);
  }
  let mut parts = tag.split(['_', '-']);
  let language = parts.next().filter(|s| !s.is_empty()).map(str::to_string);
  let region = parts.next().filter(|s| !s.is_empty()).map(str::to_string);
  (language, region)
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn command_stdout(program: &str, args: &[&str]) -> Option<String> {
  let output = Command::new(program).args(args).output().ok()?;
  if !output.status.success() {
    return None;
  }
  let text = String::from_utf8(output.stdout).ok()?;
  let trimmed = text.trim();
  (!trimmed.is_empty()).then(|| trimmed.to_string())
}

#[cfg(target_os = "linux")]
fn linux_pretty_name() -> Option<String> {
  let text = fs::read_to_string("/etc/os-release").ok()?;
  for line in text.lines() {
    let line = line.trim();
    if let Some(value) = line.strip_prefix("PRETTY_NAME=") {
      return Some(unquote_os_release(value));
    }
  }
  None
}

#[cfg(target_os = "linux")]
fn unquote_os_release(value: &str) -> String {
  let value = value.trim();
  let bytes = value.as_bytes();
  if bytes.len() >= 2 {
    let quote = bytes[0];
    if (quote == b'"' || quote == b'\'') && bytes[bytes.len() - 1] == quote {
      return value[1..value.len() - 1].to_string();
    }
  }
  value.to_string()
}

#[cfg(target_os = "windows")]
fn extract_windows_version(raw: &str) -> Option<String> {
  let start = raw.find(|c: char| c.is_ascii_digit())?;
  let rest = &raw[start..];
  let end = rest
    .find(|c: char| !c.is_ascii_digit() && c != '.')
    .unwrap_or(rest.len());
  let ver = rest[..end].trim_end_matches('.');
  (!ver.is_empty()).then(|| ver.to_string())
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::Value;

  fn sample_signal(is_test_mode: bool) -> IngestSignal {
    let mut payload = BTreeMap::new();
    payload.insert("TelemetryDeck.Device.platform".into(), "macOS".into());
    IngestSignal {
      app_id: "app-id".into(),
      client_user: "deadbeef".into(),
      session_id: "sess".into(),
      signal_type: "App.dailyActive".into(),
      is_test_mode,
      telemetry_client_version: "teapot-rust 0.1.2".into(),
      payload,
    }
  }

  #[test]
  fn ingest_json_uses_app_id_casing() {
    let json = serde_json::to_value(sample_signal(true)).unwrap();
    let obj = json.as_object().unwrap();
    assert!(obj.contains_key("appID"));
    assert!(!obj.contains_key("appId"));
    assert!(obj.contains_key("clientUser"));
    assert!(obj.contains_key("sessionID"));
    assert_eq!(
      obj.get("type").and_then(Value::as_str),
      Some("App.dailyActive")
    );
    assert_eq!(obj.get("isTestMode").and_then(Value::as_bool), Some(true));
  }

  #[test]
  fn omits_is_test_mode_when_false() {
    let json = serde_json::to_value(sample_signal(false)).unwrap();
    assert!(json.get("isTestMode").is_none());
  }

  #[test]
  fn hashes_client_user_as_sha256_hex() {
    let hex = sha256_hex("teapot");
    assert_eq!(hex.len(), 64);
    assert!(hex.chars().all(|c| c.is_ascii_hexdigit()));
    assert_eq!(hex, sha256_hex("teapot"));
    assert_ne!(hex, sha256_hex("other"));
  }

  #[test]
  fn daily_marker_skips_same_local_day() {
    let dir = std::env::temp_dir().join(format!("teapot-telemetry-test-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&dir).unwrap();
    assert!(should_send_daily(&dir));
    persist_daily_marker(&dir);
    assert!(!should_send_daily(&dir));
    fs::write(dir.join(DAILY_ACTIVE_FILE), "1999-01-01\n").unwrap();
    assert!(should_send_daily(&dir));
    let _ = fs::remove_dir_all(&dir);
  }

  #[test]
  fn payload_includes_required_dimensions() {
    let payload = build_payload("zh-Hans");
    assert_eq!(
      payload
        .get("TelemetryDeck.AppInfo.version")
        .map(String::as_str),
      Some(env!("CARGO_PKG_VERSION"))
    );
    assert_eq!(
      payload.get("Teapot.App.locale").map(String::as_str),
      Some("zh-Hans")
    );
    assert!(payload.contains_key("TelemetryDeck.Device.platform"));
    assert_eq!(
      payload
        .get("TelemetryDeck.RunContext.targetEnvironment")
        .map(String::as_str),
      Some("native")
    );
  }

  #[cfg(target_os = "windows")]
  #[test]
  fn parses_windows_ver_output() {
    let raw = "Microsoft Windows [Version 10.0.26100.4652]";
    assert_eq!(
      extract_windows_version(raw).as_deref(),
      Some("10.0.26100.4652")
    );
  }
}
