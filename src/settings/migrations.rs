//! Settings migrations and corruption salvage. (split from `settings.rs`; same keys, same behavior).

use super::defaults::{
    default_agents, default_post_process_providers, default_transcribe_gpu_device,
    default_transcription_providers, get_default_settings, CURRENT_SETTINGS_SCHEMA_VERSION,
};
use super::schema::*;
use log::warn;

/// Rebuilds settings from a store value that failed to deserialize as a whole.
/// Every stored field that is individually valid is kept; only broken values
/// (e.g. an enum variant written by a newer or older version) fall back to
/// their default. This means one bad field can never reset the rest of the
/// user's configuration (#1619).
pub(crate) fn salvage_settings(stored: &serde_json::Value) -> AppSettings {
    let Some(stored_map) = stored.as_object() else {
        warn!("Stored settings are not a JSON object; falling back to defaults");
        return get_default_settings();
    };

    let mut merged = serde_json::to_value(get_default_settings())
        .expect("default settings serialize to a JSON object");

    for (key, value) in stored_map {
        let previous = merged
            .as_object_mut()
            .expect("merged settings stay an object")
            .insert(key.clone(), value.clone());
        if serde_json::from_value::<AppSettings>(merged.clone()).is_err() {
            // Log only the key: values may hold secrets (e.g. API keys).
            warn!("Dropping invalid settings field '{key}', keeping its default");
            let map = merged
                .as_object_mut()
                .expect("merged settings stay an object");
            match previous {
                Some(previous) => map.insert(key.clone(), previous),
                None => map.remove(key),
            };
        }
    }

    serde_json::from_value(merged).unwrap_or_else(|e| {
        warn!("Failed to reassemble salvaged settings ({e}); falling back to defaults");
        get_default_settings()
    })
}

