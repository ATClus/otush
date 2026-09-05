//! General settings page: Language, Overlay HUD, Pasting, and System Behavior.

use crate::context::AppContext;
use crate::settings::{AutoSubmitKey, PasteMethod, Theme, TrayTheme, TypingTool};
use crate::shortcut;
use gtk4::prelude::*;
use libadwaita::prelude::*;

/// Build the General preferences page.
pub fn build(ctx: &AppContext) -> gtk4::Widget {
    let page = libadwaita::PreferencesPage::new();
    page.set_title("General");
    page.set_icon_name(Some("preferences-other-symbolic"));

    let settings = ctx.settings();

    // ========================================================================
    // 1. Language & Localization
    // ========================================================================
    let language_group = libadwaita::PreferencesGroup::new();
    language_group.set_title("Language &amp; Localization");
    language_group.set_description(Some(
        "Spoken language for transcription. Select a regional preset or specify any custom BCP-47 language tag.",
    ));
    language_group.set_hexpand(true);

    let language_row = libadwaita::ComboRow::new();
    language_row.set_title("Transcription Language");
    language_row.set_subtitle("Select language preset or choose Custom");

    let lang_icon = gtk4::Image::from_icon_name("preferences-desktop-locale-symbolic");
    language_row.add_prefix(&lang_icon);

    let preset_labels: &[(&str, &str)] = &[
        ("auto", "Automatic Detection (auto)"),
        ("pt-BR", "Portuguese - Brazil (pt-BR)"),
        ("pt", "Portuguese - Portugal / Standard (pt)"),
        ("en-US", "English - United States (en-US)"),
        ("en", "English - Standard (en)"),
        ("es", "Spanish - Standard (es)"),
        ("es-419", "Spanish - Latin America (es-419)"),
        ("fr", "French (fr)"),
        ("de", "German (de)"),
        ("it", "Italian (it)"),
        ("ja", "Japanese (ja)"),
        ("ko", "Korean (ko)"),
        ("zh-CN", "Chinese - Simplified (zh-CN)"),
        ("zh-TW", "Chinese - Traditional (zh-TW)"),
        ("custom", "Custom Language Tag..."),
    ];

    let string_list = gtk4::StringList::new(
        &preset_labels
            .iter()
            .map(|(_, label)| *label)
            .collect::<Vec<_>>(),
    );
    language_row.set_model(Some(&string_list));

    let current = settings.selected_language.clone();
    let current_index = preset_labels
        .iter()
        .position(|(code, _)| *code == current)
        .unwrap_or(preset_labels.len() - 1);
    language_row.set_selected(current_index as u32);

    let is_custom_selected = current_index == preset_labels.len() - 1;

    let custom_lang_row = libadwaita::EntryRow::new();
    custom_lang_row.set_title("Custom Language Code (e.g. pt-BR, en-US)");
    custom_lang_row.set_text(&current);
    custom_lang_row.set_visible(is_custom_selected);

    let custom_row_weak = glib::SendWeakRef::from(custom_lang_row.downgrade());
    let language_ctx = ctx.clone();
    language_row.connect_selected_notify(move |row| {
        let selected = row.selected() as usize;
        if let Some(&(code, _)) = preset_labels.get(selected) {
            if let Some(r) = custom_row_weak.clone().into_weak_ref().upgrade() {
                if code == "custom" {
                    r.set_visible(true);
                } else {
                    r.set_visible(false);
                    r.set_text(code);
                    let _ =
                        shortcut::change_selected_language_setting(&language_ctx, code.to_string());
                }
            }
        }
    });

    let entry_ctx = ctx.clone();
    custom_lang_row.connect_changed(move |row| {
        let text = row.text().trim().to_string();
        if !text.is_empty() {
            if let Err(err) = shortcut::change_selected_language_setting(&entry_ctx, text) {
                entry_ctx.report_error("change_selected_language_setting", err);
            }
        }
    });

    language_group.add(&language_row);
    language_group.add(&custom_lang_row);

    // Translate to English toggle
    let translate_row = libadwaita::SwitchRow::new();
    translate_row.set_title("Translate to English");
    translate_row.set_subtitle(
        "Automatically translate foreign speech to English during local transcription",
    );
    let trans_icon = gtk4::Image::from_icon_name("accessories-dictionary-symbolic");
    translate_row.add_prefix(&trans_icon);
    translate_row.set_active(settings.translate_to_english);
    let trans_ctx = ctx.clone();
    translate_row.connect_active_notify(move |row| {
        let mut s = trans_ctx.settings();
        s.translate_to_english = row.is_active();
        trans_ctx.write_settings(&s);
    });
    language_group.add(&translate_row);

    page.add(&language_group);

    // ========================================================================
    // 2. Recording Overlay HUD
    // ========================================================================
    let overlay_group = libadwaita::PreferencesGroup::new();
    overlay_group.set_title("Recording Overlay HUD");
    overlay_group.set_description(Some(
        "A transparent HUD element displayed on screen during voice recording and processing.",
    ));
    overlay_group.set_hexpand(true);

    let overlay_style_row = libadwaita::ComboRow::new();
    overlay_style_row.set_title("Overlay Style");
    let style_icon = gtk4::Image::from_icon_name("display-symbolic");
    overlay_style_row.add_prefix(&style_icon);

    let style_labels = [
        ("none", "None (Hidden)"),
        ("minimal", "Minimal Pill"),
        ("live", "Live Streaming Text"),
    ];
    let model = gtk4::StringList::new(
        &style_labels
            .iter()
            .map(|(_, label)| *label)
            .collect::<Vec<_>>(),
    );
    overlay_style_row.set_model(Some(&model));
    let current_style = settings.overlay_style;
    overlay_style_row.set_selected(match current_style {
        crate::settings::OverlayStyle::None => 0,
        crate::settings::OverlayStyle::Minimal => 1,
        crate::settings::OverlayStyle::Live => 2,
    });

    let overlay_position_row = libadwaita::ComboRow::new();
    overlay_position_row.set_title("Overlay Screen Position");
    let pos_icon = gtk4::Image::from_icon_name("format-justify-center-symbolic");
    overlay_position_row.add_prefix(&pos_icon);

    let position_labels = [("bottom", "Bottom Center"), ("top", "Top Center")];
    let model = gtk4::StringList::new(
        &position_labels
            .iter()
            .map(|(_, label)| *label)
            .collect::<Vec<_>>(),
    );
    overlay_position_row.set_model(Some(&model));
    overlay_position_row.set_selected(match settings.overlay_position {
        crate::settings::OverlayPosition::Top => 1,
        crate::settings::OverlayPosition::Bottom => 0,
    });
    overlay_position_row.set_visible(current_style != crate::settings::OverlayStyle::None);

    let ctx1 = ctx.clone();
    let pos_row_weak = glib::SendWeakRef::from(overlay_position_row.downgrade());
    overlay_style_row.connect_selected_notify(move |row| {
        if let Some((id, _)) = style_labels.get(row.selected() as usize) {
            if let Err(err) = shortcut::change_overlay_style_setting(&ctx1, id.to_string()) {
                ctx1.report_error("change_overlay_style_setting", err);
            }
            if let Some(pos_row) = pos_row_weak.clone().into_weak_ref().upgrade() {
                pos_row.set_visible(*id != "none");
            }
        }
    });
    overlay_group.add(&overlay_style_row);

    let ctx2 = ctx.clone();
    overlay_position_row.connect_selected_notify(move |row| {
        if let Some((id, _)) = position_labels.get(row.selected() as usize) {
            if let Err(err) = shortcut::change_overlay_position_setting(&ctx2, id.to_string()) {
                ctx2.report_error("change_overlay_position_setting", err);
            }
        }
    });
    overlay_group.add(&overlay_position_row);

    page.add(&overlay_group);

    // ========================================================================
    // 3. Pasting & Keyboard Input
    // ========================================================================
    let paste_group = libadwaita::PreferencesGroup::new();
    paste_group.set_title("Pasting &amp; Text Insertion");
    paste_group.set_description(Some(
        "Configure how transcribed text is automatically pasted into active desktop applications.",
    ));
    paste_group.set_hexpand(true);

    let paste_method_row = libadwaita::ComboRow::new();
    paste_method_row.set_title("Paste Method");
    let paste_icon = gtk4::Image::from_icon_name("edit-paste-symbolic");
    paste_method_row.add_prefix(&paste_icon);

    let method_labels = [
        ("direct", "Direct (Virtual Keyboard Typing)"),
        ("ctrl_v", "Ctrl+V"),
        ("ctrl_shift_v", "Ctrl+Shift+V (Terminal)"),
        ("shift_insert", "Shift+Insert"),
        ("none", "None (Clipboard Only)"),
        ("external_script", "External Script"),
    ];
    let model = gtk4::StringList::new(
        &method_labels
            .iter()
            .map(|(_, label)| *label)
            .collect::<Vec<_>>(),
    );
    paste_method_row.set_model(Some(&model));
    paste_method_row.set_selected(match settings.paste_method {
        PasteMethod::Direct => 0,
        PasteMethod::CtrlV => 1,
        PasteMethod::CtrlShiftV => 2,
        PasteMethod::ShiftInsert => 3,
        PasteMethod::None => 4,
        PasteMethod::ExternalScript => 5,
    });

    let typing_tool_row = libadwaita::ComboRow::new();
    typing_tool_row.set_title("Direct Typing Backend");
    let typing_icon = gtk4::Image::from_icon_name("input-keyboard-symbolic");
    typing_tool_row.add_prefix(&typing_icon);

    // List only the tools actually installed on this system (probed on PATH),
    // so a missing tool cannot be selected and then fail at paste time.
    // "auto" is always present; the saved setting may name a tool that was
    // since uninstalled — keep it selectable with a warning label.
    fn tool_label(id: &str) -> &str {
        match id {
            "auto" => "Auto (Detect Environment)",
            "wtype" => "wtype (Wayland standard)",
            "ydotool" => "ydotool (uinput daemon)",
            "xdotool" => "xdotool (X11)",
            "dotool" => "dotool",
            "kwtype" => "kwtype (KDE)",
            _ => "Unknown tool",
        }
    }
    let mut tool_ids = crate::clipboard::get_available_typing_tools();
    let saved_tool_id = match settings.typing_tool {
        TypingTool::Auto => "auto",
        TypingTool::Wtype => "wtype",
        TypingTool::Ydotool => "ydotool",
        TypingTool::Xdotool => "xdotool",
        TypingTool::Dotool => "dotool",
        TypingTool::Kwtype => "kwtype",
    };
    let saved_missing = !tool_ids.iter().any(|id| id == saved_tool_id);
    if saved_missing {
        tool_ids.push(saved_tool_id.to_string());
        typing_tool_row
            .set_subtitle("Saved tool is not installed; pick an available backend or reinstall it");
    } else {
        typing_tool_row.set_subtitle("Virtual keystroke injector tool for Wayland/X11");
    }
    let tool_label_strings: Vec<String> = tool_ids
        .iter()
        .map(|id| {
            if saved_missing && id == saved_tool_id {
                format!("{} (not installed)", tool_label(id))
            } else {
                tool_label(id).to_string()
            }
        })
        .collect();
    let tool_label_refs: Vec<&str> = tool_label_strings.iter().map(String::as_str).collect();
    let model = gtk4::StringList::new(&tool_label_refs);
    typing_tool_row.set_model(Some(&model));
    typing_tool_row.set_selected(
        tool_ids
            .iter()
            .position(|id| id == saved_tool_id)
            .unwrap_or(0) as u32,
    );
    typing_tool_row.set_visible(settings.paste_method == PasteMethod::Direct);

    let ctx3 = ctx.clone();
    let typing_weak = glib::SendWeakRef::from(typing_tool_row.downgrade());
    paste_method_row.connect_selected_notify(move |row| {
        if let Some((id, _)) = method_labels.get(row.selected() as usize) {
            if let Err(err) = shortcut::change_paste_method_setting(&ctx3, id.to_string()) {
                ctx3.report_error("change_paste_method_setting", err);
            }
            if let Some(typing_row) = typing_weak.clone().into_weak_ref().upgrade() {
                typing_row.set_visible(*id == "direct");
            }
        }
    });
    paste_group.add(&paste_method_row);

    let ctx4 = ctx.clone();
    typing_tool_row.connect_selected_notify(move |row| {
        if let Some(id) = tool_ids.get(row.selected() as usize) {
            if let Err(err) = shortcut::change_typing_tool_setting(&ctx4, id.clone()) {
                ctx4.report_error("change_typing_tool_setting", err);
            }
        }
    });
    paste_group.add(&typing_tool_row);

    // Auto-Submit Switch & Key
    let auto_submit_row = libadwaita::SwitchRow::new();
    auto_submit_row.set_title("Auto-Submit After Pasting");
    auto_submit_row
        .set_subtitle("Automatically press Enter to submit chat messages or search bars");
    let submit_icon = gtk4::Image::from_icon_name("input-keyboard-symbolic");
    auto_submit_row.add_prefix(&submit_icon);
    auto_submit_row.set_active(settings.auto_submit);

    let submit_key_row = libadwaita::ComboRow::new();
    submit_key_row.set_title("Auto-Submit Key");
    let key_labels = [
        ("enter", "Enter"),
        ("ctrl_enter", "Ctrl+Enter"),
        ("cmd_enter", "Cmd+Enter"),
    ];
    let model = gtk4::StringList::new(
        &key_labels
            .iter()
            .map(|(_, label)| *label)
            .collect::<Vec<_>>(),
    );
    submit_key_row.set_model(Some(&model));
    submit_key_row.set_selected(match settings.auto_submit_key {
        AutoSubmitKey::Enter => 0,
        AutoSubmitKey::CtrlEnter => 1,
        AutoSubmitKey::CmdEnter => 2,
    });
    submit_key_row.set_visible(settings.auto_submit);

    let submit_ctx = ctx.clone();
    let submit_key_weak = glib::SendWeakRef::from(submit_key_row.downgrade());
    auto_submit_row.connect_active_notify(move |row| {
        let is_active = row.is_active();
        if let Err(err) = shortcut::change_auto_submit_setting(&submit_ctx, is_active) {
            submit_ctx.report_error("change_auto_submit_setting", err);
        }
        if let Some(key_row) = submit_key_weak.clone().into_weak_ref().upgrade() {
            key_row.set_visible(is_active);
        }
    });
    paste_group.add(&auto_submit_row);

    let key_ctx = ctx.clone();
    submit_key_row.connect_selected_notify(move |row| {
        if let Some((id, _)) = key_labels.get(row.selected() as usize) {
            if let Err(err) = shortcut::change_auto_submit_key_setting(&key_ctx, id.to_string()) {
                key_ctx.report_error("change_auto_submit_key_setting", err);
            }
        }
    });
    paste_group.add(&submit_key_row);

    let trailing_space_row = libadwaita::SwitchRow::new();
    trailing_space_row.set_title("Append Trailing Space");
    trailing_space_row.set_subtitle("Add a space after transcribed text for continuous dictation");
    let space_icon = gtk4::Image::from_icon_name("format-indent-more-symbolic");
    trailing_space_row.add_prefix(&space_icon);
    trailing_space_row.set_active(settings.append_trailing_space);
    let ts_ctx = ctx.clone();
    trailing_space_row.connect_active_notify(move |row| {
        if let Err(err) = shortcut::change_append_trailing_space_setting(&ts_ctx, row.is_active()) {
            ts_ctx.report_error("change_append_trailing_space_setting", err);
        }
    });
    paste_group.add(&trailing_space_row);

    page.add(&paste_group);

    // ========================================================================
    // 4. Appearance & System Integration
    // ========================================================================
    let system_group = libadwaita::PreferencesGroup::new();
    system_group.set_title("Appearance &amp; System Integration");
    system_group.set_hexpand(true);

    let theme_row = libadwaita::ComboRow::new();
    theme_row.set_title("Color Scheme");
    let theme_icon = gtk4::Image::from_icon_name("preferences-desktop-theme-symbolic");
    theme_row.add_prefix(&theme_icon);

    let theme_labels = [
        ("system", "Follow System"),
        ("light", "Light"),
        ("dark", "Dark"),
    ];
    let model = gtk4::StringList::new(
        &theme_labels
            .iter()
            .map(|(_, label)| *label)
            .collect::<Vec<_>>(),
    );
    theme_row.set_model(Some(&model));
    theme_row.set_selected(match settings.theme {
        Theme::System => 0,
        Theme::Light => 1,
        Theme::Dark => 2,
    });
    let theme_ctx = ctx.clone();
    theme_row.connect_selected_notify(move |row| {
        let theme_str = match row.selected() {
            1 => "light",
            2 => "dark",
            _ => "system",
        };
        if let Err(err) = shortcut::change_theme_setting(&theme_ctx, theme_str.to_string()) {
            theme_ctx.report_error("change_theme_setting", err);
        }
    });
    system_group.add(&theme_row);

    let tray_row = libadwaita::SwitchRow::new();
    tray_row.set_title("Show System Tray Icon");
    tray_row.set_subtitle("Display status icon in the top system tray");
    let tray_icon = gtk4::Image::from_icon_name("application-certificate-symbolic");
    tray_row.add_prefix(&tray_icon);
    tray_row.set_active(settings.show_tray_icon);
    let tray_ctx = ctx.clone();
    tray_row.connect_active_notify(move |row| {
        if let Err(err) = shortcut::change_show_tray_icon_setting(&tray_ctx, row.is_active()) {
            tray_ctx.report_error("change_show_tray_icon_setting", err);
        }
    });
    system_group.add(&tray_row);

    let tray_theme_row = libadwaita::ComboRow::new();
    tray_theme_row.set_title("Tray Icon Style");
    tray_theme_row.set_subtitle("Icon set for the top-bar status icon");
    let tray_theme_icon = gtk4::Image::from_icon_name("preferences-desktop-theme-symbolic");
    tray_theme_row.add_prefix(&tray_theme_icon);
    let tray_theme_labels = [
        ("dark", "Dark Bar (Light Icons)"),
        ("light", "Light Bar (Dark Icons)"),
        ("colored", "Colored"),
    ];
    let tray_theme_model = gtk4::StringList::new(
        &tray_theme_labels
            .iter()
            .map(|(_, label)| *label)
            .collect::<Vec<_>>(),
    );
    tray_theme_row.set_model(Some(&tray_theme_model));
    tray_theme_row.set_selected(match settings.tray_theme {
        TrayTheme::Dark => 0,
        TrayTheme::Light => 1,
        TrayTheme::Colored => 2,
    });
    tray_theme_row.set_visible(settings.show_tray_icon);
    let tray_theme_ctx = ctx.clone();
    tray_theme_row.connect_selected_notify(move |row| {
        if let Some((id, _)) = tray_theme_labels.get(row.selected() as usize) {
            if let Err(err) = shortcut::change_tray_theme_setting(&tray_theme_ctx, id.to_string()) {
                tray_theme_ctx.report_error("change_tray_theme_setting", err);
            }
        }
    });
    // Show the style row only while the tray icon itself is enabled.
    let tray_theme_weak = glib::SendWeakRef::from(tray_theme_row.downgrade());
    tray_row.connect_active_notify(move |row| {
        if let Some(r) = tray_theme_weak.clone().into_weak_ref().upgrade() {
            r.set_visible(row.is_active());
        }
    });
    system_group.add(&tray_theme_row);

    let autostart_row = libadwaita::SwitchRow::new();
    autostart_row.set_title("Launch at Login");
    autostart_row.set_subtitle("Start Otush in the background when logging into your desktop");
    let auto_icon = gtk4::Image::from_icon_name("system-run-symbolic");
    autostart_row.add_prefix(&auto_icon);
    autostart_row.set_active(settings.autostart_enabled);
    let auto_ctx = ctx.clone();
    autostart_row.connect_active_notify(move |row| {
        if let Err(err) = shortcut::change_autostart_setting(&auto_ctx, row.is_active()) {
            auto_ctx.report_error("change_autostart_setting", err);
        }
    });
    system_group.add(&autostart_row);

    let update_row = libadwaita::SwitchRow::new();
    update_row.set_title("Check for Updates");
    update_row.set_subtitle("Notify when a new version of Otush is released on GitHub");
    let upd_icon = gtk4::Image::from_icon_name("software-update-available-symbolic");
    update_row.add_prefix(&upd_icon);
    update_row.set_active(settings.update_checks_enabled);
    let upd_ctx = ctx.clone();
    update_row.connect_active_notify(move |row| {
        if let Err(err) = shortcut::change_update_checks_setting(&upd_ctx, row.is_active()) {
            upd_ctx.report_error("change_update_checks_setting", err);
        }
    });
    system_group.add(&update_row);

    page.add(&system_group);

    // ========================================================================
    // 5. Global Keyboard Shortcuts
    // ========================================================================
    let shortcuts_group = libadwaita::PreferencesGroup::new();
    shortcuts_group.set_title("Global Keyboard Shortcuts");
    shortcuts_group.set_description(Some(
        "Hotkeys are managed natively via GNOME System Settings to ensure seamless Wayland desktop integration.",
    ));
    shortcuts_group.set_hexpand(true);

    let configure_row = libadwaita::ActionRow::new();
    configure_row.set_title("System Shortcuts");
    configure_row.set_subtitle(
        "Open GNOME System Settings to view and customize hotkeys for voice dictation, meeting mode, notes, tasks, and palettes",
    );
    let shortcut_icon = gtk4::Image::from_icon_name("input-keyboard-symbolic");
    configure_row.add_prefix(&shortcut_icon);

    let config_btn = gtk4::Button::with_label("Configure in GNOME");
    config_btn.set_valign(gtk4::Align::Center);
    config_btn.add_css_class("suggested-action");
    let ctx_btn = ctx.clone();
    config_btn.connect_clicked(move |_| {
        shortcut::open_gnome_settings(&ctx_btn);
    });
    configure_row.add_suffix(&config_btn);
    configure_row.set_activatable_widget(Some(&config_btn));
    shortcuts_group.add(&configure_row);

    // Active shortcuts overview
    let expander = libadwaita::ExpanderRow::new();
    expander.set_title("Active Shortcuts Overview");
    expander.set_subtitle("View and customize key combinations across the Otush suite");
    let list_icon = gtk4::Image::from_icon_name("view-list-bullet-symbolic");
    expander.add_prefix(&list_icon);

    let mut bindings: Vec<_> = settings.bindings.values().cloned().collect();
    bindings.sort_by(|a, b| a.id.cmp(&b.id));

    for binding in bindings {
        if binding.id == "cancel" || binding.current_binding.trim().is_empty() {
            continue;
        }
        let row = libadwaita::ActionRow::new();
        row.set_use_markup(false);
        row.set_title(&binding.name);
        let badge = format_shortcut_badge_string(&binding.current_binding);
        row.set_subtitle(&badge);
        row.set_activatable(true);

        let icon_name = match binding.id.as_str() {
            "transcribe" => "audio-input-microphone-symbolic",
            "transcribe_with_post_process" => "starred-symbolic",
            "transcribe_meeting" => "system-users-symbolic",
            "transform_selection" => "edit-select-symbolic",
            "show_history" => "document-open-recent-symbolic",
            "search_overlay" => "system-search-symbolic",
            "agent_chat" => "chat-symbolic",
            "quick_note" => "text-editor-symbolic",
            "todo_palette" => "checkbox-checked-symbolic",
            "doc_parser" => "x-office-document-symbolic",
            _ => "applications-accessories-symbolic",
        };
        let icon = gtk4::Image::from_icon_name(icon_name);
        row.add_prefix(&icon);

        // Recapture button: opens a capture dialog for a new combination.
        let edit_ctx = ctx.clone();
        let edit_id = binding.id.clone();
        let edit_current = binding.current_binding.clone();
        let recapture_btn = gtk4::Button::from_icon_name("document-edit-symbolic");
        recapture_btn.set_tooltip_text(Some("Change shortcut"));
        recapture_btn.set_valign(gtk4::Align::Center);
        recapture_btn.add_css_class("flat");
        recapture_btn.connect_clicked(move |_| {
            show_shortcut_capture_dialog(&edit_ctx, &edit_id, &edit_current);
        });
        row.add_suffix(&recapture_btn);

        // Reset button: restore the default combination.
        let reset_ctx = ctx.clone();
        let reset_id = binding.id.clone();
        let reset_btn = gtk4::Button::from_icon_name("edit-undo-symbolic");
        reset_btn.set_tooltip_text(Some("Reset to default"));
        reset_btn.set_valign(gtk4::Align::Center);
        reset_btn.add_css_class("flat");
        reset_btn.connect_clicked(move |_| {
            if let Err(err) = shortcut::reset_binding(&reset_ctx, reset_id.clone()) {
                reset_ctx.report_error("reset_binding", err);
            }
        });
        row.add_suffix(&reset_btn);

        expander.add_row(&row);
    }

    shortcuts_group.add(&expander);
    page.add(&shortcuts_group);

    page.upcast::<gtk4::Widget>()
}

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

