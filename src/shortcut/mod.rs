#![allow(dead_code)]
//! Keyboard shortcut management module
//!
//! Native GNOME/Wayland keyboard shortcuts via XDG Desktop Portal GlobalShortcuts
//! (see [`portal`]).

pub mod evdev;
mod handler;
pub mod portal;
pub mod ptt;

use crate::context::{AppContext, AppEvent};
use crate::settings::{
    self, get_settings, AutoSubmitKey, ClipboardHandling, KeyboardImplementation, LLMPrompt,
    OverlayPosition, OverlayStyle, PasteMethod, ShortcutBinding, SoundTheme, Theme, TypingTool,
    VadBackend,
};
use crate::tray;
use log::{error, warn};
use serde::Serialize;

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

#[derive(Serialize)]
pub struct BindingResponse {
    success: bool,
    binding: Option<ShortcutBinding>,
    error: Option<String>,
}

pub fn change_binding(
    ctx: &AppContext,
    id: String,
    binding: String,
) -> Result<BindingResponse, String> {
    // Reject empty bindings — every shortcut should have a value
    if binding.trim().is_empty() {
        return Err("Binding cannot be empty".to_string());
    }

    let mut settings = settings::get_settings(ctx);

    // Get the binding to modify, or create it from defaults if it doesn't exist
    let binding_to_modify = match settings.bindings.get(&id) {
        Some(binding) => binding.clone(),
        None => {
            // Try to get the default binding for this id
            let default_settings = settings::get_default_settings();
            match default_settings.bindings.get(&id) {
                Some(default_binding) => {
                    warn!(
                        "Binding '{}' not found in settings, creating from defaults",
                        id
                    );
                    default_binding.clone()
                }
                None => {
                    let error_msg = format!("Binding with id '{}' not found in defaults", id);
                    warn!("change_binding error: {}", error_msg);
                    return Ok(BindingResponse {
                        success: false,
                        binding: None,
                        error: Some(error_msg),
                    });
                }
            }
        }
    };

    // If this is the cancel binding, just update the settings and return
    // It's managed dynamically, so we don't register/unregister here
    if id == "cancel" {
        if let Some(mut b) = settings.bindings.get(&id).cloned() {
            b.current_binding = binding;
            settings.bindings.insert(id.clone(), b.clone());
            ctx.notify_setting_changed("bindings", serde_json::json!(&settings.bindings));
            settings::write_settings(ctx, settings);
            return Ok(BindingResponse {
                success: true,
                binding: Some(b),
                error: None,
            });
        }
    }

    // Unregister the existing binding
    if let Err(e) = unregister_shortcut(ctx, binding_to_modify.clone()) {
        let error_msg = format!("Failed to unregister shortcut: {}", e);
        error!("change_binding error: {}", error_msg);
    }

    // Validate the new shortcut for the current keyboard implementation
    if let Err(e) = validate_shortcut_for_implementation(&binding, settings.keyboard_implementation)
    {
        warn!("change_binding validation error: {}", e);
        restore_registration(ctx, &binding_to_modify);
        return Err(e);
    }

    // Create an updated binding
    let mut updated_binding = binding_to_modify.clone();
    updated_binding.current_binding = binding.clone();

    // Register the new binding
    if let Err(e) = register_shortcut(ctx, updated_binding.clone()) {
        let error_msg = format!("Failed to register shortcut: {}", e);
        error!("change_binding error: {}", error_msg);
        restore_registration(ctx, &binding_to_modify);
        return Ok(BindingResponse {
            success: false,
            binding: None,
            error: Some(error_msg),
        });
    }

    // Unregister and clear any conflicting binding that previously held this exact key combination
    let mut conflicting_ids = Vec::new();
    for (other_id, other_binding) in &settings.bindings {
        if other_id != &id
            && !other_binding.current_binding.trim().is_empty()
            && other_binding
                .current_binding
                .trim()
                .eq_ignore_ascii_case(binding.trim())
        {
            conflicting_ids.push(other_id.clone());
        }
    }
    for conflict_id in conflicting_ids {
        if let Some(mut conflict_binding) = settings.bindings.get(&conflict_id).cloned() {
            let _ = unregister_shortcut(ctx, conflict_binding.clone());
            conflict_binding.current_binding = String::new();
            settings
                .bindings
                .insert(conflict_id.clone(), conflict_binding.clone());
            ctx.notify_setting_changed("bindings", serde_json::json!(&conflict_binding));
        }
    }

    // Update the binding in the settings
    settings.bindings.insert(id, updated_binding.clone());

    // Save the settings
    settings::write_settings(ctx, settings.clone());
    portal::sync_bindings_to_gnome_gsettings(ctx);
    portal::sync_desired_bindings(&settings);
    ctx.notify_setting_changed("bindings", serde_json::json!(&updated_binding));

    // Return the updated binding
    Ok(BindingResponse {
        success: true,
        binding: Some(updated_binding),
        error: None,
    })
}

/// Best-effort re-register of the previous binding after a failed change,
/// so a failure leaves the user's shortcut working exactly as before.
fn restore_registration(ctx: &AppContext, binding: &ShortcutBinding) {
    if let Err(e) = register_shortcut(ctx, binding.clone()) {
        error!(
            "Failed to restore previous binding '{}' ({}): {}",
            binding.id, binding.current_binding, e
        );
    }
}

pub fn reset_binding(ctx: &AppContext, id: String) -> Result<BindingResponse, String> {
    let binding = settings::get_stored_binding(ctx, &id);
    change_binding(ctx, id, binding.default_binding)
}

pub fn add_custom_binding(
    ctx: &AppContext,
    id: String,
    name: String,
    description: String,
    binding: String,
) -> Result<BindingResponse, String> {
    if binding.trim().is_empty() {
        return Err("Binding cannot be empty".to_string());
    }

    let mut settings = settings::get_settings(ctx);

    validate_shortcut_for_implementation(&binding, settings.keyboard_implementation)?;

    let shortcut_binding = ShortcutBinding {
        id: id.clone(),
        name,
        description,
        default_binding: binding.clone(),
        current_binding: binding,
    };

    if let Err(e) = register_shortcut(ctx, shortcut_binding.clone()) {
        return Ok(BindingResponse {
            success: false,
            binding: None,
            error: Some(format!("Failed to register shortcut: {e}")),
        });
    }

    settings.bindings.insert(id, shortcut_binding.clone());
    settings::write_settings(ctx, settings.clone());
    portal::sync_bindings_to_gnome_gsettings(ctx);
    portal::sync_desired_bindings(&settings);
    ctx.notify_setting_changed("bindings", serde_json::json!(&shortcut_binding));

    Ok(BindingResponse {
        success: true,
        binding: Some(shortcut_binding),
        error: None,
    })
}

