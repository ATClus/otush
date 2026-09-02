//! Advanced & System settings page: Global Shortcuts with interactive key recording,
//! Model Fine-Tuning, Diagnostics, Developer Logs, and About information.

use crate::commands;
use crate::context::AppContext;
use crate::settings::{ClipboardHandling, LogLevel, ModelUnloadTimeout, ShortcutBinding};
use crate::shortcut;
use gtk4::prelude::*;
use libadwaita::prelude::*;
use std::sync::{Arc, Mutex};

/// Build the consolidated Advanced & System preferences page.
pub fn build(ctx: &AppContext) -> gtk4::Widget {
    let page = libadwaita::PreferencesPage::new();
    page.set_title("Advanced &amp; System");
    page.set_icon_name(Some("preferences-system-symbolic"));

    let settings = ctx.settings();

    // ========================================================================
    // 1. Global Shortcuts
    // ========================================================================
    build_shortcuts(ctx, &page);

    // ========================================================================
    // 2. Model & Transcription Fine-Tuning
    // ========================================================================
    let fine_tuning_group = libadwaita::PreferencesGroup::new();
    fine_tuning_group.set_title("Model &amp; Fine-Tuning");
    fine_tuning_group.set_description(Some(
        "Memory management and vocabulary adaptation for speech recognition.",
    ));

    let unload_row = libadwaita::ComboRow::new();
    unload_row.set_title("Unload Model After Inactivity");
    unload_row.set_subtitle("Release VRAM/RAM when no recordings are made for a period");
    let unload_labels = [
        ("never", "Never (Keep in Memory)"),
        ("immediately", "Immediately"),
        ("sec15", "15 seconds"),
        ("min2", "2 minutes"),
        ("min5", "5 minutes"),
        ("min10", "10 minutes"),
        ("min15", "15 minutes"),
        ("hour1", "1 hour"),
    ];
    let model = gtk4::StringList::new(
        &unload_labels
            .iter()
            .map(|(_, label)| *label)
            .collect::<Vec<_>>(),
    );
    unload_row.set_model(Some(&model));
    if let Some(i) = unload_labels
        .iter()
        .position(|(id, _)| timeout_to_id(settings.model_unload_timeout) == *id)
    {
        unload_row.set_selected(i as u32);
    }
    let unload_ctx = ctx.clone();
    unload_row.connect_selected_notify(move |row| {
        if let Some((id, _)) = unload_labels.get(row.selected() as usize) {
            let timeout = id_to_timeout(id);
            crate::commands::transcription::set_model_unload_timeout(&unload_ctx, timeout);
        }
    });
    fine_tuning_group.add(&unload_row);

    let word_adj = gtk4::Adjustment::new(
        settings.word_correction_threshold * 100.0,
        0.0,
        100.0,
        1.0,
        5.0,
        0.0,
    );
    let word_row = libadwaita::SpinRow::new(Some(&word_adj), 1.0, 0);
    word_row.set_title("Word Correction Fuzzy Threshold (%)");
    word_row.set_subtitle("Fuzzy matching sensitivity for vocabulary substitution (0 = disabled)");
    word_row.set_snap_to_ticks(true);
    let word_ctx = ctx.clone();
    word_adj.connect_value_changed(move |adj| {
        let mut s = word_ctx.settings();
        s.word_correction_threshold = adj.value() / 100.0;
        word_ctx.write_settings(&s);
    });
    fine_tuning_group.add(&word_row);

    // Custom Vocabulary
    let custom_words_row = libadwaita::EntryRow::new();
    custom_words_row.set_title("Custom Vocabulary / Jargon (Comma-Separated)");
    custom_words_row.set_text(&settings.custom_words.join(", "));
    let cw_ctx = ctx.clone();
    custom_words_row.connect_changed(move |r| {
        let words: Vec<String> = r
            .text()
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        let mut s = cw_ctx.settings();
        s.custom_words = words;
        cw_ctx.write_settings(&s);
    });
    fine_tuning_group.add(&custom_words_row);

    page.add(&fine_tuning_group);

    // ========================================================================
    // 3. Clipboard & Automation
    // ========================================================================
    let auto_group = libadwaita::PreferencesGroup::new();
    auto_group.set_title("Clipboard &amp; Automation");

    let clipboard_row = libadwaita::ComboRow::new();
    clipboard_row.set_title("Clipboard Handling");
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
            let _ = shortcut::change_clipboard_handling_setting(&clip_ctx, id.to_string());
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
        let _ = shortcut::change_external_script_path_setting(&sc_ctx, opt);
    });
    auto_group.add(&script_row);

    let exp_row = libadwaita::SwitchRow::new();
    exp_row.set_title("Experimental Features");
    exp_row.set_subtitle("Enable work-in-progress features and developer flags");
    exp_row.set_active(settings.experimental_enabled);
    let exp_ctx = ctx.clone();
    exp_row.connect_active_notify(move |row| {
        let _ = shortcut::change_experimental_enabled_setting(&exp_ctx, row.is_active());
    });
    auto_group.add(&exp_row);

    page.add(&auto_group);

    // ========================================================================
    // 4. Diagnostics & Developer Logs
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
        let _ = shortcut::change_debug_mode_setting(&dbg_ctx, row.is_active());
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
            let _ = commands::set_log_level(&log_lvl_ctx, level);
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
        let _ = commands::open_log_dir(&log_dir_ctx);
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
        let _ = commands::open_app_data_dir(&data_dir_ctx);
    });
    data_dir_row.add_suffix(&data_btn);
    debug_group.add(&data_dir_row);

    page.add(&debug_group);

    // ========================================================================
    // 5. About Otush
    // ========================================================================
    let about_group = libadwaita::PreferencesGroup::new();
    about_group.set_title("About Otush");

    let version = crate::updater::current_version();
    let version_row = libadwaita::ActionRow::new();
    version_row.set_title("Otush Version");
    version_row.set_subtitle(&format!("v{} • Native GNOME / Wayland", version));
    version_row.set_activatable(false);

    let about_button = gtk4::Button::with_label("About Dialog");
    about_button.set_valign(gtk4::Align::Center);
    about_button.add_css_class("flat");
    let about_ctx = ctx.clone();
    about_button.connect_clicked(move |_| show_about(&about_ctx));
    version_row.add_suffix(&about_button);
    about_group.add(&version_row);

    let github_row = libadwaita::ActionRow::new();
    github_row.set_title("GitHub Repository");
    github_row.set_subtitle("github.com/ATClus/otush");
    github_row.set_activatable(true);
    let gh_btn = gtk4::Button::from_icon_name("software-update-available-symbolic");
    gh_btn.set_tooltip_text(Some("Open GitHub in Browser"));
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