/// Modal capture dialog for rebinding one shortcut.
///
/// Suspends global dispatch while open (so the pressed chord cannot fire an
/// action mid-capture), captures the next key press with modifiers via an
/// `EventControllerKey`, validates it, and persists it through
/// [`shortcut::change_binding`]. Escape cancels. Dispatch is always resumed
/// when the dialog closes.
fn show_shortcut_capture_dialog(ctx: &AppContext, id: &str, current: &str) {
    crate::shortcut::suspend_all_shortcuts(ctx);

    let dialog = libadwaita::Window::new();
    dialog.set_title(Some("Press a New Shortcut"));
    dialog.set_modal(true);
    dialog.set_default_size(360, 160);

    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    content.set_margin_top(24);
    content.set_margin_bottom(24);
    content.set_margin_start(24);
    content.set_margin_end(24);

    let heading = gtk4::Label::new(Some("Press a New Shortcut"));
    heading.add_css_class("title-2");
    content.append(&heading);

    let hint = gtk4::Label::new(Some(&format!(
        "Current: {}\nPress the new key combination, or Escape to cancel.",
        format_shortcut_badge_string(current)
    )));
    hint.set_wrap(true);
    content.append(&hint);

    let status_row = libadwaita::ActionRow::new();
    status_row.set_title("Waiting for keys…");
    status_row.set_activatable(false);
    content.append(&status_row);

    let cancel_btn = gtk4::Button::with_label("Cancel");
    cancel_btn.set_halign(gtk4::Align::Center);
    content.append(&cancel_btn);

    dialog.set_content(Some(&content));

    let capture_ctx = ctx.clone();
    let capture_id = id.to_string();
    let dialog_weak = glib::SendWeakRef::from(dialog.downgrade());
    let status_weak = glib::SendWeakRef::from(status_row.downgrade());

    let dialog_weak_cancel = dialog_weak.clone();
    cancel_btn.connect_clicked(move |_| {
        if let Some(d) = dialog_weak_cancel.clone().into_weak_ref().upgrade() {
            d.close();
        }
    });

    let key_controller = gtk4::EventControllerKey::new();
    key_controller.connect_key_pressed(move |_, keyval, _keycode, state| {
        use gdk4::ModifierType;

        // Escape cancels without applying.
        if keyval == gdk4::Key::Escape {
            if let Some(d) = dialog_weak.clone().into_weak_ref().upgrade() {
                d.close();
            }
            return glib::Propagation::Stop;
        }

        // Ignore lone modifier presses; wait for the full chord.
        let name = keyval.name().map(|s| s.to_string()).unwrap_or_default();
        if name.is_empty()
            || [
                "Shift_L",
                "Shift_R",
                "Control_L",
                "Control_R",
                "Alt_L",
                "Alt_R",
                "Meta_L",
                "Meta_R",
                "Super_L",
                "Super_R",
            ]
            .contains(&name.as_str())
        {
            return glib::Propagation::Stop;
        }

        let mut parts: Vec<String> = Vec::new();
        if state.contains(ModifierType::CONTROL_MASK) {
            parts.push("Ctrl".to_string());
        }
        if state.contains(ModifierType::ALT_MASK) {
            parts.push("Alt".to_string());
        }
        if state.contains(ModifierType::SHIFT_MASK) {
            parts.push("Shift".to_string());
        }
        if state.contains(ModifierType::META_MASK) {
            parts.push("Super".to_string());
        }
        parts.push(capture_key_label(&name));
        let candidate = parts.join("+");

        match shortcut::change_binding(&capture_ctx, capture_id.clone(), candidate.clone()) {
            Ok(response) if response.success => {
                if let Some(d) = dialog_weak.clone().into_weak_ref().upgrade() {
                    d.close();
                }
            }
            Err(e) => {
                if let Some(s) = status_weak.clone().into_weak_ref().upgrade() {
                    s.set_title(&format!("Invalid: {e}"));
                }
            }
            Ok(response) => {
                if let Some(s) = status_weak.clone().into_weak_ref().upgrade() {
                    let detail = response.error.unwrap_or_else(|| "rejected".to_string());
                    s.set_title(&format!("Not applied: {detail}"));
                }
            }
        }
        glib::Propagation::Stop
    });
    dialog.add_controller(key_controller);

    let dialog_ctx = ctx.clone();
    dialog.connect_destroy(move |_| {
        crate::shortcut::resume_all_shortcuts(&dialog_ctx);
    });
    dialog.present();
}

/// Map a GDK key name to the binding-string token used by the portal engine.
fn capture_key_label(gdk_name: &str) -> String {
    match gdk_name {
        "space" => "Space".to_string(),
        "Escape" => "Escape".to_string(),
        "Return" => "Enter".to_string(),
        "Tab" => "Tab".to_string(),
        "BackSpace" => "Backspace".to_string(),
        "Delete" => "Delete".to_string(),
        "Insert" => "Insert".to_string(),
        "Pause" => "Pause".to_string(),
        "Home" => "Home".to_string(),
        "End" => "End".to_string(),
        "Page_Up" => "PageUp".to_string(),
        "Page_Down" => "PageDown".to_string(),
        "Scroll_Lock" => "Scroll_Lock".to_string(),
        single if single.len() == 1 => single.to_uppercase(),
        other => other.to_string(),
    }
}
