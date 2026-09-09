//! High-level command interface for application actions invoked by UI widgets and CLI flags.

pub mod agents;
pub mod audio;
pub mod errors;
pub mod history;
pub mod models;
pub mod transcription;
pub mod tts;

pub use errors::{CommandError, CommandResult};

use crate::context::AppContext;
use crate::settings::{get_settings, write_settings, LogLevel};

/// Update and persist the application logging level.
pub fn set_log_level(ctx: &AppContext, level: LogLevel) -> CommandResult<()> {
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

fn open_path(path: String) -> CommandResult<()> {
    opener::open(&path).map_err(|e| CommandError::Input(format!("Failed to open {path}: {e}")))
}

/// Open the log directory in the default file manager.
pub fn open_log_dir(ctx: &AppContext) -> CommandResult<()> {
    open_path(ctx.paths.log_dir.to_string_lossy().to_string())
}

/// Open the main app data directory in the default file manager.
pub fn open_app_data_dir(ctx: &AppContext) -> CommandResult<()> {
    open_path(ctx.paths.data_dir.to_string_lossy().to_string())
}

/// Try to initialize Enigo (keyboard/mouse simulation).
pub fn initialize_enigo() -> CommandResult<()> {
    crate::input::initialize_enigo().map_err(CommandError::Input)
}

/// Initialize keyboard shortcuts. Idempotent — calling it multiple times is
/// safe.
pub fn initialize_shortcuts(ctx: &AppContext) -> CommandResult<()> {
    crate::shortcut::init_shortcuts(ctx);
    log::info!("Shortcuts initialized successfully");
    Ok(())
}

/// Show/raise the main window (tray "Settings" action, second instance).
pub fn show_main_window_command(_ctx: &AppContext) -> CommandResult<()> {
    crate::app::show_main_window();
    Ok(())
}
