//! Tray menu internationalization.
//!
//! Interim English-only implementation until gettext lands (Phase 7). The
//! lookup/fallback structure is preserved so adding languages later is purely
//! additive; the original compile-time generation from the frontend locale
//! files no longer applies (the webview frontend was removed).

use std::collections::HashMap;
use std::sync::OnceLock;

/// Localized tray menu strings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrayStrings {
    pub settings: String,
    pub check_updates: String,
    pub copy_last_transcript: String,
    pub quit: String,
    pub cancel: String,
    pub model: String,
    pub unload_model: String,
    pub secure_input_warning: String,
}

fn english() -> TrayStrings {
    TrayStrings {
        settings: "Settings".to_string(),
        check_updates: "Check for Updates".to_string(),
        copy_last_transcript: "Copy Last Transcript".to_string(),
        quit: "Quit Otush".to_string(),
        cancel: "Cancel".to_string(),
        model: "Model".to_string(),
        unload_model: "Unload Model".to_string(),
        secure_input_warning: String::new(),
    }
}

fn translations() -> &'static HashMap<&'static str, TrayStrings> {
    static TRANSLATIONS: OnceLock<HashMap<&'static str, TrayStrings>> = OnceLock::new();
    TRANSLATIONS.get_or_init(|| {
        let mut map = HashMap::new();
        map.insert("en", english());
        map
    })
}

/// Get localized tray menu strings based on the system locale.
///
/// Lookup order: exact locale → language code → English.
pub fn get_tray_translations(locale: Option<String>) -> TrayStrings {
    let normalized = locale
        .as_deref()
        .unwrap_or("en")
        .to_lowercase()
        .replace('_', "-");
    let language = normalized.split('-').next().unwrap_or("en");

    let map = translations();
    map.get(normalized.as_str())
        .or_else(|| map.get(language))
        .or_else(|| map.get("en"))
        .cloned()
        .expect("English translations must exist")
}

#[cfg(test)]
mod tests {
    use super::get_tray_translations;

    #[test]
    fn resolves_locale_fallbacks() {
        for (locale, expected) in [("fr-FR", "en"), ("zh-Hant-TW", "en"), ("XX-YY", "en")] {
            assert_eq!(
                get_tray_translations(Some(locale.into())),
                get_tray_translations(Some(expected.into())),
                "{locale} should resolve to {expected}"
            );
        }
    }
}
