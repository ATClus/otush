//! History settings page: recent transcriptions with saved/delete/retry
//! actions, refreshed through the event bus.

use crate::commands::history as history_cmds;
use crate::context::{AppContext, AppEvent};
use gdk4::prelude::*;
use gtk4::prelude::*;
use libadwaita::prelude::*;

/// Build the History preferences page.
pub fn build(ctx: &AppContext) -> gtk4::Widget {
    let page = libadwaita::PreferencesPage::new();
    page.set_title("History");

    // --- File Transcription group ---
    let transcribe_group = libadwaita::PreferencesGroup::new();
    transcribe_group.set_title("File Transcription");
    transcribe_group.set_description(Some(
        "Drag & drop audio or video files here, or click to transcribe media files into text and subtitles.",
    ));

    let file_action_row = libadwaita::ActionRow::new();
    file_action_row.set_title("Transcribe Audio/Video File…");
    file_action_row.set_subtitle("Supports MP3, WAV, M4A, MP4, FLAC, OGG, AAC, WebM, MKV");
    file_action_row.set_activatable(true);

    let upload_btn = gtk4::Button::from_icon_name("document-open-symbolic");
    upload_btn.set_tooltip_text(Some("Open Media File Transcriber"));
    upload_btn.set_valign(gtk4::Align::Center);
    upload_btn.add_css_class("flat");
    file_action_row.add_suffix(&upload_btn);

    let ctx_dialog = ctx.clone();
    file_action_row.connect_activated(move |_| {
        crate::ui::file_transcription::show_file_transcription_dialog(&ctx_dialog, None);
    });

    let ctx_btn = ctx.clone();
    upload_btn.connect_clicked(move |_| {
        crate::ui::file_transcription::show_file_transcription_dialog(&ctx_btn, None);
    });

    transcribe_group.add(&file_action_row);
    page.add(&transcribe_group);

    // Drop target for drag-and-dropping files on the History page
    let drop_target = gtk4::DropTarget::new(gio::File::static_type(), gdk4::DragAction::COPY);
    let ctx_drop = ctx.clone();
    drop_target.connect_drop(move |_, value, _, _| {
        if let Ok(file) = value.get::<gio::File>() {
            if let Some(path) = file.path() {
                crate::ui::file_transcription::show_file_transcription_dialog(
                    &ctx_drop,
                    Some(path),
                );
                return true;
            }
        }
        false
    });
    page.add_controller(drop_target);

    // --- Retention group ---
    let retention_group = libadwaita::PreferencesGroup::new();
    retention_group.set_title("Retention");

    let limit_adjustment = gtk4::Adjustment::new(
        ctx.settings().history_limit as f64,
        1.0,
        100.0,
        1.0,
        10.0,
        0.0,
    );
    let limit_row = libadwaita::SpinRow::new(Some(&limit_adjustment), 0.0, 0);
    limit_row.set_title("History limit");
    limit_row.set_subtitle("Entries kept before the oldest are trimmed");
    limit_row.set_snap_to_ticks(true);
    limit_row.set_numeric(true);
    let limit_ctx = ctx.clone();
    limit_adjustment.connect_value_changed(move |adj| {
        let ctx = limit_ctx.clone();
        let value = adj.value() as usize;
        glib::spawn_future_local(async move {
            let _ = history_cmds::update_history_limit(&ctx, value).await;
        });
    });
    retention_group.add(&limit_row);
    page.add(&retention_group);

    // --- Entries group ---
    let entries_group = libadwaita::PreferencesGroup::new();
    entries_group.set_widget_name("history-list");
    entries_group.set_title("Recent Transcriptions");
    page.add(&entries_group);

    // Initial render + live refresh on history events.
    let ctx = ctx.clone();
    refresh_entries(&ctx, &entries_group);

    let group_for_events = glib::SendWeakRef::from(entries_group.downgrade());
    let bus = ctx.bus.clone();
    bus.subscribe(move |event| {
        let ctx = ctx.clone();
        let group = group_for_events.clone();
        glib::MainContext::default().invoke(move || {
            let weak = group.into_weak_ref();
            let Some(group) = weak.upgrade() else {
                return;
            };
            if matches!(event, AppEvent::HistoryUpdated(_)) {
                refresh_entries(&ctx, &group);
            }
        });
    });

    page.upcast::<gtk4::Widget>()
}

