//! Keyboard shortcut management module
//!
//! Native GNOME/Wayland keyboard shortcuts via XDG Desktop Portal GlobalShortcuts
//! (see [`portal`]).

pub mod evdev;
mod handler;
pub mod portal;
pub mod ptt;

use crate::context::AppContext;
use crate::settings::ShortcutBinding;

pub mod settings_accelerator;
pub mod settings_audio_dsp;
pub mod settings_audio_ui;
pub mod settings_bindings;
pub mod settings_general;
pub mod settings_keyboard;
pub mod settings_post_process;
pub mod settings_transcription;
pub mod settings_tray;
pub mod settings_web;

/// Initialize shortcuts using XDG Desktop Portal and Direct Evdev Keyboard Listener
pub fn init_shortcuts(ctx: &AppContext) {
    portal::init_shortcuts(ctx);
    evdev::init_shortcuts(ctx);
}

/// Register the cancel shortcut (called when recording starts)
pub fn register_cancel_shortcut(ctx: &AppContext) {
    portal::register_cancel_shortcut(ctx);
}

/// Unregister the cancel shortcut (called when recording stops)
pub fn unregister_cancel_shortcut(ctx: &AppContext) {
    portal::unregister_cancel_shortcut(ctx);
}

/// Register a shortcut
pub fn register_shortcut(ctx: &AppContext, binding: ShortcutBinding) -> Result<(), String> {
    portal::register_shortcut(ctx, binding)
}

/// Unregister a shortcut
pub fn unregister_shortcut(ctx: &AppContext, binding: ShortcutBinding) -> Result<(), String> {
    portal::unregister_shortcut(ctx, binding)
}

/// Open GNOME System Settings to configure shortcuts directly
pub fn open_gnome_settings(ctx: &AppContext) {
    portal::open_gnome_settings(ctx);
}

// ============================================================================
// Binding Management Commands
// ============================================================================

pub use settings_bindings::{
    change_binding, reset_binding, resume_all_shortcuts, suspend_all_shortcuts,
};

pub use settings_general::{
    change_auto_submit_key_setting, change_auto_submit_setting, change_autostart_setting,
    change_clipboard_handling_setting, change_debug_mode_setting,
    change_experimental_enabled_setting, change_external_script_path_setting,
    change_overlay_position_setting, change_overlay_style_setting, change_paste_method_setting,
    change_post_process_enabled_setting, change_ptt_setting, change_selected_language_setting,
    change_theme_setting, change_typing_tool_setting, change_update_checks_setting,
};

pub use settings_post_process::{
    add_post_process_prompt, change_post_process_api_key_setting,
    change_post_process_base_url_setting, change_post_process_model_setting,
    change_post_process_timeout_setting, move_post_process_provider_priority,
    remove_post_process_prompt, set_post_process_prompt_preferred_provider,
    set_post_process_provider_reasoning, set_post_process_selected_prompt,
    test_post_process_provider_connection, toggle_post_process_provider_enabled,
    update_post_process_prompt_content, update_post_process_prompt_name,
};

pub use settings_audio_ui::{
    change_append_trailing_space_setting, change_filler_word_removal_enabled_setting,
    change_vad_backend_setting, change_vad_enabled_setting,
};

pub use settings_tray::{change_show_tray_icon_setting, change_tray_theme_setting};

pub use settings_accelerator::{
    change_ort_accelerator_setting, change_transcribe_accelerator_setting,
    toggle_local_transcription_setting,
};

pub use settings_transcription::{
    change_transcription_api_key_setting, change_transcription_base_url_setting,
    change_transcription_model_setting, change_transcription_timeout_setting,
    move_transcription_provider_priority, test_transcription_provider_connection,
    toggle_transcription_provider_enabled, update_deepgram_config,
};

pub use settings_web::{
    change_web_provider_api_key_setting, change_web_provider_base_url_setting,
    change_web_provider_timeout_setting, fetch_llm_provider_models, test_web_provider_connection,
    toggle_web_provider_enabled,
};

pub use settings_audio_dsp::{
    change_audio_high_pass_filter_setting, change_audio_input_gain_setting,
    change_audio_noise_gate_threshold_setting, change_audio_noise_reduction_setting,
    change_audio_normalization_setting,
};
