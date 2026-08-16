//! Supported UI locales.

/// Interface languages: English, Japanese, Simplified Chinese, Traditional Chinese, Korean.
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
  pub const ALL: [Locale; 5] = [
    Locale::En,
    Locale::Ja,
    Locale::ZhHans,
    Locale::ZhHant,
    Locale::Ko,
  ];

  /// Canonical id stored in localStorage / the desktop locale file.
  pub fn id(self) -> &'static str {
    match self {
      Locale::En => "en",
      Locale::Ja => "ja",
      Locale::ZhHans => "zh-Hans",
      Locale::ZhHant => "zh-Hant",
      Locale::Ko => "ko",
    }
  }

  /// BCP 47 tag for `<html lang>`.
  pub fn html_lang(self) -> &'static str {
    match self {
      Locale::En => "en",
      Locale::Ja => "ja",
      Locale::ZhHans => "zh-CN",
      Locale::ZhHant => "zh-TW",
      Locale::Ko => "ko",
    }
  }

  /// Autonym shown in the language picker (not translated).
  pub fn native_name(self) -> &'static str {
    match self {
      Locale::En => "English",
      Locale::Ja => "日本語",
      Locale::ZhHans => "简体中文",
      Locale::ZhHant => "繁體中文",
      Locale::Ko => "한국어",
    }
  }

  /// Parse a stored canonical id (or a common alias).
  pub fn parse(id: &str) -> Option<Self> {
    match id.trim() {
      "en" => Some(Locale::En),
      "ja" => Some(Locale::Ja),
      "zh-Hans" | "zh-CN" | "zh" => Some(Locale::ZhHans),
      "zh-Hant" | "zh-TW" => Some(Locale::ZhHant),
      "ko" => Some(Locale::Ko),
      _ => None,
    }
  }

  /// Best-effort match from a browser / OS language tag.
  pub fn from_tag(tag: &str) -> Self {
    let lower = tag.trim().replace('_', "-").to_ascii_lowercase();
    if lower.starts_with("ja") {
      return Locale::Ja;
    }
    if lower.starts_with("ko") {
      return Locale::Ko;
    }
    if lower.starts_with("zh") {
      if lower.contains("hant")
        || lower.contains("-tw")
        || lower.contains("-hk")
        || lower.contains("-mo")
      {
        return Locale::ZhHant;
      }
      return Locale::ZhHans;
    }
    Locale::En
  }
}

#[cfg(test)]
mod tests {
  use super::Locale;

  #[test]
  fn parse_canonical_and_aliases() {
    assert_eq!(Locale::parse("en"), Some(Locale::En));
    assert_eq!(Locale::parse("ja"), Some(Locale::Ja));
    assert_eq!(Locale::parse("zh-Hans"), Some(Locale::ZhHans));
    assert_eq!(Locale::parse("zh-CN"), Some(Locale::ZhHans));
    assert_eq!(Locale::parse("zh-Hant"), Some(Locale::ZhHant));
    assert_eq!(Locale::parse("zh-TW"), Some(Locale::ZhHant));
    assert_eq!(Locale::parse("ko"), Some(Locale::Ko));
    assert_eq!(Locale::parse("fr"), None);
  }

  #[test]
  fn from_navigator_tags() {
    assert_eq!(Locale::from_tag("en-US"), Locale::En);
    assert_eq!(Locale::from_tag("ja-JP"), Locale::Ja);
    assert_eq!(Locale::from_tag("zh-CN"), Locale::ZhHans);
    assert_eq!(Locale::from_tag("zh-Hans-CN"), Locale::ZhHans);
    assert_eq!(Locale::from_tag("zh-TW"), Locale::ZhHant);
    assert_eq!(Locale::from_tag("zh-Hant-HK"), Locale::ZhHant);
    assert_eq!(Locale::from_tag("zh-HK"), Locale::ZhHant);
    assert_eq!(Locale::from_tag("ko-KR"), Locale::Ko);
    assert_eq!(Locale::from_tag("fr-FR"), Locale::En);
  }
}
