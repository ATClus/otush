//! General settings page — the first fully-wired page, proving the
//! command → event → widget pattern the rest of the settings UI follows.

use crate::commands;
use crate::context::AppContext;
use libadwaita::prelude::*;

/// Build the General preferences page.
pub fn build(ctx: &AppContext) -> gtk4::Widget {
    let page = libadwaita::PreferencesPage::new();
    page.set_title("General");

    // --- Recording group ---
    let recording_group = libadwaita::PreferencesGroup::new();
    recording_group.set_title("Recording");

    // Push-to-talk
    let ptt = libadwaita::SwitchRow::new();
    ptt.set_title("Push-to-Talk");
    ptt.set_subtitle("Hold the shortcut to record; release to transcribe");
    let ptt_value = ctx.settings().push_to_talk;
    ptt.set_active(ptt_value);
    let ptt_ctx = ctx.clone();
    ptt.connect_active_notify(move |row| {
        let _ = crate::shortcut::change_ptt_setting(&ptt_ctx, row.is_active());
    });
    recording_group.add(&ptt);

    // Microphone selection (combo, populated asynchronously)
    let mic_row = libadwaita::ComboRow::new();
    mic_row.set_title("Microphone");
    mic_row.set_subtitle("Input device used for recording");
    let current = ctx.settings().selected_microphone.unwrap_or_default();
    mic_row.set_subtitle(&if current.is_empty() {
        "Default".to_string()
    } else {
        current.clone()
    });
    recording_group.add(&mic_row);
    populate_microphones(ctx, &mic_row);

    // Always-on microphone
    let always_on = libadwaita::SwitchRow::new();
    always_on.set_title("Always-On Microphone");
    always_on.set_subtitle("Keep the microphone open; detect speech automatically");
    always_on.set_active(ctx.settings().always_on_microphone);
    let always_on_ctx = ctx.clone();
    always_on.connect_active_notify(move |row| {
        let ctx = always_on_ctx.clone();
        let enabled = row.is_active();
        crate::runtime::spawn(async move {
            let _ = commands::audio::update_microphone_mode(&ctx, enabled).await;
        });
    });
    recording_group.add(&always_on);

    page.add(&recording_group);

    // --- Language group ---
    let language_group = libadwaita::PreferencesGroup::new();
    language_group.set_title("Language");

    let language_row = libadwaita::ComboRow::new();
    language_row.set_title("Transcription Language");
    language_row.set_subtitle("Language the model transcribes into");
    let languages: Vec<&str> = vec!["auto", "en", "pt", "es", "de", "fr", "ja", "zh"];
    let model = gtk4::StringList::new(&languages);
    language_row.set_model(Some(&model));
    let current = ctx.settings().selected_language.clone();
    if let Some(i) = languages.iter().position(|l| *l == current) {
        language_row.set_selected(i as u32);
    }
    let language_ctx = ctx.clone();
    language_row.connect_selected_notify(move |row| {
        if let Some(item) = row.selected_item() {
            let lang = item
                .downcast_ref::<gtk4::StringObject>()
                .map(|s| s.string().to_string())
                .unwrap_or_else(|| "auto".to_string());
            let _ = crate::shortcut::change_selected_language_setting(&language_ctx, lang);
        }
    });
    language_group.add(&language_row);

    page.add(&language_group);

    // --- Overlay group ---
    let overlay_group = libadwaita::PreferencesGroup::new();
    overlay_group.set_title("Overlay");
    overlay_group.set_description(Some(
        "A small always-on-top pill shows the recording state while you speak.",
    ));

    let overlay_style_row = libadwaita::ComboRow::new();
    overlay_style_row.set_title("Overlay style");
    let style_labels = [
        ("none", "None"),
        ("minimal", "Minimal (pill)"),
        ("live", "Live (pill + transcript)"),
    ];
    let model = gtk4::StringList::new(
        &style_labels
            .iter()
            .map(|(_, label)| *label)
            .collect::<Vec<_>>(),
    );
    overlay_style_row.set_model(Some(&model));
    let current_style = ctx.settings().overlay_style;
    overlay_style_row.set_selected(match current_style {
        crate::settings::OverlayStyle::None => 0,
        crate::settings::OverlayStyle::Minimal => 1,
        crate::settings::OverlayStyle::Live => 2,
    });
    let ctx1 = ctx.clone();
    overlay_style_row.connect_selected_notify(move |row| {
        if let Some((id, _)) = style_labels.get(row.selected() as usize) {
            let _ = crate::shortcut::change_overlay_style_setting(&ctx1, id.to_string());
        }
    });
    overlay_group.add(&overlay_style_row);

    let overlay_position_row = libadwaita::ComboRow::new();
    overlay_position_row.set_title("Overlay position");
    let position_labels = [("bottom", "Bottom"), ("top", "Top")];
    let model = gtk4::StringList::new(
        &position_labels
            .iter()
            .map(|(_, label)| *label)
            .collect::<Vec<_>>(),
    );
    overlay_position_row.set_model(Some(&model));
    overlay_position_row.set_selected(match ctx.settings().overlay_position {
        crate::settings::OverlayPosition::Top => 1,
        crate::settings::OverlayPosition::Bottom => 0,
    });
    let ctx1 = ctx.clone();
    overlay_position_row.connect_selected_notify(move |row| {
        if let Some((id, _)) = position_labels.get(row.selected() as usize) {
            let _ = crate::shortcut::change_overlay_position_setting(&ctx1, id.to_string());
        }
    });
    overlay_group.add(&overlay_position_row);

    page.add(&overlay_group);

    page.upcast::<gtk4::Widget>()
}

/// Populate the microphone combo asynchronously (cpal enumeration can stall).
fn populate_microphones(ctx: &AppContext, row: &libadwaita::ComboRow) {
    let ctx = ctx.clone();
    let row_weak = glib::SendWeakRef::from(row.downgrade());
    crate::runtime::spawn(async move {
        let devices = commands::audio::get_available_microphones().await;
        let row_weak = row_weak.clone();
        let ctx = ctx.clone();
        glib::MainContext::default().invoke(move || {
            let Some(row) = row_weak.into_weak_ref().upgrade() else {
                return;
            };
            match devices {
                Ok(devices) => {
                    let names: Vec<&str> = devices.iter().map(|d| d.name.as_str()).collect();
                    let model = gtk4::StringList::new(&names);
                    row.set_model(Some(&model));
                    let current = ctx.settings().selected_microphone.unwrap_or_default();
                    if let Some(i) = names.iter().position(|n| *n == current) {
                        row.set_selected(i as u32);
                    }
                    let row_for_select = row.clone();
                    let ctx_for_select = ctx.clone();
                    row_for_select.connect_selected_notify(move |row| {
                        if let Some(item) = row.selected_item() {
                            let name = item
                                .downcast_ref::<gtk4::StringObject>()
                                .map(|s| s.string().to_string())
                                .unwrap_or_default();
                            let ctx = ctx_for_select.clone();
                            glib::spawn_future_local(async move {
                                let _ = commands::audio::set_selected_microphone(&ctx, name).await;
                            });
                        }
                    });
                }
                Err(e) => {
                    log::warn!("Failed to enumerate microphones: {}", e);
                }
            }
        });
    });
}