fn build_shortcuts(ctx: &AppContext, page: &libadwaita::PreferencesPage) {
    let group = libadwaita::PreferencesGroup::new();
    group.set_widget_name("shortcuts");
    group.set_title("Global Shortcuts (GNOME / Wayland)");
    group.set_description(Some(
        "Global keyboard shortcuts managed via XDG Desktop Portal. Click 'Edit' or 'Record' to change combinations.",
    ));
    page.add(&group);

    rebuild_shortcuts(ctx, &group);
}

fn rebuild_shortcuts(ctx: &AppContext, group: &libadwaita::PreferencesGroup) {
    crate::ui::pages::clear_group_rows(group);

    let settings = ctx.settings();
    let mut bindings: Vec<ShortcutBinding> = settings.bindings.values().cloned().collect();
    bindings.sort_by(|a, b| a.id.cmp(&b.id));

    let default_ids = [
        "transcribe",
        "transcribe_with_post_process",
        "transcribe_meeting",
        "cancel",
        "transform_selection",
    ];

    for binding in bindings {
        let is_custom = !default_ids.contains(&binding.id.as_str());

        let row = libadwaita::ActionRow::new();
        row.set_widget_name(&format!("sc-{}", binding.id));
        row.set_title(&binding.name);

        let display_shortcut = format_shortcut_badge_string(&binding.current_binding);
        row.set_subtitle(&display_shortcut);
        row.set_activatable(false);

        // Edit Button
        let edit_button = gtk4::Button::with_label("Edit");
        edit_button.set_tooltip_text(Some("Change or record key combination"));
        edit_button.set_valign(gtk4::Align::Center);
        edit_button.add_css_class("flat");

        let ctx_edit = ctx.clone();
        let binding_clone = binding.clone();
        let group_edit = group.clone();
        edit_button.connect_clicked(move |btn| {
            show_shortcut_dialog(&ctx_edit, &group_edit, &binding_clone, btn);
        });
        row.add_suffix(&edit_button);

        if is_custom {
            // Delete button for custom shortcuts
            let del_button = gtk4::Button::from_icon_name("user-trash-symbolic");
            del_button.set_tooltip_text(Some("Remove custom shortcut"));
            del_button.set_valign(gtk4::Align::Center);
            del_button.add_css_class("flat");
            let ctx_del = ctx.clone();
            let id_del = binding.id.clone();
            let group_del = group.clone();
            del_button.connect_clicked(move |_| {
                let _ = shortcut::remove_custom_binding(&ctx_del, &id_del);
                rebuild_shortcuts(&ctx_del, &group_del);
            });
            row.add_suffix(&del_button);
        } else {
            // Reset Button for default shortcuts
            let reset_button = gtk4::Button::with_label("Reset");
            reset_button.set_tooltip_text(Some("Reset to default binding"));
            reset_button.set_valign(gtk4::Align::Center);
            reset_button.add_css_class("flat");
            let ctx_reset = ctx.clone();
            let id_reset = binding.id.clone();
            let group_reset = group.clone();
            reset_button.connect_clicked(move |_| {
                let _ = shortcut::reset_binding(&ctx_reset, id_reset.clone());
                rebuild_shortcuts(&ctx_reset, &group_reset);
            });
            row.add_suffix(&reset_button);
        }

        group.add(&row);
        crate::ui::pages::track_row(group, &row);
    }

    // Add Custom Shortcut Button Row
    let add_row = libadwaita::ActionRow::new();
    add_row.set_title("Add New Global Shortcut");
    add_row.set_subtitle("Map a dedicated hotkey to a custom prompt template or action");

    let add_btn = gtk4::Button::with_label("Add Shortcut");
    add_btn.set_valign(gtk4::Align::Center);
    add_btn.add_css_class("suggested-action");
    let ctx_add = ctx.clone();
    let group_add = group.clone();
    add_btn.connect_clicked(move |btn| {
        show_add_shortcut_dialog(&ctx_add, &group_add, btn);
    });
    add_row.add_suffix(&add_btn);
    group.add(&add_row);
    crate::ui::pages::track_row(group, &add_row);
}