fn refresh_entries(ctx: &AppContext, group: &libadwaita::PreferencesGroup) {
    crate::ui::pages::clear_group_rows(group);

    let ctx = ctx.clone();
    let group_weak = glib::SendWeakRef::from(group.downgrade());
    crate::runtime::spawn(async move {
        let result = history_cmds::get_history_entries(&ctx, None, Some(20)).await;
        let ctx = ctx.clone();
        let group_weak = group_weak.clone();
        glib::MainContext::default().invoke(move || {
            let Some(group) = group_weak.into_weak_ref().upgrade() else {
                return;
            };
            match result {
                Ok(paginated) => {
                    for entry in paginated.entries {
                        let row = libadwaita::ExpanderRow::new();
                        row.set_widget_name(&entry.id.to_string());
                        row.set_title(&glib::markup_escape_text(&entry.title));
                        let ts = chrono::DateTime::from_timestamp(entry.timestamp, 0)
                            .map(|t| t.format("%Y-%m-%d %H:%M").to_string())
                            .unwrap_or_else(|| "unknown time".to_string());
                        row.set_subtitle(&ts);
                        row.set_expanded(false);

                        let primary_text = entry
                            .post_processed_text
                            .as_deref()
                            .filter(|s| !s.trim().is_empty())
                            .unwrap_or(&entry.transcription_text);

                        let has_distinct_post_process = entry
                            .post_processed_text
                            .as_deref()
                            .map(|pp| !pp.trim().is_empty() && pp != entry.transcription_text)
                            .unwrap_or(false);

                        if has_distinct_post_process {
                            let pp_text = entry.post_processed_text.as_deref().unwrap_or_default();
                            let processed_row = libadwaita::ActionRow::new();
                            processed_row.set_title("Processed Transcript");
                            processed_row.set_subtitle(&glib::markup_escape_text(pp_text));
                            processed_row.set_subtitle_lines(0);
                            processed_row.set_activatable(false);
                            let copy_pp_btn = create_copy_button(&ctx, pp_text);
                            processed_row.add_suffix(&copy_pp_btn);
                            row.add_row(&processed_row);

                            if !entry.transcription_text.trim().is_empty() {
                                let raw_row = libadwaita::ActionRow::new();
                                raw_row.set_title("Original Transcript");
                                raw_row.set_subtitle(&glib::markup_escape_text(
                                    &entry.transcription_text,
                                ));
                                raw_row.set_subtitle_lines(0);
                                raw_row.set_activatable(false);
                                let copy_raw_btn =
                                    create_copy_button(&ctx, &entry.transcription_text);
                                raw_row.add_suffix(&copy_raw_btn);
                                row.add_row(&raw_row);
                            }
                        } else {
                            let text_row = libadwaita::ActionRow::new();
                            text_row.set_title("Transcript");
                            text_row.set_subtitle(&glib::markup_escape_text(primary_text));
                            text_row.set_subtitle_lines(0);
                            text_row.set_activatable(false);
                            let copy_text_btn = create_copy_button(&ctx, primary_text);
                            text_row.add_suffix(&copy_text_btn);
                            row.add_row(&text_row);
                        }

                        // Saved toggle.
                        let saved_row = libadwaita::SwitchRow::new();
                        saved_row.set_title("Keep this entry");
                        saved_row.set_active(entry.saved);
                        let save_ctx = ctx.clone();
                        let id = entry.id;
                        saved_row.connect_active_notify(move |_r| {
                            let ctx = save_ctx.clone();
                            let id = id;
                            crate::runtime::spawn(async move {
                                let _ = history_cmds::toggle_history_entry_saved(&ctx, id).await;
                            });
                        });
                        row.add_row(&saved_row);

                        // Copy button for the expander row header.
                        let copy_button = create_copy_button(&ctx, primary_text);
                        row.add_suffix(&copy_button);

                        // Audio Playback.
                        let is_playing = history_cmds::is_playing_history_audio(entry.id);
                        let play_button = if is_playing {
                            let btn = gtk4::Button::from_icon_name("media-playback-stop-symbolic");
                            btn.set_tooltip_text(Some("Stop audio playback"));
                            btn
                        } else {
                            let btn = gtk4::Button::from_icon_name("media-playback-start-symbolic");
                            btn.set_tooltip_text(Some("Play recording audio"));
                            btn
                        };
                        play_button.set_valign(gtk4::Align::Center);

                        let audio_path = ctx.history.get_audio_file_path(&entry.file_name);
                        if !audio_path.exists() {
                            play_button.set_sensitive(false);
                            play_button.set_tooltip_text(Some("Audio recording not available"));
                        } else {
                            let play_ctx = ctx.clone();
                            let play_id = entry.id;
                            let btn_weak = glib::SendWeakRef::from(play_button.downgrade());
                            play_button.connect_clicked(move |_| {
                                let ctx = play_ctx.clone();
                                let id = play_id;
                                let btn_weak = btn_weak.clone();
                                crate::runtime::spawn(async move {
                                    match history_cmds::toggle_play_history_audio(&ctx, id).await {
                                        Ok(playing) => {
                                            glib::MainContext::default().invoke(move || {
                                                if let Some(btn) =
                                                    btn_weak.into_weak_ref().upgrade()
                                                {
                                                    if playing {
                                                        btn.set_icon_name(
                                                            "media-playback-stop-symbolic",
                                                        );
                                                        btn.set_tooltip_text(Some(
                                                            "Stop audio playback",
                                                        ));
                                                    } else {
                                                        btn.set_icon_name(
                                                            "media-playback-start-symbolic",
                                                        );
                                                        btn.set_tooltip_text(Some(
                                                            "Play recording audio",
                                                        ));
                                                    }
                                                }
                                            });
                                        }
                                        Err(e) => {
                                            log::error!("Failed to play history audio: {}", e);
                                        }
                                    }
                                });
                            });
                        }
                        play_button.add_css_class("flat");
                        row.add_suffix(&play_button);

                        // Retry
                        let retry_button = gtk4::Button::from_icon_name("view-refresh-symbolic");
                        retry_button.set_tooltip_text(Some("Retry transcription"));
                        retry_button.set_valign(gtk4::Align::Center);
                        retry_button.add_css_class("flat");
                        let retry_ctx = ctx.clone();
                        let retry_id = entry.id;
                        retry_button.connect_clicked(move |_| {
                            let ctx = retry_ctx.clone();
                            let id = retry_id;
                            crate::runtime::spawn(async move {
                                let _ =
                                    history_cmds::retry_history_entry_transcription(&ctx, id).await;
                            });
                        });
                        row.add_suffix(&retry_button);

                        // Delete
                        let delete_button = gtk4::Button::from_icon_name("user-trash-symbolic");
                        delete_button.set_tooltip_text(Some("Delete recording"));
                        delete_button.set_valign(gtk4::Align::Center);
                        delete_button.add_css_class("flat");
                        let delete_ctx = ctx.clone();
                        let delete_id = entry.id;
                        delete_button.connect_clicked(move |_| {
                            let ctx = delete_ctx.clone();
                            let id = delete_id;
                            crate::runtime::spawn(async move {
                                let _ = history_cmds::delete_history_entry(&ctx, id).await;
                            });
                        });
                        row.add_suffix(&delete_button);

                        group.add(&row);
                        crate::ui::pages::track_row(&group, &row);
                    }
                }
                Err(e) => {
                    let row = libadwaita::ActionRow::new();
                    row.set_title("Failed to load history");
                    row.set_subtitle(&e);
                    group.add(&row);
                    crate::ui::pages::track_row(&group, &row);
                }
            }
        });
    });
}

