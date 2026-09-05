//! Advanced & About settings page: Automation integrations,
//! Diagnostics, Developer Logs, and Application Information.

use crate::commands;
use crate::context::AppContext;
use crate::settings::{ClipboardHandling, LogLevel};
use crate::shortcut;
use gtk4::prelude::*;
use libadwaita::prelude::*;

/// Build the consolidated Advanced & About preferences page.
pub fn build(ctx: &AppContext) -> gtk4::Widget {
    let page = libadwaita::PreferencesPage::new();
    page.set_title("Advanced &amp; About");
    page.set_icon_name(Some("preferences-system-symbolic"));

    let settings = ctx.settings();

    // ========================================================================
    // 1. Clipboard & Automation
    // ========================================================================
    let auto_group = libadwaita::PreferencesGroup::new();
    auto_group.set_title("Clipboard &amp; Automation");
    auto_group.set_description(Some(
        "Clipboard synchronization, external post-processing scripts, and developer flags.",
    ));

    let clipboard_row = libadwaita::ComboRow::new();
    clipboard_row.set_title("Clipboard Handling");
    clipboard_row.set_subtitle("Determine if transcribed text is written to the system clipboard");
    let clip_labels = [
        (
            "dont_modify",
            "Don't Modify (Direct Typing / Keystroke only)",
        ),
        ("copy_to_clipboard", "Copy to System Clipboard"),
    ];
    let model = gtk4::StringList::new(
        &clip_labels
            .iter()
            .map(|(_, label)| *label)
            .collect::<Vec<_>>(),
    );
    clipboard_row.set_model(Some(&model));
    if settings.clipboard_handling == ClipboardHandling::CopyToClipboard {
        clipboard_row.set_selected(1);
    }
    let clip_ctx = ctx.clone();
    clipboard_row.connect_selected_notify(move |row| {
        if let Some((id, _)) = clip_labels.get(row.selected() as usize) {
            if let Err(err) = shortcut::change_clipboard_handling_setting(&clip_ctx, id.to_string())
            {
                clip_ctx.report_error("change_clipboard_handling_setting", err);
            }
        }
    });
    auto_group.add(&clipboard_row);

    let script_row = libadwaita::EntryRow::new();
    script_row.set_title("External Script Hook Path");
    script_row.set_text(settings.external_script_path.as_deref().unwrap_or(""));
    let sc_ctx = ctx.clone();
    script_row.connect_changed(move |r| {
        let text = r.text().trim().to_string();
        let opt = if text.is_empty() { None } else { Some(text) };
        if let Err(err) = shortcut::change_external_script_path_setting(&sc_ctx, opt) {
            sc_ctx.report_error("change_external_script_path_setting", err);
        }
    });
    auto_group.add(&script_row);

    let exp_row = libadwaita::SwitchRow::new();
    exp_row.set_title("Experimental Features");
    exp_row.set_subtitle("Enable work-in-progress features and developer flags");
    exp_row.set_active(settings.experimental_enabled);
    let exp_ctx = ctx.clone();
    exp_row.connect_active_notify(move |row| {
        if let Err(err) = shortcut::change_experimental_enabled_setting(&exp_ctx, row.is_active()) {
            exp_ctx.report_error("change_experimental_enabled_setting", err);
        }
    });
    auto_group.add(&exp_row);

    page.add(&auto_group);

    // ========================================================================
    // 2. Diagnostics & Developer Logs
    // ========================================================================
    let debug_group = libadwaita::PreferencesGroup::new();
    debug_group.set_title("Diagnostics &amp; Developer Logs");
    debug_group.set_description(Some(
        "Application logging and filesystem storage paths for debugging.",
    ));

    let debug_mode_row = libadwaita::SwitchRow::new();
    debug_mode_row.set_title("Debug Mode");
    debug_mode_row.set_subtitle("Enable verbose logging and diagnostic tools");
    debug_mode_row.set_active(settings.debug_mode);
    let dbg_ctx = ctx.clone();
    debug_mode_row.connect_active_notify(move |row| {
        if let Err(err) = shortcut::change_debug_mode_setting(&dbg_ctx, row.is_active()) {
            dbg_ctx.report_error("change_debug_mode_setting", err);
        }
    });
    debug_group.add(&debug_mode_row);

    let log_level_row = libadwaita::ComboRow::new();
    log_level_row.set_title("Log Level");
    let log_labels = [
        ("trace", "Trace (Verbose)"),
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
    let log_lvl_ctx = ctx.clone();
    log_level_row.connect_selected_notify(move |row| {
        if let Some((id, _)) = log_labels.get(row.selected() as usize) {
            let level = match *id {
                "trace" => LogLevel::Trace,
                "debug" => LogLevel::Debug,
                "info" => LogLevel::Info,
                "warn" => LogLevel::Warn,
                _ => LogLevel::Error,
            };
            if let Err(err) = commands::set_log_level(&log_lvl_ctx, level) {
                log_lvl_ctx.report_error("set_log_level", err);
            }
        }
    });
    debug_group.add(&log_level_row);

    // Open Logs folder
    let log_dir_row = libadwaita::ActionRow::new();
    log_dir_row.set_title("Logs Directory");
    log_dir_row.set_subtitle(ctx.paths.log_dir.to_string_lossy().as_ref());
    let log_btn = gtk4::Button::from_icon_name("folder-open-symbolic");
    log_btn.set_tooltip_text(Some("Open Logs Folder"));
    log_btn.set_valign(gtk4::Align::Center);
    log_btn.add_css_class("flat");
    let log_dir_ctx = ctx.clone();
    log_btn.connect_clicked(move |_| {
        if let Err(err) = commands::open_log_dir(&log_dir_ctx) {
            log_dir_ctx.report_error("open_log_dir", err);
        }
    });
    log_dir_row.add_suffix(&log_btn);
    debug_group.add(&log_dir_row);

    // Open Data folder
    let data_dir_row = libadwaita::ActionRow::new();
    data_dir_row.set_title("App Data Directory");
    data_dir_row.set_subtitle(ctx.paths.data_dir.to_string_lossy().as_ref());
    let data_btn = gtk4::Button::from_icon_name("folder-open-symbolic");
    data_btn.set_tooltip_text(Some("Open App Data Folder"));
    data_btn.set_valign(gtk4::Align::Center);
    data_btn.add_css_class("flat");
    let data_dir_ctx = ctx.clone();
    data_btn.connect_clicked(move |_| {
        if let Err(err) = commands::open_app_data_dir(&data_dir_ctx) {
            data_dir_ctx.report_error("open_app_data_dir", err);
        }
    });
    data_dir_row.add_suffix(&data_btn);
    debug_group.add(&data_dir_row);

    page.add(&debug_group);

    // ========================================================================
    // 3. About Otush
    // ========================================================================
    let about_group = libadwaita::PreferencesGroup::new();
    about_group.set_title("About Otush");

    let version = crate::updater::current_version();
    let version_row = libadwaita::ActionRow::new();
    version_row.set_title("Otush");
    version_row.set_subtitle(&format!(
        "Version {} • Native GNOME / Wayland (GTK4 + libadwaita)",
        version
    ));
    version_row.set_activatable(false);

    let about_icon = gtk4::Image::from_icon_name("help-about-symbolic");
    version_row.add_prefix(&about_icon);

    let about_button = gtk4::Button::with_label("About Dialog…");
    about_button.set_valign(gtk4::Align::Center);
    about_button.add_css_class("flat");
    let about_ctx = ctx.clone();
    about_button.connect_clicked(move |_| show_about(&about_ctx));
    version_row.add_suffix(&about_button);
    about_group.add(&version_row);

    let github_row = libadwaita::ActionRow::new();
    github_row.set_title("GitHub Repository");
    github_row.set_subtitle("https://github.com/ATClus/otush");
    github_row.set_activatable(true);
    let gh_icon = gtk4::Image::from_icon_name("software-update-available-symbolic");
    github_row.add_prefix(&gh_icon);
    let gh_btn = gtk4::Button::from_icon_name("web-browser-symbolic");
    gh_btn.set_tooltip_text(Some("Open GitHub in Web Browser"));
    gh_btn.set_valign(gtk4::Align::Center);
    gh_btn.add_css_class("flat");
    github_row.connect_activated(|_| {
        let _ = opener::open("https://github.com/ATClus/otush");
    });
    github_row.add_suffix(&gh_btn);
    about_group.add(&github_row);

    page.add(&about_group);

    page.upcast::<gtk4::Widget>()
}

fn show_about(_ctx: &AppContext) {
    let version = crate::updater::current_version();
    let about = libadwaita::AboutWindow::new();
    about.set_application_name("Otush");
    about.set_version(&version);
    about.set_developer_name("Otush contributors");
    about.set_copyright("© Otush contributors");
    about.set_license_type(gtk4::License::MitX11);
    about.set_website("https://github.com/ATClus/otush");
    about.set_comments("A native, offline-first AI voice and productivity suite for GNOME.");
    about.set_translator_credits("translator-credits");
    about.present();
}