pub(crate) fn apply_settings_migrations(
    settings: &mut AppSettings,
    settings_value: &serde_json::Value,
) -> bool {
    let mut updated = false;

    // One-time onboarding migration: users with an explicit selected model have
    // already made it through model selection. Users who merely have compatible
    // files on disk should still see onboarding.
    if settings_value.get("onboarding_completed").is_none() {
        settings.onboarding_completed = !settings.selected_model.is_empty();
        updated = true;
    }

    // One-time What's New migration: migrations only run on an existing store
    // (fresh installs stamp the current version via get_default_settings). A
    // missing key here means a user upgrading from before it existed — blank it
    // so they see the current release's What's New, mirroring the onboarding
    // migration's explicit first-run-vs-upgrade decision.
    if settings_value.get("whats_new_last_seen_version").is_none() {
        settings.whats_new_last_seen_version = String::new();
        updated = true;
    }

    let stored_schema_version = settings_value
        .get("settings_schema_version")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    if stored_schema_version < 1 {
        // Before schema 1 this was a UI ordinal. Preserve the original safety
        // migration: a positive selection was ambiguous even in 0.1.
        let had_positive_legacy_selection = settings_value
            .get("transcribe_gpu_device")
            .and_then(|value| value.as_i64())
            .is_some_and(|value| value > 0);
        if had_positive_legacy_selection {
            settings.transcribe_accelerator = TranscribeAcceleratorSetting::Auto;
        }
    }
    if stored_schema_version < 2 {
        // transcribe.cpp 0.2 replaced integer registry indices with opaque
        // process-local handles. Clear every old index once.
        settings.transcribe_gpu_device = default_transcribe_gpu_device();
        settings.settings_schema_version = CURRENT_SETTINGS_SCHEMA_VERSION;
        updated = true;
    }

    // The generic GPU choice was removed in favor of Auto or an exact device.
    // Normalize settings created by builds that exposed that short-lived option.
    if settings.transcribe_accelerator == TranscribeAcceleratorSetting::Gpu
        && settings.transcribe_gpu_device.is_none()
    {
        settings.transcribe_accelerator = TranscribeAcceleratorSetting::Auto;
        updated = true;
    }

    // One-time overlay migration (only while the new key is absent): the retired
    // overlay_position `none` meant "hide the overlay" → OverlayStyle::None; any
    // other position had it visible → Live. The position enum no longer has a
    // `none` variant (legacy "none" deserializes to Bottom via a serde alias), so
    // read the raw stored string to recover the old intent.
    if settings_value.get("overlay_style").is_none() {
        let was_hidden = settings_value
            .get("overlay_position")
            .and_then(|v| v.as_str())
            == Some("none");
        settings.overlay_style = if was_hidden {
            OverlayStyle::None
        } else {
            OverlayStyle::Live
        };
        updated = true;
    }

    if !settings.bindings.contains_key("transform_selection") {
        settings.bindings.insert(
            "transform_selection".to_string(),
            ShortcutBinding {
                id: "transform_selection".to_string(),
                name: "Transform Selected Text".to_string(),
                description: "Opens prompt palette to transform selected text with AI.".to_string(),
                default_binding: "ctrl+alt+p".to_string(),
                current_binding: "ctrl+alt+p".to_string(),
            },
        );
        updated = true;
    }

    let suite_bindings = [
        (
            "search_overlay",
            "Chat & Research",
            "Opens AI chat & web research overlay (last mode).",
            "ctrl+alt+s",
        ),
        (
            "agent_chat",
            "AI Agent Chat",
            "Opens AI agent chat overlay directly.",
            "ctrl+alt+c",
        ),
        (
            "quick_note",
            "Quick Note & Idea Capture",
            "Opens quick note scratchpad overlay.",
            "ctrl+alt+n",
        ),
        (
            "todo_palette",
            "Todo & Tasks",
            "Opens task checklist & voice todo palette.",
            "ctrl+alt+t",
        ),
        (
            "doc_parser",
            "Document Parser & OCR",
            "Opens document parser & vision OCR dialog.",
            "ctrl+alt+d",
        ),
    ];

    for (id, name, desc, def) in suite_bindings {
        if !settings.bindings.contains_key(id) {
            settings.bindings.insert(
                id.to_string(),
                ShortcutBinding {
                    id: id.to_string(),
                    name: name.to_string(),
                    description: desc.to_string(),
                    default_binding: def.to_string(),
                    current_binding: def.to_string(),
                },
            );
            updated = true;
        }
    }

    // Merge any missing default transcription providers
    for default_p in default_transcription_providers() {
        if !settings
            .transcription_providers
            .iter()
            .any(|p| p.id == default_p.id)
        {
            settings.transcription_providers.push(default_p);
            updated = true;
        }
    }

    // Merge any missing default post process providers
    for default_p in default_post_process_providers() {
        if !settings
            .post_process_providers
            .iter()
            .any(|p| p.id == default_p.id)
        {
            settings.post_process_providers.push(default_p);
            updated = true;
        }
    }

    // Initialize web providers if empty
    if settings.web_providers.is_empty() {
        settings.web_providers = default_web_providers();
        updated = true;
    }

    // Seed chat agents for stores predating the agents feature.
    if settings.agents.is_empty() {
        settings.agents = default_agents();
        updated = true;
    }

    // Upgrade Firecrawl base URL from /v1 to /v2 if present
    for wp in &mut settings.web_providers {
        if wp.id == "firecrawl"
            && (wp.base_url == "https://api.firecrawl.dev/v1"
                || wp.base_url == "https://api.firecrawl.dev")
        {
            wp.base_url = "https://api.firecrawl.dev/v2".to_string();
            updated = true;
        }
    }

    // Auto-heal any corrupted or legacy shortcut bindings (e.g. ones with "press <...>")
    for binding in settings.bindings.values_mut() {
        let cur = binding.current_binding.trim();
        if cur.to_lowercase().starts_with("press ") || cur.contains('<') {
            if let Some(cleaned) = crate::shortcut::portal::portal_trigger_to_otush_binding(cur) {
                if cleaned != binding.current_binding {
                    binding.current_binding = cleaned;
                    updated = true;
                }
            }
        }
    }

    updated
}