pub fn remove_custom_binding(ctx: &AppContext, id: &str) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    if let Some(binding) = settings.bindings.remove(id) {
        let _ = unregister_shortcut(ctx, binding);
        settings::write_settings(ctx, settings.clone());
        portal::sync_bindings_to_gnome_gsettings(ctx);
        portal::sync_desired_bindings(&settings);
        ctx.notify_setting_changed("bindings", serde_json::json!(&settings.bindings));
    }
    Ok(())
}

/// Temporarily suspend global shortcut execution while the user is recording a new shortcut in
/// the UI, so no existing shortcut can fire mid-capture.
pub fn suspend_all_shortcuts(_ctx: &AppContext) {
    handler::set_shortcuts_suspended(true);
}

/// Re-enable shortcut execution after shortcut recording ends and synchronize bindings.
pub fn resume_all_shortcuts(ctx: &AppContext) {
    handler::set_shortcuts_suspended(false);
    let settings = get_settings(ctx);
    portal::sync_bindings_to_gnome_gsettings(ctx);
    portal::sync_desired_bindings(&settings);
}

/// Temporarily unregister all bindings while the user is recording a
/// shortcut in the UI. This avoids firing actions while keys are recorded.
pub fn suspend_all_bindings(ctx: &AppContext) -> Result<(), String> {
    suspend_all_shortcuts(ctx);
    Ok(())
}

/// Re-register all bindings after the user has finished recording.
pub fn resume_all_bindings(ctx: &AppContext) -> Result<(), String> {
    resume_all_shortcuts(ctx);
    Ok(())
}

// ============================================================================
// Keyboard Implementation (Portal)
// ============================================================================

/// Result of changing keyboard implementation
#[derive(Serialize)]
pub struct ImplementationChangeResult {
    pub success: bool,
    pub reset_bindings: Vec<String>,
}

pub fn change_keyboard_implementation_setting(
    _ctx: &AppContext,
    _implementation: String,
) -> Result<ImplementationChangeResult, String> {
    Ok(ImplementationChangeResult {
        success: true,
        reset_bindings: vec![],
    })
}

pub fn get_keyboard_implementation(_ctx: &AppContext) -> String {
    "portal".to_string()
}

fn validate_shortcut_for_implementation(
    raw: &str,
    _implementation: KeyboardImplementation,
) -> Result<(), String> {
    portal::validate_shortcut(raw)
}

fn parse_keyboard_implementation(_s: &str) -> KeyboardImplementation {
    KeyboardImplementation::Portal
}

fn unregister_all_shortcuts(ctx: &AppContext, _implementation: KeyboardImplementation) {
    let bindings = settings::get_bindings(ctx);
    for (id, binding) in bindings {
        if id == "cancel" {
            continue;
        }
        let _ = portal::unregister_shortcut(ctx, binding);
    }
}

fn register_all_shortcuts_for_implementation(
    ctx: &AppContext,
    _implementation: KeyboardImplementation,
) -> Vec<String> {
    let default_bindings = settings::get_default_settings().bindings;
    let current_settings = settings::get_settings(ctx);

    for (id, default_binding) in &default_bindings {
        if id == "cancel" {
            continue;
        }
        if id == "transcribe_with_post_process" && !current_settings.post_process_enabled {
            continue;
        }
        let binding = current_settings
            .bindings
            .get(id)
            .cloned()
            .unwrap_or_else(|| default_binding.clone());
        let _ = portal::register_shortcut(ctx, binding);
    }
    Vec::new()
}

// ============================================================================
// General Settings Commands
// ============================================================================

pub fn change_ptt_setting(ctx: &AppContext, enabled: bool) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    settings.push_to_talk = enabled;
    settings::write_settings(ctx, settings);
    ctx.notify_setting_changed("push_to_talk", serde_json::json!(enabled));
    Ok(())
}

pub fn change_audio_feedback_setting(ctx: &AppContext, enabled: bool) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    settings.audio_feedback = enabled;
    settings::write_settings(ctx, settings);
    ctx.notify_setting_changed("audio_feedback", serde_json::json!(enabled));
    Ok(())
}

pub fn change_audio_feedback_volume_setting(ctx: &AppContext, volume: f32) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    settings.audio_feedback_volume = volume;
    settings::write_settings(ctx, settings);
    ctx.notify_setting_changed("audio_feedback_volume", serde_json::json!(volume));
    Ok(())
}

pub fn change_sound_theme_setting(ctx: &AppContext, theme: String) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    let parsed = match theme.as_str() {
        "marimba" => SoundTheme::Marimba,
        "pop" => SoundTheme::Pop,
        "custom" => SoundTheme::Custom,
        other => {
            warn!("Invalid sound theme '{}', defaulting to marimba", other);
            SoundTheme::Marimba
        }
    };
    settings.sound_theme = parsed;
    settings::write_settings(ctx, settings);
    ctx.notify_setting_changed("sound_theme", serde_json::json!(theme));
    Ok(())
}

pub fn change_theme_setting(ctx: &AppContext, theme: String) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    let parsed = match theme.as_str() {
        "system" => Theme::System,
        "light" => Theme::Light,
        "dark" => Theme::Dark,
        other => {
            warn!("Invalid theme '{}', defaulting to system", other);
            Theme::System
        }
    };
    settings.theme = parsed;
    settings::write_settings(ctx, settings);
    // Notify the UI (and the overlay) so they re-apply the palette live.
    ctx.bus.send(AppEvent::ThemeChanged(parsed));
    Ok(())
}

pub fn change_translate_to_english_setting(ctx: &AppContext, enabled: bool) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    settings.translate_to_english = enabled;
    settings::write_settings(ctx, settings);
    ctx.notify_setting_changed("translate_to_english", serde_json::json!(enabled));
    Ok(())
}

pub fn change_selected_language_setting(ctx: &AppContext, language: String) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    settings.selected_language = language.clone();
    settings::write_settings(ctx, settings);
    ctx.notify_setting_changed("selected_language", serde_json::json!(language));
    Ok(())
}

