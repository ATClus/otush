//! Settings store: JSON file load/save plus the `AppContext` accessors. (split from `settings.rs`; same keys, same behavior).

use super::defaults::{
    ensure_agent_defaults, ensure_post_process_defaults, ensure_transcription_provider_defaults,
    get_default_settings,
};
use super::migrations::{apply_settings_migrations, salvage_settings};
use super::schema::AppSettings;
use crate::context::AppContext;
use log::{debug, warn};

pub const SETTINGS_STORE_PATH: &str = "settings_store.json";

/// Read the whole store file as JSON; a missing or corrupt file yields an
/// empty object (the caller falls back to defaults).
fn read_store_at(path: &std::path::Path) -> serde_json::Value {
    match std::fs::read_to_string(path) {
        Ok(contents) => serde_json::from_str(&contents).unwrap_or_else(|e| {
            warn!("Failed to parse settings store ({e}); starting from defaults");
            serde_json::json!({})
        }),
        Err(_) => serde_json::json!({}),
    }
}

/// Persist the store file atomically (temp file + rename).
pub(crate) fn write_store_at(path: &std::path::Path, store: &serde_json::Value) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(metadata) = std::fs::metadata(parent) {
                let mut perms = metadata.permissions();
                if perms.mode() & 0o777 != 0o700 {
                    perms.set_mode(0o700);
                    let _ = std::fs::set_permissions(parent, perms);
                }
            }
        }
    }
    let tmp = path.with_extension("json.tmp");
    match serde_json::to_string_pretty(store) {
        Ok(contents) => {
            if std::fs::write(&tmp, contents).is_ok() {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if let Ok(metadata) = std::fs::metadata(&tmp) {
                        let mut perms = metadata.permissions();
                        perms.set_mode(0o600);
                        let _ = std::fs::set_permissions(&tmp, perms);
                    }
                }
                let _ = std::fs::rename(&tmp, path);
            } else {
                warn!("Failed to write settings store to {}", tmp.display());
            }
        }
        Err(e) => warn!("Failed to serialize settings store ({e})"),
    }
}

/// Convenience: build the store object with the `settings` key populated.
fn store_with_settings(settings: &AppSettings) -> serde_json::Value {
    serde_json::json!({ "settings": serde_json::to_value(settings).unwrap() })
}

/// Path-based core of [`get_settings`]; used by components that only know
/// their settings file path (e.g. managers), not the full [`AppContext`].
pub fn read_settings_from(path: &std::path::Path) -> AppSettings {
    let store = read_store_at(path);

    // Settings reads also persist one-time migrations. Migration helpers are
    // idempotent, so this converges after the first read of an older store.
    let mut settings = if let Some(settings_value) = store.get("settings") {
        let (mut settings, mut updated) =
            match serde_json::from_value::<AppSettings>(settings_value.clone()) {
                Ok(settings) => (settings, false),
                Err(e) => {
                    warn!("Failed to parse stored settings ({e}); salvaging valid fields");
                    (salvage_settings(settings_value), true)
                }
            };

        if apply_settings_migrations(&mut settings, settings_value) {
            updated = true;
        }

        // Merge in any bindings added since this store was written.
        for (key, value) in get_default_settings().bindings {
            if let std::collections::hash_map::Entry::Vacant(entry) = settings.bindings.entry(key) {
                debug!("Adding missing binding: {}", entry.key());
                entry.insert(value);
                updated = true;
            }
        }

        if updated {
            write_store_at(path, &store_with_settings(&settings));
        }

        settings
    } else {
        let default_settings = get_default_settings();
        write_store_at(path, &store_with_settings(&default_settings));
        default_settings
    };

    if ensure_post_process_defaults(&mut settings) {
        write_store_at(path, &store_with_settings(&settings));
    }
    if ensure_transcription_provider_defaults(&mut settings) {
        write_store_at(path, &store_with_settings(&settings));
    }
    if ensure_agent_defaults(&mut settings) {
        write_store_at(path, &store_with_settings(&settings));
    }

    settings
}

/// Path-based core of [`write_settings`].
pub fn write_settings_to(path: &std::path::Path, settings: &AppSettings) {
    write_store_at(path, &store_with_settings(settings));
}

pub fn get_settings(ctx: &AppContext) -> AppSettings {
    read_settings_from(&ctx.paths.settings_store_path())
}

pub fn write_settings(ctx: &AppContext, settings: AppSettings) {
    write_settings_to(&ctx.paths.settings_store_path(), &settings);
}
