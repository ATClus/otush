#![allow(dead_code)]
//! Keyboard shortcut management module
//!
//! Unified interface for keyboard shortcuts with multiple backend
//! implementations:
//!
//! - `portal`: XDG Desktop Portal GlobalShortcuts (Wayland-native; GNOME) —
//!   see [`portal`]
//! - `evdev`: uses the evdev-keys library (evdev) for more control
//!
//! The active implementation is determined by the `keyboard_implementation`
//! setting and can be changed at runtime.

pub mod evdev;
mod handler;
mod portal;

use crate::context::{AppContext, AppEvent};
use crate::settings::{
    self, get_settings, AutoSubmitKey, ClipboardHandling, KeyboardImplementation, LLMPrompt,
    OverlayPosition, OverlayStyle, PasteMethod, ShortcutBinding, SoundTheme, Theme, TypingTool,
    VadBackend, APPLE_INTELLIGENCE_PROVIDER_ID,
};
use crate::tray;
use log::{debug, error, info, warn};
use serde::Serialize;

/// Initialize shortcuts using the configured implementation
pub fn init_shortcuts(ctx: &AppContext) {
    let user_settings = settings::load_or_create_app_settings(ctx);

    // Check which implementation to use
    match user_settings.keyboard_implementation {
        KeyboardImplementation::Portal => {
            portal::init_shortcuts(ctx);
        }
        KeyboardImplementation::Evdev => {
            if let Err(e) = evdev::init_shortcuts(ctx) {
                error!("Failed to initialize evdev-keys shortcuts: {}", e);
                // Fall back to the portal implementation and persist this fallback
                warn!("Falling back to portal global shortcut implementation and saving fallback to settings");

                // Update settings to persist the fallback so we don't retry Evdev on next launch
                let mut settings = settings::get_settings(ctx);
                settings.keyboard_implementation = KeyboardImplementation::Portal;
                settings::write_settings(ctx, settings);

                portal::init_shortcuts(ctx);
            }
        }
    }
}

/// Register the cancel shortcut (called when recording starts)
pub fn register_cancel_shortcut(ctx: &AppContext) {
    let settings = get_settings(ctx);
    match settings.keyboard_implementation {
        KeyboardImplementation::Portal => portal::register_cancel_shortcut(ctx),
        KeyboardImplementation::Evdev => evdev::register_cancel_shortcut(ctx),
    }
}

/// Unregister the cancel shortcut (called when recording stops)
pub fn unregister_cancel_shortcut(ctx: &AppContext) {
    let settings = get_settings(ctx);
    match settings.keyboard_implementation {
        KeyboardImplementation::Portal => portal::unregister_cancel_shortcut(ctx),
        KeyboardImplementation::Evdev => evdev::unregister_cancel_shortcut(ctx),
    }
}

/// Register a shortcut using the appropriate implementation
pub fn register_shortcut(ctx: &AppContext, binding: ShortcutBinding) -> Result<(), String> {
    let settings = get_settings(ctx);
    match settings.keyboard_implementation {
        KeyboardImplementation::Portal => portal::register_shortcut(ctx, binding),
        KeyboardImplementation::Evdev => evdev::register_shortcut(ctx, binding),
    }
}

