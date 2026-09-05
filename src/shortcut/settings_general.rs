//! General settings commands (split from `shortcut/mod.rs`; same behavior, same paths).

use super::{register_shortcut, unregister_shortcut};
use crate::context::{AppContext, AppEvent};
use crate::settings::{
    self, AutoSubmitKey, ClipboardHandling, OverlayPosition, OverlayStyle, PasteMethod, Theme,
    TypingTool,
};
use log::warn;

pub fn change_ptt_setting(ctx: &AppContext, enabled: bool) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    settings.push_to_talk = enabled;
    settings::write_settings(ctx, settings);
    ctx.notify_setting_changed("push_to_talk", serde_json::json!(enabled));
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
    crate::overlay::update_overlay_position(ctx);

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
    crate::overlay::update_overlay_position(ctx);

    ctx.notify_setting_changed("overlay_style", serde_json::json!(style));
    Ok(())
}

pub fn change_debug_mode_setting(ctx: &AppContext, enabled: bool) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    settings.debug_mode = enabled;
    settings::write_settings(ctx, settings);

    // Keep UI log streaming in sync: the live log viewer only exists in
    // debug mode, so logs are forwarded to the UI only while it is on.
    crate::logging::UI_LOG_STREAMING.store(enabled, std::sync::atomic::Ordering::Relaxed);

    ctx.notify_setting_changed("debug_mode", serde_json::json!(enabled));
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
