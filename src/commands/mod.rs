//! High-level command interface for application actions invoked by UI widgets and CLI flags.

#![allow(dead_code)]
pub mod audio;
pub mod history;
pub mod models;
pub mod transcription;

use crate::context::{AppContext, AppEvent};
use crate::settings::{get_settings, write_settings, AppSettings, LogLevel};
use crate::utils::cancel_current_operation;

/// Cancel any active recording, transcription, or post-processing pipeline.
pub fn cancel_operation(ctx: &AppContext) {
    cancel_current_operation(ctx);
}

/// Check if Otush is currently running in portable directory mode.
pub fn is_portable() -> bool {
    crate::portable::is_portable()
}

/// Retrieve the absolute path of the user data directory.
pub fn get_app_dir_path(ctx: &AppContext) -> Result<String, String> {
    Ok(ctx.paths.data_dir.to_string_lossy().to_string())
}

/// Retrieve a snapshot of the current application settings.
pub fn get_app_settings(ctx: &AppContext) -> Result<AppSettings, String> {
    Ok(get_settings(ctx))
}

/// Retrieve default application settings.
pub fn get_default_settings() -> Result<AppSettings, String> {
    Ok(crate::settings::get_default_settings())
}

/// Retrieve the absolute path of the log directory.
pub fn get_log_dir_path(ctx: &AppContext) -> Result<String, String> {
    Ok(ctx.paths.log_dir.to_string_lossy().to_string())
}

/// Update and persist the application logging level.
pub fn set_log_level(ctx: &AppContext, level: LogLevel) -> Result<(), String> {
    let log_level: log::Level = match level {
        LogLevel::Trace => log::Level::Trace,
        LogLevel::Debug => log::Level::Debug,
        LogLevel::Info => log::Level::Info,
        LogLevel::Warn => log::Level::Warn,
        LogLevel::Error => log::Level::Error,
    };
    // Update the file log level atomic so the filter picks up the new level
    crate::logging::FILE_LOG_LEVEL.store(
        log_level.to_level_filter() as u8,
        std::sync::atomic::Ordering::Relaxed,
    );

    let mut settings = get_settings(ctx);
    settings.log_level = level;
    write_settings(ctx, settings);

    Ok(())
}

fn open_path(path: String) -> Result<(), String> {
    opener::open(&path).map_err(|e| format!("Failed to open {}: {}", path, e))
}

/// Open the saved audio recordings folder in the default file manager.
pub fn open_recordings_folder(ctx: &AppContext) -> Result<(), String> {
    let recordings_dir = ctx.paths.recordings_dir();
    open_path(recordings_dir.to_string_lossy().to_string())
}

/// Open the log directory in the default file manager.
pub fn open_log_dir(ctx: &AppContext) -> Result<(), String> {
    open_path(ctx.paths.log_dir.to_string_lossy().to_string())
}

/// Open the main app data directory in the default file manager.
pub fn open_app_data_dir(ctx: &AppContext) -> Result<(), String> {
    open_path(ctx.paths.data_dir.to_string_lossy().to_string())
}

/// Try to initialize Enigo (keyboard/mouse simulation).
pub fn initialize_enigo() -> Result<(), String> {
    crate::input::initialize_enigo()
}

/// Initialize keyboard shortcuts. Idempotent — calling it multiple times is
/// safe.
pub fn initialize_shortcuts(ctx: &AppContext) -> Result<(), String> {
    crate::shortcut::init_shortcuts(ctx);
    log::info!("Shortcuts initialized successfully");
    Ok(())
}

/// Notify the app to check for updates (UI opens the release page).
pub fn trigger_update_check(ctx: &AppContext) -> Result<(), String> {
    let settings = get_settings(ctx);
    if !settings.update_checks_enabled {
        return Ok(());
    }
    ctx.bus.send(AppEvent::CheckForUpdates);
    Ok(())
}

/// Show/raise the main window (tray "Settings" action, second instance).
pub fn show_main_window_command(_ctx: &AppContext) -> Result<(), String> {
    crate::app::show_main_window();
    Ok(())
}