pub fn change_overlay_position_setting(ctx: &AppContext, position: String) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    let parsed = match position.as_str() {
        // "none" is retired (visibility is overlay_style now); fold legacy callers
        // onto Bottom rather than warn.
        "none" | "bottom" => OverlayPosition::Bottom,
        "top" => OverlayPosition::Top,
        other => {
            warn!("Invalid overlay position '{}', defaulting to bottom", other);
            OverlayPosition::Bottom
        }
    };
    settings.overlay_position = parsed;
    settings::write_settings(ctx, settings);

    // Whether the overlay shows at all is owned by overlay_style now; position
    // only ever toggles Top/Bottom, so the enabled cache is untouched here.
    crate::utils::update_overlay_position(ctx);

    ctx.notify_setting_changed("overlay_position", serde_json::json!(position));
    Ok(())
}

pub fn change_overlay_style_setting(ctx: &AppContext, style: String) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    let parsed = match style.as_str() {
        "none" => OverlayStyle::None,
        "minimal" => OverlayStyle::Minimal,
        "live" => OverlayStyle::Live,
        other => {
            warn!("Invalid overlay style '{}', defaulting to minimal", other);
            OverlayStyle::Minimal
        }
    };
    settings.overlay_style = parsed;
    settings::write_settings(ctx, settings);

    // Keep the cached overlay-enabled flag in sync so emit_levels stops (or
    // resumes) emitting on the next audio callback.
    crate::overlay::update_overlay_enabled_cache(parsed != OverlayStyle::None);

    // Reposition in case the window needs to re-center for the new style.
    crate::utils::update_overlay_position(ctx);

    ctx.notify_setting_changed("overlay_style", serde_json::json!(style));
    Ok(())
}

pub fn change_debug_mode_setting(ctx: &AppContext, enabled: bool) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    settings.debug_mode = enabled;
    settings::write_settings(ctx, settings);

    // Keep UI log streaming in sync: the live log viewer only exists in
    // debug mode, so logs are forwarded to the UI only while it is on.
    crate::logging::WEBVIEW_LOG_STREAMING.store(enabled, std::sync::atomic::Ordering::Relaxed);

    ctx.notify_setting_changed("debug_mode", serde_json::json!(enabled));
    Ok(())
}

pub fn change_start_hidden_setting(ctx: &AppContext, enabled: bool) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    settings.start_hidden = enabled;
    settings::write_settings(ctx, settings);
    ctx.notify_setting_changed("start_hidden", serde_json::json!(enabled));
    Ok(())
}

pub fn change_autostart_setting(ctx: &AppContext, enabled: bool) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    settings.autostart_enabled = enabled;
    settings::write_settings(ctx, settings);

    // Apply the autostart setting immediately
    crate::autostart::apply_autostart(ctx, enabled);

    ctx.notify_setting_changed("autostart_enabled", serde_json::json!(enabled));
    Ok(())
}

pub fn change_update_checks_setting(ctx: &AppContext, enabled: bool) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    settings.update_checks_enabled = enabled;
    settings::write_settings(ctx, settings);
    ctx.notify_setting_changed("update_checks_enabled", serde_json::json!(enabled));
    Ok(())
}

pub fn change_show_whats_new_on_update_setting(
    ctx: &AppContext,
    enabled: bool,
) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    settings.show_whats_new_on_update = enabled;
    settings::write_settings(ctx, settings);
    ctx.notify_setting_changed("show_whats_new_on_update", serde_json::json!(enabled));
    Ok(())
}

pub fn change_whats_new_last_seen_version_setting(
    ctx: &AppContext,
    version: String,
) -> Result<(), String> {
    let version = version.trim().to_string();
    let mut settings = settings::get_settings(ctx);
    settings.whats_new_last_seen_version = version.clone();
    settings::write_settings(ctx, settings);
    ctx.notify_setting_changed("whats_new_last_seen_version", serde_json::json!(version));
    Ok(())
}

pub fn update_custom_words(ctx: &AppContext, words: Vec<String>) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    settings.custom_words = words.clone();
    settings::write_settings(ctx, settings);
    ctx.notify_setting_changed("custom_words", serde_json::json!(words));
    Ok(())
}

pub fn change_word_correction_threshold_setting(
    ctx: &AppContext,
    threshold: f64,
) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    settings.word_correction_threshold = threshold;
    settings::write_settings(ctx, settings);
    ctx.notify_setting_changed("word_correction_threshold", serde_json::json!(threshold));
    Ok(())
}

pub fn change_extra_recording_buffer_setting(ctx: &AppContext, ms: u64) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    settings.extra_recording_buffer_ms = ms;
    settings::write_settings(ctx, settings);
    ctx.notify_setting_changed("extra_recording_buffer_ms", serde_json::json!(ms));
    Ok(())
}

pub fn change_paste_delay_ms_setting(ctx: &AppContext, ms: u64) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    settings.paste_delay_ms = ms;
    settings::write_settings(ctx, settings);
    ctx.notify_setting_changed("paste_delay_ms", serde_json::json!(ms));
    Ok(())
}

pub fn change_paste_delay_after_ms_setting(ctx: &AppContext, ms: u64) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    settings.paste_delay_after_ms = ms;
    settings::write_settings(ctx, settings);
    ctx.notify_setting_changed("paste_delay_after_ms", serde_json::json!(ms));
    Ok(())
}

pub fn change_reliable_paste_setting(ctx: &AppContext, enabled: bool) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    settings.reliable_paste = enabled;
    settings::write_settings(ctx, settings);
    ctx.notify_setting_changed("reliable_paste", serde_json::json!(enabled));
    Ok(())
}

pub fn change_paste_method_setting(ctx: &AppContext, method: String) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    let parsed = match method.as_str() {
        "ctrl_v" => PasteMethod::CtrlV,
        "direct" => PasteMethod::Direct,
        "none" => PasteMethod::None,
        "shift_insert" => PasteMethod::ShiftInsert,
        "ctrl_shift_v" => PasteMethod::CtrlShiftV,
        "external_script" => PasteMethod::ExternalScript,
        other => {
            warn!("Invalid paste method '{}', defaulting to ctrl_v", other);
            PasteMethod::CtrlV
        }
    };
    settings.paste_method = parsed;
    settings::write_settings(ctx, settings);
    ctx.notify_setting_changed("paste_method", serde_json::json!(method));
    Ok(())
}

pub fn get_available_typing_tools() -> Vec<String> {
    crate::clipboard::get_available_typing_tools()
}

