//! Shared shortcut event handling logic
//!
//! This module contains the common logic for handling shortcut events,
//! used by both the portal and evdev-keys implementations.

use log::{debug, warn};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::actions::ACTION_MAP;
use crate::context::AppContext;
use crate::transcription_coordinator::is_transcribe_binding;

static SHORTCUTS_SUSPENDED: AtomicBool = AtomicBool::new(false);

pub fn set_shortcuts_suspended(suspended: bool) {
    SHORTCUTS_SUSPENDED.store(suspended, Ordering::Relaxed);
}

pub fn are_shortcuts_suspended() -> bool {
    SHORTCUTS_SUSPENDED.load(Ordering::Relaxed)
}

/// Handle a shortcut event from either implementation.
///
/// This function contains the shared logic for:
/// - Looking up the action in ACTION_MAP
/// - Handling the cancel binding (only fires when recording)
/// - Handling push-to-talk mode (start on press, stop on release)
/// - Handling toggle mode (toggle state on press only)
pub fn handle_shortcut_event(
    ctx: &AppContext,
    binding_id: &str,
    hotkey_string: &str,
    is_pressed: bool,
) {
    if are_shortcuts_suspended() {
        debug!(
            "Shortcut event '{}' ignored because shortcuts are suspended",
            binding_id
        );
        return;
    }

    let settings = crate::settings::get_settings(ctx);

    // Transcribe bindings are handled by the coordinator.
    if is_transcribe_binding(binding_id) {
        let is_meeting = binding_id == "transcribe_meeting";
        if settings.push_to_talk && !is_meeting {
            let hotkey = if hotkey_string.is_empty() {
                settings
                    .bindings
                    .get(binding_id)
                    .map(|b| b.current_binding.as_str())
                    .unwrap_or_default()
            } else {
                hotkey_string
            };

            // In Push-to-Talk mode, start recording on the first press and spawn the release watcher.
            // Subsequent auto-repeat press events while recording are ignored until physical key release.
            if !ctx.audio.is_recording() {
                ctx.coordinator.send_input(binding_id, hotkey, true, true);
                crate::shortcut::ptt::start_ptt_release_watcher(ctx, binding_id, hotkey);
            }
        } else if is_pressed {
            ctx.coordinator
                .send_input(binding_id, hotkey_string, true, false);
        }
        return;
    }

    if let Some(prompt_id) = binding_id
        .strip_prefix("custom_prompt_")
        .or_else(|| binding_id.strip_prefix("prompt_"))
    {
        if is_pressed {
            crate::ui::prompt_palette::execute_prompt_by_id(ctx, prompt_id);
        }
        return;
    }

    let Some(action) = ACTION_MAP.get(binding_id) else {
        warn!(
            "No action defined in ACTION_MAP for shortcut ID '{}'. Shortcut: '{}', Pressed: {}",
            binding_id, hotkey_string, is_pressed
        );
        return;
    };

    // Cancel binding: only fires when recording and key is pressed
    if binding_id == "cancel" {
        if ctx.audio.is_recording() && is_pressed {
            action.start(ctx, binding_id, hotkey_string);
        }
        return;
    }

    // Remaining bindings (e.g. "test") use simple start/stop on press/release.
    if is_pressed {
        action.start(ctx, binding_id, hotkey_string);
    } else {
        action.stop(ctx, binding_id, hotkey_string);
    }
}