fn create_copy_button(ctx: &AppContext, text: &str) -> gtk4::Button {
    let btn = gtk4::Button::from_icon_name("edit-copy-symbolic");
    btn.set_tooltip_text(Some("Copy transcript"));
    btn.set_valign(gtk4::Align::Center);
    btn.add_css_class("flat");

    if text.trim().is_empty() {
        btn.set_sensitive(false);
        btn.set_tooltip_text(Some("Transcript is empty"));
        return btn;
    }

    let ctx = ctx.clone();
    let text = text.to_string();
    let btn_weak = glib::SendWeakRef::from(btn.downgrade());
    btn.connect_clicked(move |_| {
        let _ = crate::clipboard::write_clipboard_text(&ctx, &text);
        if let Some(display) = gdk4::Display::default() {
            display.clipboard().set_text(&text);
        }
        if let Some(btn) = btn_weak.clone().into_weak_ref().upgrade() {
            btn.set_icon_name("object-select-symbolic");
            btn.set_tooltip_text(Some("Copied!"));
            let btn_reset = glib::SendWeakRef::from(btn.downgrade());
            glib::timeout_add_local_once(std::time::Duration::from_millis(1500), move || {
                if let Some(btn) = btn_reset.into_weak_ref().upgrade() {
                    btn.set_icon_name("edit-copy-symbolic");
                    btn.set_tooltip_text(Some("Copy transcript"));
                }
            });
        }
    });
    btn
}