pub fn change_typing_tool_setting(ctx: &AppContext, tool: String) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    let parsed = match tool.as_str() {
        "auto" => TypingTool::Auto,
        "wtype" => TypingTool::Wtype,
        "kwtype" => TypingTool::Kwtype,
        "dotool" => TypingTool::Dotool,
        "ydotool" => TypingTool::Ydotool,
        "xdotool" => TypingTool::Xdotool,
        other => {
            warn!("Invalid typing tool '{}', defaulting to auto", other);
            TypingTool::Auto
        }
    };
    settings.typing_tool = parsed;
    settings::write_settings(ctx, settings);
    ctx.notify_setting_changed("typing_tool", serde_json::json!(tool));
    Ok(())
}

pub fn change_external_script_path_setting(
    ctx: &AppContext,
    path: Option<String>,
) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    settings.external_script_path = path.clone();
    settings::write_settings(ctx, settings);
    ctx.notify_setting_changed("external_script_path", serde_json::json!(path));
    Ok(())
}

pub fn change_clipboard_handling_setting(ctx: &AppContext, handling: String) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    let parsed = match handling.as_str() {
        "dont_modify" => ClipboardHandling::DontModify,
        "copy_to_clipboard" => ClipboardHandling::CopyToClipboard,
        other => {
            warn!(
                "Invalid clipboard handling '{}', defaulting to dont_modify",
                other
            );
            ClipboardHandling::DontModify
        }
    };
    settings.clipboard_handling = parsed;
    settings::write_settings(ctx, settings);
    ctx.notify_setting_changed("clipboard_handling", serde_json::json!(handling));
    Ok(())
}

pub fn change_auto_submit_setting(ctx: &AppContext, enabled: bool) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    settings.auto_submit = enabled;
    settings::write_settings(ctx, settings);
    ctx.notify_setting_changed("auto_submit", serde_json::json!(enabled));
    Ok(())
}

pub fn change_auto_submit_key_setting(ctx: &AppContext, key: String) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    let parsed = match key.as_str() {
        "enter" => AutoSubmitKey::Enter,
        "ctrl_enter" => AutoSubmitKey::CtrlEnter,
        "cmd_enter" => AutoSubmitKey::CmdEnter,
        other => {
            warn!("Invalid auto submit key '{}', defaulting to enter", other);
            AutoSubmitKey::Enter
        }
    };
    settings.auto_submit_key = parsed;
    settings::write_settings(ctx, settings);
    ctx.notify_setting_changed("auto_submit_key", serde_json::json!(key));
    Ok(())
}

pub fn change_post_process_enabled_setting(ctx: &AppContext, enabled: bool) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    settings.post_process_enabled = enabled;

    // Register or unregister the post-processing shortcut
    if let Some(binding) = settings
        .bindings
        .get("transcribe_with_post_process")
        .cloned()
    {
        if enabled {
            let _ = register_shortcut(ctx, binding);
        } else {
            let _ = unregister_shortcut(ctx, binding);
        }
    }

    settings::write_settings(ctx, settings);
    ctx.notify_setting_changed("post_process_enabled", serde_json::json!(enabled));
    Ok(())
}

pub fn change_experimental_enabled_setting(ctx: &AppContext, enabled: bool) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    settings.experimental_enabled = enabled;
    settings::write_settings(ctx, settings);
    ctx.notify_setting_changed("experimental_enabled", serde_json::json!(enabled));
    Ok(())
}

pub fn change_post_process_base_url_setting(
    ctx: &AppContext,
    provider_id: String,
    base_url: String,
) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    let label = settings
        .post_process_provider(&provider_id)
        .map(|provider| provider.label.clone())
        .ok_or_else(|| format!("Provider '{}' not found", provider_id))?;

    let provider = settings
        .post_process_provider_mut(&provider_id)
        .expect("Provider looked up above must exist");

    if provider.id != "custom" {
        return Err(format!(
            "Provider '{}' does not allow editing the base URL",
            label
        ));
    }

    provider.base_url = base_url;
    settings::write_settings(ctx, settings);
    ctx.notify_setting_changed("post_process_base_url", serde_json::json!(provider_id));
    Ok(())
}

/// Generic helper to validate provider exists
fn validate_provider_exists(
    settings: &settings::AppSettings,
    provider_id: &str,
) -> Result<(), String> {
    if !settings
        .post_process_providers
        .iter()
        .any(|provider| provider.id == provider_id)
    {
        return Err(format!("Provider '{}' not found", provider_id));
    }
    Ok(())
}

pub fn change_post_process_api_key_setting(
    ctx: &AppContext,
    provider_id: String,
    api_key: String,
) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    validate_provider_exists(&settings, &provider_id)?;
    settings
        .post_process_api_keys
        .insert(provider_id.clone(), api_key);
    settings::write_settings(ctx, settings);
    ctx.notify_setting_changed("post_process_api_key", serde_json::json!(provider_id));
    Ok(())
}

pub fn change_post_process_model_setting(
    ctx: &AppContext,
    provider_id: String,
    model: String,
) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    validate_provider_exists(&settings, &provider_id)?;
    settings
        .post_process_models
        .insert(provider_id.clone(), model);
    settings::write_settings(ctx, settings);
    ctx.notify_setting_changed("post_process_model", serde_json::json!(provider_id));
    Ok(())
}

pub fn change_post_process_timeout_setting(
    ctx: &AppContext,
    provider_id: String,
    timeout_seconds: u32,
) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    validate_provider_exists(&settings, &provider_id)?;
    if let Some(provider) = settings.post_process_provider_mut(&provider_id) {
        provider.timeout_seconds = timeout_seconds;
        settings::write_settings(ctx, settings);
        ctx.notify_setting_changed(
            "post_process_provider_timeout",
            serde_json::json!(provider_id),
        );
    }
    Ok(())
}

pub fn set_post_process_provider(ctx: &AppContext, provider_id: String) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    validate_provider_exists(&settings, &provider_id)?;
    settings.post_process_provider_id = provider_id.clone();
    settings::write_settings(ctx, settings);
    ctx.notify_setting_changed("post_process_provider_id", serde_json::json!(provider_id));
    Ok(())
}

pub fn add_post_process_prompt(
    ctx: &AppContext,
    name: String,
    prompt: String,
) -> Result<LLMPrompt, String> {
    let mut settings = settings::get_settings(ctx);

    // Generate unique ID using timestamp and random component
    let id = format!("prompt_{}", chrono::Utc::now().timestamp_millis());

    let new_prompt = LLMPrompt {
        id,
        name,
        prompt,
        preferred_provider_id: None,
    };

    settings.post_process_prompts.push(new_prompt.clone());
    ctx.notify_setting_changed(
        "post_process_prompts_structure",
        serde_json::json!(&settings.post_process_prompts),
    );
    settings::write_settings(ctx, settings);

    Ok(new_prompt)
}

