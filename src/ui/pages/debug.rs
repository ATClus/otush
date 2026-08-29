//! Debug settings page: log level, debug mode and folder shortcuts.

use crate::commands;
use crate::context::AppContext;
use crate::settings::LogLevel;
use crate::shortcut;
use libadwaita::prelude::*;

/// Build the Debug preferences page.
pub fn build(ctx: &AppContext) -> gtk4::Widget {
    let page = libadwaita::PreferencesPage::new();
    page.set_title("Debug");

    let settings = ctx.settings();

    // --- Debug mode ---
    let debug_group = libadwaita::PreferencesGroup::new();
    debug_group.set_title("Debug");

    let debug_mode = libadwaita::SwitchRow::new();
    debug_mode.set_title("Debug mode");
    debug_mode.set_subtitle("Enable verbose logging and debug tools");
    debug_mode.set_active(settings.debug_mode);
    let ctx1 = ctx.clone();
    debug_mode.connect_active_notify(move |row| {
        let _ = shortcut::change_debug_mode_setting(&ctx1, row.is_active());
    });
    debug_group.add(&debug_mode);

    let log_level_row = libadwaita::ComboRow::new();
    log_level_row.set_title("Log level");
    let log_labels = [
        ("trace", "Trace"),
        ("debug", "Debug"),
        ("info", "Info"),
        ("warn", "Warning"),
        ("error", "Error"),
    ];
    let model = gtk4::StringList::new(
        &log_labels
            .iter()
            .map(|(_, label)| *label)
            .collect::<Vec<_>>(),
    );
    log_level_row.set_model(Some(&model));
    let level_index = match settings.log_level {
        LogLevel::Trace => 0,
        LogLevel::Debug => 1,
        LogLevel::Info => 2,
        LogLevel::Warn => 3,
        LogLevel::Error => 4,
    };
    log_level_row.set_selected(level_index);
    let ctx1 = ctx.clone();
    log_level_row.connect_selected_notify(move |row| {
        if let Some((id, _)) = log_labels.get(row.selected() as usize) {
            let level = match *id {
                "trace" => LogLevel::Trace,
                "debug" => LogLevel::Debug,
                "info" => LogLevel::Info,
                "warn" => LogLevel::Warn,
                _ => LogLevel::Error,
            };
            let _ = commands::set_log_level(&ctx1, level);
        }
    });
    debug_group.add(&log_level_row);

    // Live log viewer (reads the log file).
    let log_viewer_button = gtk4::Button::with_label("Open log directory");
    let log_ctx = ctx.clone();
    log_viewer_button.connect_clicked(move |_| {
        let _ = commands::open_log_dir(&log_ctx);
    });
    let log_dir_row = libadwaita::ActionRow::new();
    log_dir_row.set_title("Logs");
    log_dir_row.set_subtitle(ctx.paths.log_dir.to_string_lossy().as_ref());
    log_dir_row.add_suffix(&log_viewer_button);
    debug_group.add(&log_dir_row);

    page.add(&debug_group);

    // --- Paths ---
    let paths_group = libadwaita::PreferencesGroup::new();
    paths_group.set_title("Paths");

    let data_dir_button = gtk4::Button::with_label("Open");
    let data_ctx = ctx.clone();
    data_dir_button.connect_clicked(move |_| {
        let _ = commands::open_app_data_dir(&data_ctx);
    });
    let data_dir_row = libadwaita::ActionRow::new();
    data_dir_row.set_title("App data directory");
    data_dir_row.set_subtitle(ctx.paths.data_dir.to_string_lossy().as_ref());
    data_dir_row.add_suffix(&data_dir_button);
    paths_group.add(&data_dir_row);

    let recordings_button = gtk4::Button::with_label("Open");
    let rec_ctx = ctx.clone();
    recordings_button.connect_clicked(move |_| {
        let _ = commands::open_recordings_folder(&rec_ctx);
    });
    let recordings_row = libadwaita::ActionRow::new();
    recordings_row.set_title("Recordings folder");
    recordings_row.set_subtitle(ctx.paths.recordings_dir().to_string_lossy().as_ref());
    recordings_row.add_suffix(&recordings_button);
    paths_group.add(&recordings_row);

    page.add(&paths_group);

    page.upcast::<gtk4::Widget>()
}
