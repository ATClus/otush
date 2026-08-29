//! Shared shortcut event handling logic
//!
//! This module contains the common logic for handling shortcut events,
//! used by both the portal and evdev-keys implementations.

use log::warn;

use crate::actions::ACTION_MAP;
use crate::context::AppContext;
use crate::settings::get_settings;
use crate::transcription_coordinator::is_transcribe_binding;

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
    let settings = get_settings(ctx);

    // Transcribe bindings are handled by the coordinator.
    if is_transcribe_binding(binding_id) {
        ctx.coordinator
            .send_input(binding_id, hotkey_string, is_pressed, settings.push_to_talk);
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
