//! Native-shell copy keyed by [`Msg`]. Each locale is an exhaustive match.

use std::fs;
use std::path::PathBuf;

use tauri::{AppHandle, Manager};

const LOCALE_FILE: &str = "locale";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Locale {
  #[default]
  En,
  Ja,
  ZhHans,
  ZhHant,
  Ko,
}

impl Locale {
  pub fn parse(id: &str) -> Self {
    match id.trim() {
      "ja" => Locale::Ja,
      "zh-Hans" | "zh-CN" | "zh" => Locale::ZhHans,
      "zh-Hant" | "zh-TW" => Locale::ZhHant,
      "ko" => Locale::Ko,
      _ => Locale::En,
    }
  }

  pub fn id(self) -> &'static str {
    match self {
      Locale::En => "en",
      Locale::Ja => "ja",
      Locale::ZhHans => "zh-Hans",
      Locale::ZhHant => "zh-Hant",
      Locale::Ko => "ko",
    }
  }

  pub fn t(self, msg: Msg) -> &'static str {
    match self {
      Locale::En => msg.en(),
      Locale::Ja => msg.ja(),
      Locale::ZhHans => msg.zh_hans(),
      Locale::ZhHant => msg.zh_hant(),
      Locale::Ko => msg.ko(),
    }
  }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Msg {
  MenuSettings,
  MenuAbout,
  MenuDebug,
  MenuCheckUpdates,
  #[cfg_attr(target_os = "macos", allow(dead_code))]
  MenuFile,
  MenuEdit,
  MenuWindow,
  MenuHelp,
  CloseGuardTitle,
  CloseGuardMessage,
}

impl Msg {
  fn en(self) -> &'static str {
    match self {
      Msg::MenuSettings => "Settings…",
      Msg::MenuAbout => "About Teapot",
      Msg::MenuDebug => "Debug Logs",
      Msg::MenuCheckUpdates => "Check for Updates…",
      Msg::MenuFile => "File",
      Msg::MenuEdit => "Edit",
      Msg::MenuWindow => "Window",
      Msg::MenuHelp => "Help",
      Msg::CloseGuardTitle => "teapotx is running",
      Msg::CloseGuardMessage => "You should shut down teapotx first.",
    }
  }

  fn ja(self) -> &'static str {
    match self {
      Msg::MenuSettings => "設定…",
      Msg::MenuAbout => "Teapotについて",
      Msg::MenuDebug => "デバッグログ",
      Msg::MenuCheckUpdates => "アップデートを確認…",
      Msg::MenuFile => "ファイル",
      Msg::MenuEdit => "編集",
      Msg::MenuWindow => "ウィンドウ",
      Msg::MenuHelp => "ヘルプ",
      Msg::CloseGuardTitle => "teapotx が実行中です",
      Msg::CloseGuardMessage => "先に teapotx を停止してください。",
    }
  }

  fn zh_hans(self) -> &'static str {
    match self {
      Msg::MenuSettings => "设置…",
      Msg::MenuAbout => "关于 Teapot",
      Msg::MenuDebug => "调试日志",
      Msg::MenuCheckUpdates => "检查更新…",
      Msg::MenuFile => "文件",
      Msg::MenuEdit => "编辑",
      Msg::MenuWindow => "窗口",
      Msg::MenuHelp => "帮助",
      Msg::CloseGuardTitle => "teapotx 正在运行",
      Msg::CloseGuardMessage => "请先关闭 teapotx。",
    }
  }

  fn zh_hant(self) -> &'static str {
    match self {
      Msg::MenuSettings => "設定…",
      Msg::MenuAbout => "關於 Teapot",
      Msg::MenuDebug => "除錯記錄",
      Msg::MenuCheckUpdates => "檢查更新…",
      Msg::MenuFile => "檔案",
      Msg::MenuEdit => "編輯",
      Msg::MenuWindow => "視窗",
      Msg::MenuHelp => "輔助說明",
      Msg::CloseGuardTitle => "teapotx 正在執行",
      Msg::CloseGuardMessage => "請先關閉 teapotx。",
    }
  }

  fn ko(self) -> &'static str {
    match self {
      Msg::MenuSettings => "설정…",
      Msg::MenuAbout => "Teapot 정보",
      Msg::MenuDebug => "디버그 로그",
      Msg::MenuCheckUpdates => "업데이트 확인…",
      Msg::MenuFile => "파일",
      Msg::MenuEdit => "편집",
      Msg::MenuWindow => "윈도우",
      Msg::MenuHelp => "도움말",
      Msg::CloseGuardTitle => "teapotx가 실행 중입니다",
      Msg::CloseGuardMessage => "먼저 teapotx를 중지하십시오.",
    }
  }
}

fn locale_path(app: &AppHandle) -> Result<PathBuf, String> {
  let dir = app
    .path()
    .app_config_dir()
    .map_err(|e| format!("resolve app config dir: {e}"))?;
  fs::create_dir_all(&dir).map_err(|e| format!("create config dir: {e}"))?;
  Ok(dir.join(LOCALE_FILE))
}

pub fn load_locale(app: &AppHandle) -> Locale {
  let Ok(path) = locale_path(app) else {
    return Locale::En;
  };
  let Ok(text) = fs::read_to_string(path) else {
    return Locale::En;
  };
  Locale::parse(&text)
}

pub fn save_locale(app: &AppHandle, locale: Locale) -> Result<(), String> {
  let path = locale_path(app)?;
  fs::write(path, locale.id()).map_err(|e| format!("write locale: {e}"))
}