pub fn update_post_process_prompt_name(
    ctx: &AppContext,
    id: String,
    name: String,
) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);

    if let Some(existing_prompt) = settings
        .post_process_prompts
        .iter_mut()
        .find(|p| p.id == id)
    {
        existing_prompt.name = name.clone();
        ctx.notify_setting_changed(
            "post_process_prompt_name",
            serde_json::json!({ "id": id, "name": name }),
        );
        settings::write_settings(ctx, settings);
        Ok(())
    } else {
        Err(format!("Prompt with id '{}' not found", id))
    }
}

pub fn update_post_process_prompt_content(
    ctx: &AppContext,
    id: String,
    prompt: String,
) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);

    if let Some(existing_prompt) = settings
        .post_process_prompts
        .iter_mut()
        .find(|p| p.id == id)
    {
        existing_prompt.prompt = prompt;
        ctx.notify_setting_changed("post_process_prompt_content", serde_json::json!(&id));
        settings::write_settings(ctx, settings);
        Ok(())
    } else {
        Err(format!("Prompt with id '{}' not found", id))
    }
}

pub fn update_post_process_prompt(
    ctx: &AppContext,
    id: String,
    name: String,
    prompt: String,
) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);

    if let Some(existing_prompt) = settings
        .post_process_prompts
        .iter_mut()
        .find(|p| p.id == id)
    {
        existing_prompt.name = name.clone();
        existing_prompt.prompt = prompt;
        ctx.notify_setting_changed(
            "post_process_prompt_name",
            serde_json::json!({ "id": id, "name": name }),
        );
        ctx.notify_setting_changed("post_process_prompt_content", serde_json::json!(&id));
        settings::write_settings(ctx, settings);
        Ok(())
    } else {
        Err(format!("Prompt with id '{}' not found", id))
    }
}

pub fn delete_post_process_prompt(ctx: &AppContext, id: String) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);

    // Don't allow deleting the last prompt
    if settings.post_process_prompts.len() <= 1 {
        return Err("Cannot delete the last prompt".to_string());
    }

    // Find and remove the prompt
    let original_len = settings.post_process_prompts.len();
    settings.post_process_prompts.retain(|p| p.id != id);

    if settings.post_process_prompts.len() == original_len {
        return Err(format!("Prompt with id '{}' not found", id));
    }

    // If the deleted prompt was selected, select the first one or None
    if settings.post_process_selected_prompt_id.as_ref() == Some(&id) {
        settings.post_process_selected_prompt_id =
            settings.post_process_prompts.first().map(|p| p.id.clone());
    }

    ctx.notify_setting_changed(
        "post_process_prompts_structure",
        serde_json::json!(&settings.post_process_prompts),
    );
    settings::write_settings(ctx, settings);
    Ok(())
}

pub fn remove_post_process_prompt(ctx: &AppContext, id: String) -> Result<(), String> {
    delete_post_process_prompt(ctx, id)
}

pub async fn fetch_post_process_models(
    ctx: &AppContext,
    provider_id: String,
) -> Result<Vec<String>, String> {
    let settings = settings::get_settings(ctx);

    // Find the provider
    let provider = settings
        .post_process_providers
        .iter()
        .find(|p| p.id == provider_id)
        .ok_or_else(|| format!("Provider '{}' not found", provider_id))?;

    // Get API key
    let api_key = settings
        .post_process_api_keys
        .get(&provider_id)
        .cloned()
        .unwrap_or_default();

    // Skip fetching if no API key for providers that typically need one
    if api_key.trim().is_empty() && provider.id != "custom" {
        return Err(format!(
            "API key is required for {}. Please add an API key to list available models.",
            provider.label
        ));
    }

    crate::llm_client::fetch_models(provider, api_key).await
}

pub fn set_post_process_selected_prompt(ctx: &AppContext, id: String) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);

    // Verify the prompt exists
    if !settings.post_process_prompts.iter().any(|p| p.id == id) {
        return Err(format!("Prompt with id '{}' not found", id));
    }

    settings.post_process_selected_prompt_id = Some(id);
    ctx.notify_setting_changed(
        "post_process_selected_prompt_id",
        serde_json::json!(&settings.post_process_selected_prompt_id),
    );
    settings::write_settings(ctx, settings);
    Ok(())
}

pub fn set_post_process_prompt_preferred_provider(
    ctx: &AppContext,
    prompt_id: String,
    preferred_provider_id: Option<String>,
) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);

    if let Some(prompt) = settings
        .post_process_prompts
        .iter_mut()
        .find(|p| p.id == prompt_id)
    {
        prompt.preferred_provider_id = preferred_provider_id;
        ctx.notify_setting_changed(
            "post_process_prompts",
            serde_json::json!(&settings.post_process_prompts),
        );
        settings::write_settings(ctx, settings);
        Ok(())
    } else {
        Err(format!("Prompt with id '{}' not found", prompt_id))
    }
}

pub fn move_post_process_provider_priority(
    ctx: &AppContext,
    provider_id: &str,
    up: bool,
) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    let len = settings.post_process_providers.len();
    if len <= 1 {
        return Ok(());
    }

    let Some(index) = settings
        .post_process_providers
        .iter()
        .position(|p| p.id == provider_id)
    else {
        return Err(format!("Provider '{}' not found", provider_id));
    };

    if up && index > 0 {
        settings.post_process_providers.swap(index, index - 1);
    } else if !up && index + 1 < len {
        settings.post_process_providers.swap(index, index + 1);
    } else {
        return Ok(());
    }

    settings::write_settings(ctx, settings);
    ctx.notify_setting_changed("post_process_providers", serde_json::json!("reordered"));
    ctx.notify_setting_changed(
        "post_process_providers_reordered",
        serde_json::json!(provider_id),
    );
    Ok(())
}

pub fn toggle_post_process_provider_enabled(
    ctx: &AppContext,
    provider_id: String,
    enabled: bool,
) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    if let Some(p) = settings
        .post_process_providers
        .iter_mut()
        .find(|p| p.id == provider_id)
    {
        p.enabled = enabled;
        settings::write_settings(ctx, settings);
        ctx.notify_setting_changed("post_process_providers", serde_json::json!(&provider_id));
        ctx.notify_setting_changed(
            "post_process_provider_enabled",
            serde_json::json!(&provider_id),
        );
        Ok(())
    } else {
        Err(format!("Provider '{}' not found", provider_id))
    }
}