/// Interactive Shortcut Recorder & Editor Dialog
fn show_shortcut_dialog(
    ctx: &AppContext,
    group: &libadwaita::PreferencesGroup,
    binding: &ShortcutBinding,
    parent_btn: &gtk4::Button,
) {
    let window = parent_btn
        .root()
        .and_then(|r| r.downcast::<gtk4::Window>().ok());

    let dialog = gtk4::Dialog::builder()
        .title(format!("Edit Shortcut: {}", binding.name))
        .transient_for(window.as_ref().unwrap_or(&gtk4::Window::new()))
        .modal(true)
        .use_header_bar(1)
        .default_width(460)
        .build();

    // Suspend global shortcuts while modal recorder is active to prevent accidental triggering
    shortcut::suspend_all_shortcuts(ctx);

    let content_area = dialog.content_area();
    content_area.set_margin_start(24);
    content_area.set_margin_end(24);
    content_area.set_margin_top(16);
    content_area.set_margin_bottom(16);
    content_area.set_spacing(16);

    let recorded_key = Arc::new(Mutex::new(binding.current_binding.clone()));

    // 1. Action Info
    let info_box = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    let title_label = gtk4::Label::new(Some(&binding.name));
    title_label.add_css_class("heading");
    title_label.set_xalign(0.0);
    info_box.append(&title_label);

    let desc_label = gtk4::Label::new(Some(&binding.description));
    desc_label.add_css_class("dim-label");
    desc_label.set_xalign(0.0);
    desc_label.set_wrap(true);
    info_box.append(&desc_label);
    content_area.append(&info_box);

    // 2. Interactive Key Capture Area
    let capture_frame = gtk4::Frame::new(None);
    capture_frame.add_css_class("card");
    let capture_box = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    capture_box.set_margin_start(16);
    capture_box.set_margin_end(16);
    capture_box.set_margin_top(16);
    capture_box.set_margin_bottom(16);

    let hint_label = gtk4::Label::new(Some("⌨️ Press your desired key combination below:"));
    hint_label.set_xalign(0.5);
    hint_label.add_css_class("caption");
    capture_box.append(&hint_label);

    let display_label = gtk4::Label::new(Some(&format_shortcut_badge_string(
        &binding.current_binding,
    )));
    display_label.add_css_class("title-1");
    display_label.set_xalign(0.5);
    capture_box.append(&display_label);

    let manual_entry = gtk4::Entry::new();
    manual_entry.set_text(&binding.current_binding);
    manual_entry.set_placeholder_text(Some("Or type manually: e.g. ctrl+alt+space"));

    capture_frame.set_child(Some(&capture_box));
    content_area.append(&capture_frame);

    // 3. Quick Presets
    let presets_box = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
    let preset_label = gtk4::Label::new(Some("Quick Common Presets:"));
    preset_label.add_css_class("dim-label");
    preset_label.add_css_class("caption");
    preset_label.set_xalign(0.0);
    presets_box.append(&preset_label);

    let flow_box = gtk4::FlowBox::new();
    flow_box.set_selection_mode(gtk4::SelectionMode::None);
    flow_box.set_max_children_per_line(4);
    flow_box.set_column_spacing(8);
    flow_box.set_row_spacing(8);

    let presets = [
        "ctrl+space",
        "super+space",
        "alt+space",
        "ctrl+shift+space",
        "ctrl+alt+space",
        "f8",
        "f9",
        "pause",
    ];

    for preset in presets {
        let btn = gtk4::Button::with_label(preset);
        btn.add_css_class("flat");
        btn.add_css_class("pill");
        let rec_k = recorded_key.clone();
        let disp_lbl = display_label.clone();
        let man_ent = manual_entry.clone();
        let p_str = preset.to_string();
        btn.connect_clicked(move |_| {
            if let Ok(mut lock) = rec_k.lock() {
                *lock = p_str.clone();
            }
            disp_lbl.set_text(&format_shortcut_badge_string(&p_str));
            man_ent.set_text(&p_str);
        });
        flow_box.insert(&btn, -1);
    }
    presets_box.append(&flow_box);
    content_area.append(&presets_box);

    // Manual Entry Row
    let entry_box = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    let entry_lbl = gtk4::Label::new(Some("Shortcut String (Normalized):"));
    entry_lbl.add_css_class("dim-label");
    entry_lbl.add_css_class("caption");
    entry_lbl.set_xalign(0.0);
    entry_box.append(&entry_lbl);

    let rec_k_entry = recorded_key.clone();
    let disp_lbl_entry = display_label.clone();
    manual_entry.connect_changed(move |e| {
        let text = e.text().trim().to_lowercase();
        if !text.is_empty() {
            if let Ok(mut lock) = rec_k_entry.lock() {
                *lock = text.clone();
            }
            disp_lbl_entry.set_text(&format_shortcut_badge_string(&text));
        }
    });
    entry_box.append(&manual_entry);
    content_area.append(&entry_box);

    // 4. Keyboard Event Controller to capture keypresses live
    let key_controller = gtk4::EventControllerKey::new();
    let rec_k_ctrl = recorded_key.clone();

    key_controller.connect_key_pressed(move |_ctrl, keyval, _keycode, state| {
        if keyval == gdk4::Key::Escape {
            return glib::Propagation::Proceed;
        }

        if let Some(combo) = format_key_combination(state, keyval) {
            if let Ok(mut lock) = rec_k_ctrl.lock() {
                *lock = combo.clone();
            }
            display_label.set_text(&format_shortcut_badge_string(&combo));
            manual_entry.set_text(&combo);
            return glib::Propagation::Stop;
        }

        glib::Propagation::Proceed
    });

    dialog.add_controller(key_controller);

    dialog.add_button("Cancel", gtk4::ResponseType::Cancel);
    let save_btn = dialog.add_button("Save Shortcut", gtk4::ResponseType::Ok);
    save_btn.add_css_class("suggested-action");

    let ctx = ctx.clone();
    let id = binding.id.clone();
    let group = group.clone();

    let ctx_close = ctx.clone();
    dialog.connect_close_request(move |_| {
        shortcut::resume_all_shortcuts(&ctx_close);
        glib::Propagation::Proceed
    });

    dialog.connect_response(move |d, resp| {
        if resp == gtk4::ResponseType::Ok {
            let final_key = recorded_key.lock().unwrap().clone();
            if !final_key.trim().is_empty() {
                let _ = shortcut::change_binding(&ctx, id.clone(), final_key);
                rebuild_shortcuts(&ctx, &group);
            }
        }
        shortcut::resume_all_shortcuts(&ctx);
        d.close();
    });

    dialog.present();
}

