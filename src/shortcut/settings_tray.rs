//! Tray settings (split from `shortcut/mod.rs`; same behavior, same paths).

use crate::context::AppContext;
use crate::settings;
use crate::tray;
use log::warn;

pub fn change_show_tray_icon_setting(ctx: &AppContext, enabled: bool) -> Result<(), String> {
    let mut settings = settings::get_settings(ctx);
    settings.show_tray_icon = enabled;
    settings::write_settings(ctx, settings);

    // Apply change immediately
    tray::set_tray_visibility(ctx, enabled);

    ctx.notify_setting_changed("show_tray_icon", serde_json::json!(enabled));
    Ok(())
}

/// Change the tray icon style (`dark` for dark top bars, `light` for light
/// top bars, `colored` for full-color icons) and refresh the tray live.
pub fn change_tray_theme_setting(ctx: &AppContext, theme: String) -> Result<(), String> {
    use crate::settings::TrayTheme;

    let mut settings = settings::get_settings(ctx);
    settings.tray_theme = match theme.as_str() {
        "light" => TrayTheme::Light,
        "colored" => TrayTheme::Colored,
        "dark" => TrayTheme::Dark,
        other => {
            warn!("Invalid tray theme '{other}', defaulting to dark");
            TrayTheme::Dark
        }
    };
    settings::write_settings(ctx, settings);

    // Apply change immediately
    tray::refresh_tray_theme(ctx);

    ctx.notify_setting_changed("tray_theme", serde_json::json!(theme));
    Ok(())
}