pub fn set_post_process_provider_reasoning(
    ctx: &AppContext,
    provider_id: String,
    effort: crate::settings::ReasoningEffort,
    budget_tokens: Option<u32>,
) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    if let Some(p) = settings
        .post_process_providers
        .iter_mut()
        .find(|p| p.id == provider_id)
    {
        p.reasoning = crate::settings::ProviderReasoningConfig {
            effort,
            budget_tokens,
        };
        settings::write_settings(ctx, settings);
        ctx.notify_setting_changed("post_process_providers", serde_json::json!(provider_id));
        Ok(())
    } else {
        Err(format!("Provider '{}' not found", provider_id))
    }
}

pub async fn test_post_process_provider_connection(
    ctx: &AppContext,
    provider_id: String,
) -> Result<(String, u128), String> {
    let settings = settings::get_settings(ctx);
    let provider = settings
        .post_process_providers
        .iter()
        .find(|p| p.id == provider_id)
        .ok_or_else(|| format!("Provider '{}' not found", provider_id))?;

    let api_key = settings
        .post_process_api_keys
        .get(&provider_id)
        .cloned()
        .unwrap_or_default();

    let model = settings
        .post_process_models
        .get(&provider_id)
        .cloned()
        .unwrap_or_else(|| crate::settings::default_model_for_provider(&provider_id));

    if model.is_empty() {
        return Err("No model selected for this provider".to_string());
    }

    crate::llm_client::test_provider_connection(provider, api_key, &model).await
}

pub fn change_mute_while_recording_setting(ctx: &AppContext, enabled: bool) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    settings.mute_while_recording = enabled;
    settings::write_settings(ctx, settings);
    ctx.notify_setting_changed("mute_while_recording", serde_json::json!(enabled));
    Ok(())
}

pub fn change_append_trailing_space_setting(ctx: &AppContext, enabled: bool) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    settings.append_trailing_space = enabled;
    settings::write_settings(ctx, settings);
    ctx.notify_setting_changed("append_trailing_space", serde_json::json!(enabled));
    Ok(())
}

pub fn change_lazy_stream_close_setting(ctx: &AppContext, enabled: bool) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    settings.lazy_stream_close = enabled;
    settings::write_settings(ctx, settings);
    ctx.notify_setting_changed("lazy_stream_close", serde_json::json!(enabled));
    Ok(())
}

pub fn change_vad_enabled_setting(ctx: &AppContext, enabled: bool) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    settings.vad_enabled = enabled;
    settings::write_settings(ctx, settings);
    ctx.notify_setting_changed("vad_enabled", serde_json::json!(enabled));
    Ok(())
}

pub async fn change_vad_backend_setting(
    ctx: &AppContext,
    backend: VadBackend,
) -> Result<(), String> {
    if settings::get_settings(ctx).vad_backend == backend {
        return Ok(());
    }

    // Construct/swap the detector and, when necessary, reopen cpal away from
    // the main thread. Persist only after the runtime change succeeds so a
    // rejected in-progress switch or failed microphone reopen rolls back cleanly.
    let manager = ctx.audio.clone();
    crate::runtime::spawn_blocking(move || manager.update_vad_backend(backend))
        .await
        .map_err(|e| format!("audio task join failed: {e}"))?
        .map_err(|e| format!("Failed to update VAD backend: {e}"))?;

    let mut current_settings = settings::get_settings(ctx);
    current_settings.vad_backend = backend;
    settings::write_settings(ctx, current_settings);
    ctx.notify_setting_changed("vad_backend", serde_json::json!(backend));
    Ok(())
}

pub fn change_filler_word_removal_enabled_setting(
    ctx: &AppContext,
    enabled: bool,
) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    settings.filler_word_removal_enabled = enabled;
    settings::write_settings(ctx, settings);
    ctx.notify_setting_changed("filler_word_removal_enabled", serde_json::json!(enabled));
    Ok(())
}

pub fn change_app_language_setting(ctx: &AppContext, language: String) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    settings.app_language = language.clone();
    settings::write_settings(ctx, settings);

    // Refresh the tray menu with the new language
    tray::update_tray_menu(ctx);

    ctx.notify_setting_changed("app_language", serde_json::json!(language));
    Ok(())
}

pub fn change_show_tray_icon_setting(ctx: &AppContext, enabled: bool) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    settings.show_tray_icon = enabled;
    settings::write_settings(ctx, settings);

    // Apply change immediately
    tray::set_tray_visibility(ctx, enabled);

    ctx.notify_setting_changed("show_tray_icon", serde_json::json!(enabled));
    Ok(())
}

/// Save accelerator settings and make the next model use reload with them.
/// The currently running transcription, if any, keeps its existing engine.
fn save_accelerator_and_reload_next_use(ctx: &AppContext, s: settings::AppSettings) {
    settings::write_settings(ctx, s);
    ctx.transcription.reload_model_on_next_use();
}

pub fn change_transcribe_accelerator_setting(
    ctx: &AppContext,
    accelerator: settings::TranscribeAcceleratorSetting,
) -> Result<(), String> {
    let mut s = settings::get_settings(ctx);
    s.transcribe_accelerator = accelerator;
    save_accelerator_and_reload_next_use(ctx, s);
    ctx.notify_setting_changed("transcribe_accelerator", serde_json::json!(accelerator));
    Ok(())
}

pub fn change_ort_accelerator_setting(
    ctx: &AppContext,
    accelerator: settings::OrtAcceleratorSetting,
) -> Result<(), String> {
    let mut s = settings::get_settings(ctx);
    s.ort_accelerator = accelerator;
    save_accelerator_and_reload_next_use(ctx, s);
    ctx.notify_setting_changed("ort_accelerator", serde_json::json!(accelerator));
    Ok(())
}

pub fn change_transcribe_gpu_device(
    ctx: &AppContext,
    device: Option<String>,
) -> Result<(), String> {
    let mut s = settings::get_settings(ctx);
    s.transcribe_gpu_device = device.clone();
    save_accelerator_and_reload_next_use(ctx, s);
    ctx.notify_setting_changed("transcribe_gpu_device", serde_json::json!(device));
    Ok(())
}