/// Dialog to add a brand-new custom shortcut mapping
fn show_add_shortcut_dialog(
    ctx: &AppContext,
    group: &libadwaita::PreferencesGroup,
    parent_btn: &gtk4::Button,
) {
    let window = parent_btn
        .root()
        .and_then(|r| r.downcast::<gtk4::Window>().ok());

    let dialog = gtk4::Dialog::builder()
        .title("Add New Global Shortcut")
        .transient_for(window.as_ref().unwrap_or(&gtk4::Window::new()))
        .modal(true)
        .use_header_bar(1)
        .default_width(480)
        .build();

    // Suspend global shortcuts while modal recorder is active to prevent accidental triggering
    shortcut::suspend_all_shortcuts(ctx);

    let content_area = dialog.content_area();
    content_area.set_margin_start(24);
    content_area.set_margin_end(24);
    content_area.set_margin_top(16);
    content_area.set_margin_bottom(16);
    content_area.set_spacing(16);

    let recorded_key = Arc::new(Mutex::new("ctrl+alt+t".to_string()));

    // 1. Action Type Selector
    let settings = ctx.settings();
    let prompts = settings.post_process_prompts;

    let mut action_options = vec![
        (
            "transcribe".to_string(),
            "Speech-to-Text Dictation (Standard)".to_string(),
        ),
        (
            "transcribe_with_post_process".to_string(),
            "Speech-to-Text with AI Post-Processing".to_string(),
        ),
        (
            "transcribe_meeting".to_string(),
            "Meeting Mode (Live Meets & Minutes)".to_string(),
        ),
        (
            "transform_selection".to_string(),
            "Transform Selected Text (Open AI Palette)".to_string(),
        ),
        (
            "cancel".to_string(),
            "Cancel Recording / Operation".to_string(),
        ),
    ];

    for p in &prompts {
        action_options.push((
            format!("prompt_{}", p.id),
            format!("Direct AI Prompt: {}", p.name),
        ));
    }

    let action_combo = libadwaita::ComboRow::new();
    action_combo.set_title("Action to Trigger");
    let model = gtk4::StringList::new(
        &action_options
            .iter()
            .map(|(_, label)| label.as_str())
            .collect::<Vec<_>>(),
    );
    action_combo.set_model(Some(&model));
    content_area.append(&action_combo);

    // 2. Interactive Key Capture Frame
    let capture_frame = gtk4::Frame::new(None);
    capture_frame.add_css_class("card");
    let capture_box = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    capture_box.set_margin_start(16);
    capture_box.set_margin_end(16);
    capture_box.set_margin_top(16);
    capture_box.set_margin_bottom(16);

    let hint_label = gtk4::Label::new(Some("⌨️ Press shortcut keys to record:"));
    hint_label.set_xalign(0.5);
    hint_label.add_css_class("caption");
    capture_box.append(&hint_label);

    let display_label = gtk4::Label::new(Some(&format_shortcut_badge_string("ctrl+alt+t")));
    display_label.add_css_class("title-1");
    display_label.set_xalign(0.5);
    capture_box.append(&display_label);

    capture_frame.set_child(Some(&capture_box));
    content_area.append(&capture_frame);

    // Manual Entry Row
    let manual_entry = gtk4::Entry::new();
    manual_entry.set_text("ctrl+alt+t");
    let rec_k_entry = recorded_key.clone();
    let disp_lbl_entry = display_label.clone();
    manual_entry.connect_changed(move |e| {
        let text = e.text().trim().to_lowercase();
        if !text.is_empty() {
            if let Ok(mut lock) = rec_k_entry.lock() {
                *lock = text.clone();
            }
            disp_lbl_entry.set_text(&format_shortcut_badge_string(&text));
        }
    });
    content_area.append(&manual_entry);

    // Keyboard Controller
    let key_controller = gtk4::EventControllerKey::new();
    let rec_k_ctrl = recorded_key.clone();

    key_controller.connect_key_pressed(move |_ctrl, keyval, _keycode, state| {
        if keyval == gdk4::Key::Escape {
            return glib::Propagation::Proceed;
        }

        if let Some(combo) = format_key_combination(state, keyval) {
            if let Ok(mut lock) = rec_k_ctrl.lock() {
                *lock = combo.clone();
            }
            display_label.set_text(&format_shortcut_badge_string(&combo));
            manual_entry.set_text(&combo);
            return glib::Propagation::Stop;
        }

        glib::Propagation::Proceed
    });

    dialog.add_controller(key_controller);

    dialog.add_button("Cancel", gtk4::ResponseType::Cancel);
    let add_btn = dialog.add_button("Add Shortcut", gtk4::ResponseType::Ok);
    add_btn.add_css_class("suggested-action");

    let ctx = ctx.clone();
    let group = group.clone();

    let ctx_close = ctx.clone();
    dialog.connect_close_request(move |_| {
        shortcut::resume_all_shortcuts(&ctx_close);
        glib::Propagation::Proceed
    });

    dialog.connect_response(move |d, resp| {
        let core_ids = [
            "transcribe",
            "transcribe_with_post_process",
            "transcribe_meeting",
            "transform_selection",
            "cancel",
        ];

        if resp == gtk4::ResponseType::Ok {
            let final_key = recorded_key.lock().unwrap().clone();
            let selected_idx = action_combo.selected() as usize;
            if let Some((action_id, action_name)) = action_options.get(selected_idx) {
                if core_ids.contains(&action_id.as_str()) {
                    let _ = shortcut::change_binding(&ctx, action_id.clone(), final_key);
                } else if let Some(p_id) = action_id.strip_prefix("prompt_") {
                    let binding_id = format!("custom_prompt_{}", p_id);
                    let _ = shortcut::add_custom_binding(
                        &ctx,
                        binding_id,
                        action_name.clone(),
                        format!("Direct prompt shortcut: {}", action_name),
                        final_key,
                    );
                }
                rebuild_shortcuts(&ctx, &group);
            }
        }
        shortcut::resume_all_shortcuts(&ctx);
        d.close();
    });

    dialog.present();
}

