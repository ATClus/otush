//! General settings page: Language, Overlay, Keyboard & Pasting, and System Behavior.

use crate::context::AppContext;
use crate::settings::{AutoSubmitKey, PasteMethod, Theme, TypingTool};
use crate::shortcut;
use libadwaita::prelude::*;

/// Build the General preferences page.
pub fn build(ctx: &AppContext) -> gtk4::Widget {
    let page = libadwaita::PreferencesPage::new();
    page.set_title("General");
    page.set_icon_name(Some("emblem-default-symbolic"));

    let settings = ctx.settings();

    // ========================================================================
    // 1. Language & Localization
    // ========================================================================
    let language_group = libadwaita::PreferencesGroup::new();
    language_group.set_title("Language &amp; Localization");
    language_group.set_description(Some(
        "Transcription language. Select a regional preset or specify any custom BCP-47 language tag.",
    ));

    let language_row = libadwaita::ComboRow::new();
    language_row.set_title("Transcription Language");
    language_row.set_subtitle("Select language preset or choose Custom");

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

    let custom_lang_row = libadwaita::EntryRow::new();
    custom_lang_row.set_title("Custom Language Code (e.g. pt-BR, en-US)");
    custom_lang_row.set_text(&current);

    let custom_row_weak = glib::SendWeakRef::from(custom_lang_row.downgrade());
    let language_ctx = ctx.clone();
    language_row.connect_selected_notify(move |row| {
        let selected = row.selected() as usize;
        if let Some(&(code, _)) = preset_labels.get(selected) {
            if code != "custom" {
                if let Some(r) = custom_row_weak.clone().into_weak_ref().upgrade() {
                    r.set_text(code);
                }
                let _ = shortcut::change_selected_language_setting(&language_ctx, code.to_string());
            }
        }
    });

    let entry_ctx = ctx.clone();
    custom_lang_row.connect_changed(move |row| {
        let text = row.text().trim().to_string();
        if !text.is_empty() {
            let _ = shortcut::change_selected_language_setting(&entry_ctx, text);
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

    let overlay_style_row = libadwaita::ComboRow::new();
    overlay_style_row.set_title("Overlay Style");
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
    let ctx1 = ctx.clone();
    overlay_style_row.connect_selected_notify(move |row| {
        if let Some((id, _)) = style_labels.get(row.selected() as usize) {
            let _ = shortcut::change_overlay_style_setting(&ctx1, id.to_string());
        }
    });
    overlay_group.add(&overlay_style_row);

    let overlay_position_row = libadwaita::ComboRow::new();
    overlay_position_row.set_title("Overlay Screen Position");
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
    let ctx2 = ctx.clone();
    overlay_position_row.connect_selected_notify(move |row| {
        if let Some((id, _)) = position_labels.get(row.selected() as usize) {
            let _ = shortcut::change_overlay_position_setting(&ctx2, id.to_string());
        }
    });
    overlay_group.add(&overlay_position_row);

    page.add(&overlay_group);

    // ========================================================================
    // 3. Pasting & Keyboard Input
    // ========================================================================
    let paste_group = libadwaita::PreferencesGroup::new();
    paste_group.set_title("Pasting &amp; Keyboard Input");
    paste_group.set_description(Some(
        "Configure how transcribed text is automatically pasted into active applications.",
    ));

    let paste_method_row = libadwaita::ComboRow::new();
    paste_method_row.set_title("Paste Method");
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
    let ctx3 = ctx.clone();
    paste_method_row.connect_selected_notify(move |row| {
        if let Some((id, _)) = method_labels.get(row.selected() as usize) {
            let _ = shortcut::change_paste_method_setting(&ctx3, id.to_string());
        }
    });
    paste_group.add(&paste_method_row);

    let typing_tool_row = libadwaita::ComboRow::new();
    typing_tool_row.set_title("Direct Typing Tool");
    typing_tool_row.set_subtitle("Virtual keystroke injector backend for Wayland/X11");
    let tool_labels = [
        ("auto", "Auto (Detect Environment)"),
        ("wtype", "wtype (Wayland standard)"),
        ("ydotool", "ydotool (uinput daemon)"),
        ("xdotool", "xdotool (X11)"),
        ("dotool", "dotool"),
        ("kwtype", "kwtype (KDE)"),
    ];
    let model = gtk4::StringList::new(
        &tool_labels
            .iter()
            .map(|(_, label)| *label)
            .collect::<Vec<_>>(),
    );
    typing_tool_row.set_model(Some(&model));
    typing_tool_row.set_selected(match settings.typing_tool {
        TypingTool::Auto => 0,
        TypingTool::Wtype => 1,
        TypingTool::Ydotool => 2,
        TypingTool::Xdotool => 3,
        TypingTool::Dotool => 4,
        TypingTool::Kwtype => 5,
    });
    let ctx4 = ctx.clone();
    typing_tool_row.connect_selected_notify(move |row| {
        if let Some((id, _)) = tool_labels.get(row.selected() as usize) {
            let _ = shortcut::change_typing_tool_setting(&ctx4, id.to_string());
        }
    });
    paste_group.add(&typing_tool_row);

    // Auto-Submit Switch & Key
    let auto_submit_row = libadwaita::SwitchRow::new();
    auto_submit_row.set_title("Auto-Submit After Pasting");
    auto_submit_row
        .set_subtitle("Automatically press Enter to submit chat messages or search bars");
    auto_submit_row.set_active(settings.auto_submit);
    let submit_ctx = ctx.clone();
    auto_submit_row.connect_active_notify(move |row| {
        let _ = shortcut::change_auto_submit_setting(&submit_ctx, row.is_active());
    });
    paste_group.add(&auto_submit_row);

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
    let key_ctx = ctx.clone();
    submit_key_row.connect_selected_notify(move |row| {
        if let Some((id, _)) = key_labels.get(row.selected() as usize) {
            let _ = shortcut::change_auto_submit_key_setting(&key_ctx, id.to_string());
        }
    });
    paste_group.add(&submit_key_row);

    let trailing_space_row = libadwaita::SwitchRow::new();
    trailing_space_row.set_title("Append Trailing Space");
    trailing_space_row.set_subtitle("Add a space after transcribed text for continuous dictation");
    trailing_space_row.set_active(settings.append_trailing_space);
    let ts_ctx = ctx.clone();
    trailing_space_row.connect_active_notify(move |row| {
        let _ = shortcut::change_append_trailing_space_setting(&ts_ctx, row.is_active());
    });
    paste_group.add(&trailing_space_row);

    page.add(&paste_group);

    // ========================================================================
    // 4. Appearance & System Integration
    // ========================================================================
    let system_group = libadwaita::PreferencesGroup::new();
    system_group.set_title("Appearance &amp; System Integration");

    let theme_row = libadwaita::ComboRow::new();
    theme_row.set_title("Color Scheme");
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
        let _ = shortcut::change_theme_setting(&theme_ctx, theme_str.to_string());
    });
    system_group.add(&theme_row);

    let tray_row = libadwaita::SwitchRow::new();
    tray_row.set_title("Show System Tray Icon");
    tray_row.set_subtitle("Show Otush status indicator in the top panel tray");
    tray_row.set_active(settings.show_tray_icon);
    let tray_ctx = ctx.clone();
    tray_row.connect_active_notify(move |row| {
        let _ = shortcut::change_show_tray_icon_setting(&tray_ctx, row.is_active());
    });
    system_group.add(&tray_row);

    let autostart_row = libadwaita::SwitchRow::new();
    autostart_row.set_title("Launch at Login");
    autostart_row.set_subtitle("Start Otush in the background when logging into your desktop");
    autostart_row.set_active(settings.autostart_enabled);
    let auto_ctx = ctx.clone();
    autostart_row.connect_active_notify(move |row| {
        let _ = shortcut::change_autostart_setting(&auto_ctx, row.is_active());
    });
    system_group.add(&autostart_row);

    let update_row = libadwaita::SwitchRow::new();
    update_row.set_title("Check for Updates");
    update_row.set_subtitle("Notify when a new version of Otush is released on GitHub");
    update_row.set_active(settings.update_checks_enabled);
    let upd_ctx = ctx.clone();
    update_row.connect_active_notify(move |row| {
        let _ = shortcut::change_update_checks_setting(&upd_ctx, row.is_active());
    });
    system_group.add(&update_row);

    page.add(&system_group);

    page.upcast::<gtk4::Widget>()
}