/// Return which accelerators and GPU devices are available for this build.
///
/// First-call cost is dominated by enumerating GPU devices through the
/// transcribe.cpp Vulkan backend, which loads dynamic libraries and probes
/// hardware. Run it on the blocking pool so the main thread stays responsive —
/// see also the startup pre-warm in `lib.rs`.
pub async fn get_available_accelerators() -> crate::managers::transcription::AvailableAccelerators {
    crate::runtime::spawn_blocking(crate::managers::transcription::get_available_accelerators)
        .await
        .expect("get_available_accelerators panicked")
}

pub fn toggle_local_transcription_setting(ctx: &AppContext, enabled: bool) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    settings.local_transcription_enabled = enabled;
    settings::write_settings(ctx, settings);
    ctx.notify_setting_changed("local_transcription_enabled", serde_json::json!(enabled));
    Ok(())
}

pub fn move_transcription_provider_priority(
    ctx: &AppContext,
    provider_id: &str,
    up: bool,
) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    let len = settings.transcription_providers.len();
    if len <= 1 {
        return Ok(());
    }

    let Some(index) = settings
        .transcription_providers
        .iter()
        .position(|p| p.id == provider_id)
    else {
        return Err(format!("Provider '{}' not found", provider_id));
    };

    if (up && index == 0) || (!up && index + 1 >= len) {
        return Ok(());
    }

    let target_index = if up { index - 1 } else { index + 1 };
    settings.transcription_providers.swap(index, target_index);
    settings::write_settings(ctx, settings);
    ctx.notify_setting_changed(
        "transcription_providers_reordered",
        serde_json::json!(provider_id),
    );
    Ok(())
}

pub fn toggle_transcription_provider_enabled(
    ctx: &AppContext,
    provider_id: String,
    enabled: bool,
) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    if let Some(p) = settings.transcription_provider_mut(&provider_id) {
        p.enabled = enabled;
        settings::write_settings(ctx, settings);
        ctx.notify_setting_changed(
            "transcription_provider_enabled",
            serde_json::json!(&provider_id),
        );
        Ok(())
    } else {
        Err(format!(
            "Transcription provider '{}' not found",
            provider_id
        ))
    }
}

pub fn change_transcription_api_key_setting(
    ctx: &AppContext,
    provider_id: String,
    key: String,
) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    settings
        .transcription_api_keys
        .insert(provider_id.clone(), key);
    settings::write_settings(ctx, settings);
    ctx.notify_setting_changed("transcription_api_keys", serde_json::json!(&provider_id));
    Ok(())
}

pub fn change_transcription_model_setting(
    ctx: &AppContext,
    provider_id: String,
    model: String,
) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    if let Some(p) = settings.transcription_provider_mut(&provider_id) {
        p.model = model.clone();
    }
    settings
        .transcription_models
        .insert(provider_id.clone(), model);
    settings::write_settings(ctx, settings);
    ctx.notify_setting_changed("transcription_models", serde_json::json!(&provider_id));
    Ok(())
}

pub fn change_transcription_base_url_setting(
    ctx: &AppContext,
    provider_id: String,
    base_url: String,
) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    if let Some(p) = settings.transcription_provider_mut(&provider_id) {
        p.base_url = base_url;
        settings::write_settings(ctx, settings);
        ctx.notify_setting_changed(
            "transcription_provider_base_url",
            serde_json::json!(&provider_id),
        );
        Ok(())
    } else {
        Err(format!(
            "Transcription provider '{}' not found",
            provider_id
        ))
    }
}

pub fn change_transcription_timeout_setting(
    ctx: &AppContext,
    provider_id: String,
    timeout_seconds: u32,
) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    if let Some(p) = settings.transcription_provider_mut(&provider_id) {
        p.timeout_seconds = timeout_seconds;
        settings::write_settings(ctx, settings);
        ctx.notify_setting_changed(
            "transcription_provider_timeout",
            serde_json::json!(&provider_id),
        );
        Ok(())
    } else {
        Err(format!(
            "Transcription provider '{}' not found",
            provider_id
        ))
    }
}

pub fn update_deepgram_config(
    ctx: &AppContext,
    update_fn: impl FnOnce(&mut settings::DeepgramConfig),
) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    if let Some(p) = settings.transcription_provider_mut("deepgram") {
        let mut cfg = p.deepgram.clone().unwrap_or_default();
        update_fn(&mut cfg);
        p.deepgram = Some(cfg);
        settings::write_settings(ctx, settings);
        ctx.notify_setting_changed("deepgram_config", serde_json::json!("deepgram"));
        Ok(())
    } else {
        Err("Transcription provider 'deepgram' not found".to_string())
    }
}

pub async fn test_transcription_provider_connection(
    ctx: &AppContext,
    provider_id: String,
) -> Result<(String, u128), String> {
    let settings = settings::get_settings(ctx);
    let provider = settings
        .transcription_provider(&provider_id)
        .ok_or_else(|| format!("Provider '{}' not found", provider_id))?;

    let api_key = settings
        .transcription_api_keys
        .get(&provider_id)
        .cloned()
        .unwrap_or_default();

    let model = settings
        .transcription_models
        .get(&provider_id)
        .cloned()
        .unwrap_or_else(|| provider.model.clone());

    let language = &settings.selected_language;

    crate::stt_client::test_transcription_provider(provider, api_key, &model, language).await
}

pub fn toggle_web_provider_enabled(
    ctx: &AppContext,
    provider_id: String,
    enabled: bool,
) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    if let Some(p) = settings
        .web_providers
        .iter_mut()
        .find(|p| p.id == provider_id)
    {
        p.enabled = enabled;
        settings::write_settings(ctx, settings);
        ctx.notify_setting_changed("web_provider_enabled", serde_json::json!(provider_id));
        Ok(())
    } else {
        Err(format!("Web provider '{}' not found", provider_id))
    }
}

pub fn change_web_provider_api_key_setting(
    ctx: &AppContext,
    provider_id: String,
    api_key: String,
) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    settings.web_api_keys.insert(provider_id.clone(), api_key);
    settings::write_settings(ctx, settings);
    ctx.notify_setting_changed("web_provider_api_key", serde_json::json!(provider_id));
    Ok(())
}

pub fn change_web_provider_base_url_setting(
    ctx: &AppContext,
    provider_id: String,
    base_url: String,
) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    if let Some(p) = settings
        .web_providers
        .iter_mut()
        .find(|p| p.id == provider_id)
    {
        p.base_url = base_url;
        settings::write_settings(ctx, settings);
        ctx.notify_setting_changed("web_provider_base_url", serde_json::json!(provider_id));
        Ok(())
    } else {
        Err(format!("Web provider '{}' not found", provider_id))
    }
}

