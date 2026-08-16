//! Native menu copy for the five UI locales.

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
}

pub struct MenuCopy {
  pub settings: &'static str,
  pub about: &'static str,
  pub debug: &'static str,
  pub check_updates: &'static str,
  #[cfg_attr(target_os = "macos", allow(dead_code))]
  pub file: &'static str,
  pub edit: &'static str,
  pub window: &'static str,
  pub help: &'static str,
}

impl Locale {
  pub fn menu(self) -> MenuCopy {
    match self {
      Locale::En => MenuCopy {
        settings: "Settings…",
        about: "About Teapot",
        debug: "Debug Logs",
        check_updates: "Check for Updates…",
        file: "File",
        edit: "Edit",
        window: "Window",
        help: "Help",
      },
      Locale::Ja => MenuCopy {
        settings: "設定…",
        about: "Teapotについて",
        debug: "デバッグログ",
        check_updates: "アップデートを確認…",
        file: "ファイル",
        edit: "編集",
        window: "ウィンドウ",
        help: "ヘルプ",
      },
      Locale::ZhHans => MenuCopy {
        settings: "设置…",
        about: "关于 Teapot",
        debug: "调试日志",
        check_updates: "检查更新…",
        file: "文件",
        edit: "编辑",
        window: "窗口",
        help: "帮助",
      },
      Locale::ZhHant => MenuCopy {
        settings: "設定…",
        about: "關於 Teapot",
        debug: "除錯記錄",
        check_updates: "檢查更新…",
        file: "檔案",
        edit: "編輯",
        window: "視窗",
        help: "輔助說明",
      },
      Locale::Ko => MenuCopy {
        settings: "설정…",
        about: "Teapot 정보",
        debug: "디버그 로그",
        check_updates: "업데이트 확인…",
        file: "파일",
        edit: "편집",
        window: "윈도우",
        help: "도움말",
      },
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