/// Unregister a shortcut using the appropriate implementation
pub fn unregister_shortcut(ctx: &AppContext, binding: ShortcutBinding) -> Result<(), String> {
    let settings = get_settings(ctx);
    match settings.keyboard_implementation {
        KeyboardImplementation::Portal => portal::unregister_shortcut(ctx, binding),
        KeyboardImplementation::Evdev => evdev::unregister_shortcut(ctx, binding),
    }
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
                binding: Some(b.clone()),
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
    updated_binding.current_binding = binding;

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

    // Update the binding in the settings
    settings.bindings.insert(id, updated_binding.clone());

    // Save the settings
    settings::write_settings(ctx, settings);
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

/// Unregister every binding while the user is recording a new shortcut in
/// the UI, so no existing shortcut can fire — or swallow the keystrokes —
/// mid-capture. The "cancel" binding is untouched: it is managed dynamically
/// by the recording lifecycle.
pub fn suspend_all_shortcuts(ctx: &AppContext) {
    for (id, binding) in settings::get_bindings(ctx) {
        if id == "cancel" {
            continue;
        }
        if let Err(e) = unregister_shortcut(ctx, binding) {
            debug!(
                "suspend_all_shortcuts: could not unregister '{}': {}",
                id, e
            );
        }
    }
}

/// Re-register every binding from settings after shortcut recording ends.
/// Registering an already-registered shortcut fails cleanly in both
/// implementations, so this is idempotent and safe on every exit path.
pub fn resume_all_shortcuts(ctx: &AppContext) {
    let settings = get_settings(ctx);
    for (id, binding) in &settings.bindings {
        if id == "cancel" {
            continue;
        }
        if id == "transcribe_with_post_process" && !settings.post_process_enabled {
            continue;
        }
        if let Err(e) = register_shortcut(ctx, binding.clone()) {
            debug!("resume_all_shortcuts: could not register '{}': {}", id, e);
        }
    }
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
// Keyboard Implementation Switching
// ============================================================================

/// Result of changing keyboard implementation
#[derive(Serialize)]
pub struct ImplementationChangeResult {
    pub success: bool,
    /// List of binding IDs that were reset to defaults due to incompatibility
    pub reset_bindings: Vec<String>,
}

/// Change the keyboard implementation with runtime switching.
/// This will unregister all shortcuts from the old implementation,
/// validate shortcuts for the new implementation (resetting invalid ones to defaults),
/// and register them with the new implementation.
pub fn change_keyboard_implementation_setting(
    ctx: &AppContext,
    implementation: String,
) -> Result<ImplementationChangeResult, String> {
    let current_settings = settings::get_settings(ctx);
    let current_impl = current_settings.keyboard_implementation;
    let new_impl = parse_keyboard_implementation(&implementation);

    // If same implementation, nothing to do
    if current_impl == new_impl {
        return Ok(ImplementationChangeResult {
            success: true,
            reset_bindings: vec![],
        });
    }

    info!(
        "Switching keyboard implementation from {:?} to {:?}",
        current_impl, new_impl
    );

    // Unregister all shortcuts from the current implementation
    unregister_all_shortcuts(ctx, current_impl);

    // Update the setting
    let mut settings = settings::get_settings(ctx);
    settings.keyboard_implementation = new_impl;
    settings::write_settings(ctx, settings);

    // Initialize new implementation if needed (Evdev needs state)
    if new_impl == KeyboardImplementation::Evdev && initialize_evdev_with_rollback(ctx)? {
        // Shortcuts already registered during init.
        return Ok(ImplementationChangeResult {
            success: true,
            reset_bindings: vec![],
        });
    }

    // Register all shortcuts with new implementation, resetting invalid ones
    let reset_bindings = register_all_shortcuts_for_implementation(ctx, new_impl);

    // Emit event to notify the UI of the change
    ctx.bus.send(AppEvent::SettingsChanged {
        setting: "keyboard_implementation".to_string(),
        value: serde_json::json!({
            "value": implementation,
            "reset_bindings": reset_bindings
        }),
    });

    info!("Keyboard implementation switched to {:?}", new_impl);

    Ok(ImplementationChangeResult {
        success: true,
        reset_bindings,
    })
}

/// Get the current keyboard implementation
pub fn get_keyboard_implementation(ctx: &AppContext) -> String {
    let settings = settings::get_settings(ctx);
    match settings.keyboard_implementation {
        KeyboardImplementation::Portal => "portal".to_string(),
        KeyboardImplementation::Evdev => "evdev".to_string(),
    }
}

// ============================================================================
// Validation Helpers
// ============================================================================

/// Validate a shortcut for a specific implementation
fn validate_shortcut_for_implementation(
    raw: &str,
    implementation: KeyboardImplementation,
) -> Result<(), String> {
    match implementation {
        KeyboardImplementation::Portal => portal::validate_shortcut(raw),
        KeyboardImplementation::Evdev => evdev::validate_shortcut(raw),
    }
}

/// Parse a keyboard implementation string into the enum
fn parse_keyboard_implementation(s: &str) -> KeyboardImplementation {
    match s {
        "portal" => KeyboardImplementation::Portal,
        "evdev" => KeyboardImplementation::Evdev,
        other => {
            warn!(
                "Invalid keyboard implementation '{}', defaulting to portal",
                other
            );
            KeyboardImplementation::Portal
        }
    }
}

/// Unregister all shortcuts for the current implementation
fn unregister_all_shortcuts(ctx: &AppContext, implementation: KeyboardImplementation) {
    let bindings = settings::get_bindings(ctx);

    for (id, binding) in bindings {
        // Skip cancel shortcut as it's dynamically registered
        if id == "cancel" {
            continue;
        }

        let result = match implementation {
            KeyboardImplementation::Portal => portal::unregister_shortcut(ctx, binding),
            KeyboardImplementation::Evdev => evdev::unregister_shortcut(ctx, binding),
        };

        if let Err(e) = result {
            warn!(
                "Failed to unregister shortcut '{}' during switch: {}",
                id, e
            );
        }
    }
}

/// Register all shortcuts for a specific implementation, validating and resetting invalid ones
fn register_all_shortcuts_for_implementation(
    ctx: &AppContext,
    implementation: KeyboardImplementation,
) -> Vec<String> {
    let mut reset_bindings = Vec::new();
    let default_bindings = settings::get_default_settings().bindings;
    let mut current_settings = settings::get_settings(ctx);

    for (id, default_binding) in &default_bindings {
        // Skip cancel shortcut as it's dynamically registered
        if id == "cancel" {
            continue;
        }

        // Skip post-processing shortcut when the feature is disabled
        if id == "transcribe_with_post_process" && !current_settings.post_process_enabled {
            continue;
        }

        let mut binding = current_settings
            .bindings
            .get(id)
            .cloned()
            .unwrap_or_else(|| default_binding.clone());

        // Validate the shortcut for the target implementation
        if let Err(e) =
            validate_shortcut_for_implementation(&binding.current_binding, implementation)
        {
            info!(
                "Shortcut '{}' ({}) is invalid for {:?}: {}. Resetting to default.",
                id, binding.current_binding, implementation, e
            );

            // Reset to default
            binding.current_binding = default_binding.current_binding.clone();
            current_settings
                .bindings
                .insert(id.clone(), binding.clone());
            reset_bindings.push(id.clone());
        }

        // Register with the appropriate implementation
        let result = match implementation {
            KeyboardImplementation::Portal => portal::register_shortcut(ctx, binding),
            KeyboardImplementation::Evdev => evdev::register_shortcut(ctx, binding),
        };

        if let Err(e) = result {
            error!(
                "Failed to register shortcut '{}' for {:?}: {}",
                id, implementation, e
            );
        }
    }

    // Save settings if any bindings were reset
    if !reset_bindings.is_empty() {
        settings::write_settings(ctx, current_settings);
    }

    reset_bindings
}

/// Initialize Evdev if not already initialized, with rollback on failure
fn initialize_evdev_with_rollback(ctx: &AppContext) -> Result<bool, String> {
    if evdev::is_initialized() {
        return Ok(false); // Already initialized, caller should continue
    }

    if let Err(e) = evdev::init_shortcuts(ctx) {
        error!("Failed to initialize Evdev: {}", e);
        // Rollback to portal
        let mut settings = settings::get_settings(ctx);
        settings.keyboard_implementation = KeyboardImplementation::Portal;
        settings::write_settings(ctx, settings);
        portal::init_shortcuts(ctx);
        return Err(format!(
            "Failed to initialize Evdev: {}. Reverted to portal.",
            e
        ));
    }

    // init_shortcuts already registered shortcuts
    Ok(true)
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
        id: id.clone(),
        name,
        prompt,
    };

    settings.post_process_prompts.push(new_prompt.clone());
    ctx.notify_setting_changed(
        "post_process_prompts",
        serde_json::json!(&settings.post_process_prompts),
    );
    settings::write_settings(ctx, settings);

    Ok(new_prompt)
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
        existing_prompt.name = name;
        existing_prompt.prompt = prompt;
        ctx.notify_setting_changed(
            "post_process_prompts",
            serde_json::json!(&settings.post_process_prompts),
        );
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
        "post_process_prompts",
        serde_json::json!(&settings.post_process_prompts),
    );
    settings::write_settings(ctx, settings);
    Ok(())
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

    if provider.id == APPLE_INTELLIGENCE_PROVIDER_ID {
        return Err(
            "Apple Intelligence is only available on Apple silicon Macs running macOS 15 or later."
                .to_string(),
        );
    }

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