/// Convert raw key combination string (e.g. "ctrl+shift+space") into user-friendly badges (e.g. "Ctrl + Shift + Space")
fn format_shortcut_badge_string(raw: &str) -> String {
    let tokens: Vec<String> = raw
        .split('+')
        .map(|t| match t.trim().to_lowercase().as_str() {
            "ctrl" | "control" => "Ctrl".to_string(),
            "alt" | "opt" => "Alt".to_string(),
            "shift" => "Shift".to_string(),
            "super" | "win" | "meta" | "cmd" => "Super".to_string(),
            "space" => "Space".to_string(),
            "esc" | "escape" => "Esc".to_string(),
            "enter" | "return" => "Enter".to_string(),
            "tab" => "Tab".to_string(),
            "backspace" => "Backspace".to_string(),
            "delete" => "Delete".to_string(),
            "insert" => "Insert".to_string(),
            "pause" => "Pause".to_string(),
            "home" => "Home".to_string(),
            "end" => "End".to_string(),
            "pageup" => "Page Up".to_string(),
            "pagedown" => "Page Down".to_string(),
            "scroll_lock" => "Scroll Lock".to_string(),
            other if other.len() == 1 => other.to_uppercase(),
            other => {
                let mut c = other.chars();
                match c.next() {
                    Some(first) => first.to_uppercase().collect::<String>() + c.as_str(),
                    None => other.to_string(),
                }
            }
        })
        .collect();
    tokens.join(" + ")
}