pub fn change_web_provider_timeout_setting(
    ctx: &AppContext,
    provider_id: String,
    timeout_seconds: u32,
) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    if let Some(p) = settings
        .web_providers
        .iter_mut()
        .find(|p| p.id == provider_id)
    {
        p.timeout_seconds = timeout_seconds;
        settings::write_settings(ctx, settings);
        ctx.notify_setting_changed("web_provider_timeout", serde_json::json!(provider_id));
        Ok(())
    } else {
        Err(format!("Web provider '{}' not found", provider_id))
    }
}

pub async fn test_web_provider_connection(
    ctx: &AppContext,
    provider_id: String,
) -> Result<(String, u128), String> {
    let settings = settings::get_settings(ctx);
    let provider = settings
        .web_providers
        .iter()
        .find(|p| p.id == provider_id)
        .ok_or_else(|| format!("Web provider '{}' not found", provider_id))?;

    let api_key = settings
        .web_api_keys
        .get(&provider_id)
        .cloned()
        .unwrap_or_default();

    match provider.id.as_str() {
        "tavily" => crate::web_client::tavily_test_connection(&provider.base_url, &api_key).await,
        "firecrawl" => {
            crate::web_client::firecrawl_test_connection(&provider.base_url, &api_key).await
        }
        _ => Err(format!("Unknown web provider: {}", provider.id)),
    }
}

pub async fn fetch_llm_provider_models(
    ctx: &AppContext,
    provider_id: String,
) -> Result<Vec<String>, String> {
    let settings = settings::get_settings(ctx);
    let provider = settings
        .post_process_providers
        .iter()
        .find(|p| p.id == provider_id)
        .ok_or_else(|| format!("Provider '{}' not found", provider_id))?;

    let api_key = settings
        .post_process_api_keys
        .get(&provider_id)
        .cloned()
        .unwrap_or_default();

    crate::llm_client::fetch_models(provider, api_key).await
}

pub fn change_audio_input_gain_setting(ctx: &AppContext, gain: f32) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    settings.audio_input_gain = gain;
    settings::write_settings(ctx, settings);
    ctx.notify_setting_changed("audio_input_gain", serde_json::json!(gain));
    Ok(())
}

pub fn change_audio_normalization_setting(ctx: &AppContext, enabled: bool) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    settings.audio_normalization_enabled = enabled;
    settings::write_settings(ctx, settings);
    ctx.notify_setting_changed("audio_normalization_enabled", serde_json::json!(enabled));
    Ok(())
}

pub fn change_audio_high_pass_filter_setting(
    ctx: &AppContext,
    enabled: bool,
) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    settings.audio_high_pass_filter_enabled = enabled;
    settings::write_settings(ctx, settings);
    ctx.notify_setting_changed("audio_high_pass_filter_enabled", serde_json::json!(enabled));
    Ok(())
}

pub fn change_audio_noise_reduction_setting(ctx: &AppContext, enabled: bool) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    settings.audio_noise_reduction_enabled = enabled;
    settings::write_settings(ctx, settings);
    ctx.notify_setting_changed("audio_noise_reduction_enabled", serde_json::json!(enabled));
    Ok(())
}

pub fn change_audio_noise_gate_threshold_setting(
    ctx: &AppContext,
    threshold_db: f32,
) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    settings.audio_noise_gate_threshold_db = threshold_db;
    settings::write_settings(ctx, settings);
    ctx.notify_setting_changed(
        "audio_noise_gate_threshold_db",
        serde_json::json!(threshold_db),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::{AppContext, AppPaths, EventBus};
    use crate::managers::audio::AudioRecordingManager;
    use crate::managers::history::HistoryManager;
    use crate::managers::model::ModelManager;
    use crate::managers::transcription::TranscriptionManager;
    use crate::TranscriptionCoordinator;
    use std::sync::Arc;

    fn create_test_context() -> (AppContext, tempfile::TempDir) {
        let temp_dir = tempfile::tempdir().unwrap();
        let paths = AppPaths {
            data_dir: temp_dir.path().to_path_buf(),
            resource_dir: temp_dir.path().to_path_buf(),
            log_dir: temp_dir.path().to_path_buf(),
        };
        paths.ensure_dirs().unwrap();
        let bus = EventBus::new();
        let model = Arc::new(ModelManager::new(&paths, bus.clone()).unwrap());
        let transcription =
            Arc::new(TranscriptionManager::new(&paths, bus.clone(), model.clone()).unwrap());
        let audio = Arc::new(
            AudioRecordingManager::new(&paths, bus.clone(), transcription.stream_router()).unwrap(),
        );
        let history = Arc::new(HistoryManager::new(&paths, bus.clone()).unwrap());
        let coordinator = Arc::new(TranscriptionCoordinator::new());
        let ctx = AppContext {
            paths,
            bus,
            model,
            transcription,
            audio,
            history,
            coordinator,
        };
        (ctx, temp_dir)
    }

    #[test]
    fn test_update_prompt_name_and_content_independent() {
        let (ctx, _dir) = create_test_context();
        let new_prompt = add_post_process_prompt(
            &ctx,
            "Initial Name".to_string(),
            "Initial content: ${output}".to_string(),
        )
        .unwrap();

        assert_eq!(new_prompt.name, "Initial Name");
        assert_eq!(new_prompt.prompt, "Initial content: ${output}");

        // 1. Update only name
        update_post_process_prompt_name(&ctx, new_prompt.id.clone(), "Updated Name".to_string())
            .unwrap();

        let s = settings::get_settings(&ctx);
        let p = s
            .post_process_prompts
            .iter()
            .find(|p| p.id == new_prompt.id)
            .unwrap();
        assert_eq!(p.name, "Updated Name");
        assert_eq!(p.prompt, "Initial content: ${output}");

        // 2. Update only content
        update_post_process_prompt_content(
            &ctx,
            new_prompt.id.clone(),
            "Brand new instructions: ${output}".to_string(),
        )
        .unwrap();

        let s2 = settings::get_settings(&ctx);
        let p2 = s2
            .post_process_prompts
            .iter()
            .find(|p| p.id == new_prompt.id)
            .unwrap();
        assert_eq!(p2.name, "Updated Name");
        assert_eq!(p2.prompt, "Brand new instructions: ${output}");
    }
}