fn keyval_to_shortcut_token(keyval: gdk4::Key) -> Option<String> {
    use gdk4::Key;
    match keyval {
        Key::space | Key::KP_Space => Some("space".to_string()),
        Key::Return | Key::KP_Enter | Key::ISO_Enter => Some("enter".to_string()),
        Key::Escape => Some("escape".to_string()),
        Key::Tab | Key::KP_Tab | Key::ISO_Left_Tab => Some("tab".to_string()),
        Key::BackSpace => Some("backspace".to_string()),
        Key::Delete | Key::KP_Delete => Some("delete".to_string()),
        Key::Insert | Key::KP_Insert => Some("insert".to_string()),
        Key::Pause => Some("pause".to_string()),
        Key::Home | Key::KP_Home => Some("home".to_string()),
        Key::End | Key::KP_End => Some("end".to_string()),
        Key::Page_Up | Key::KP_Page_Up => Some("pageup".to_string()),
        Key::Page_Down | Key::KP_Page_Down => Some("pagedown".to_string()),
        Key::Scroll_Lock => Some("scroll_lock".to_string()),
        Key::Caps_Lock => Some("caps_lock".to_string()),
        Key::grave | Key::asciitilde => Some("`".to_string()),
        Key::F1 | Key::KP_F1 => Some("f1".to_string()),
        Key::F2 | Key::KP_F2 => Some("f2".to_string()),
        Key::F3 | Key::KP_F3 => Some("f3".to_string()),
        Key::F4 | Key::KP_F4 => Some("f4".to_string()),
        Key::F5 => Some("f5".to_string()),
        Key::F6 => Some("f6".to_string()),
        Key::F7 => Some("f7".to_string()),
        Key::F8 => Some("f8".to_string()),
        Key::F9 => Some("f9".to_string()),
        Key::F10 => Some("f10".to_string()),
        Key::F11 => Some("f11".to_string()),
        Key::F12 => Some("f12".to_string()),
        other => {
            if is_modifier_key(other) {
                return None;
            }
            if let Some(name) = other.name() {
                let name_lower = name.to_lowercase();
                if name_lower.len() == 1 {
                    return Some(name_lower);
                }
                if name_lower.starts_with("kp_") {
                    return Some(name_lower.replace("kp_", ""));
                }
                return Some(name_lower);
            }
            None
        }
    }
}

fn is_modifier_key(key: gdk4::Key) -> bool {
    matches!(
        key,
        gdk4::Key::Control_L
            | gdk4::Key::Control_R
            | gdk4::Key::Alt_L
            | gdk4::Key::Alt_R
            | gdk4::Key::Shift_L
            | gdk4::Key::Shift_R
            | gdk4::Key::Super_L
            | gdk4::Key::Super_R
            | gdk4::Key::Meta_L
            | gdk4::Key::Meta_R
            | gdk4::Key::Hyper_L
            | gdk4::Key::Hyper_R
            | gdk4::Key::ISO_Level3_Shift
    )
}

fn format_key_combination(state: gdk4::ModifierType, keyval: gdk4::Key) -> Option<String> {
    let primary = keyval_to_shortcut_token(keyval)?;
    let mut parts = Vec::new();

    if state.contains(gdk4::ModifierType::CONTROL_MASK) {
        parts.push("ctrl");
    }
    if state.contains(gdk4::ModifierType::ALT_MASK) {
        parts.push("alt");
    }
    if state.contains(gdk4::ModifierType::SHIFT_MASK) {
        parts.push("shift");
    }
    if state.contains(gdk4::ModifierType::SUPER_MASK)
        || state.contains(gdk4::ModifierType::META_MASK)
    {
        parts.push("super");
    }

    parts.push(&primary);
    Some(parts.join("+"))
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
    about.set_comments("A free, open source, offline speech-to-text application for GNOME.");
    about.set_translator_credits("translator-credits");
    about.present();
}

fn timeout_to_id(timeout: ModelUnloadTimeout) -> &'static str {
    match timeout {
        ModelUnloadTimeout::Never => "never",
        ModelUnloadTimeout::Immediately => "immediately",
        ModelUnloadTimeout::Min2 => "min2",
        ModelUnloadTimeout::Min5 => "min5",
        ModelUnloadTimeout::Min10 => "min10",
        ModelUnloadTimeout::Min15 => "min15",
        ModelUnloadTimeout::Hour1 => "hour1",
        ModelUnloadTimeout::Sec15 => "sec15",
    }
}

fn id_to_timeout(id: &str) -> ModelUnloadTimeout {
    match id {
        "never" => ModelUnloadTimeout::Never,
        "immediately" => ModelUnloadTimeout::Immediately,
        "min2" => ModelUnloadTimeout::Min2,
        "min10" => ModelUnloadTimeout::Min10,
        "min15" => ModelUnloadTimeout::Min15,
        "hour1" => ModelUnloadTimeout::Hour1,
        "sec15" => ModelUnloadTimeout::Sec15,
        _ => ModelUnloadTimeout::Min5,
    }
}
