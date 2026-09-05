//! History page: global application activity covering standard
//! voice transcriptions, meeting minutes, AI prompt transformations, and file transcriptions.

use crate::commands::history as history_cmds;
use crate::context::{AppContext, AppEvent};
use crate::managers::history::HistoryEntry;
use gdk4::prelude::*;
use gtk4::prelude::*;
use libadwaita::prelude::*;
use std::cell::RefCell;
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CategoryFilter {
    All,
    Voice,
    Meeting,
    PostProcess,
    File,
}

thread_local! {
    static HISTORY_TRACKED: RefCell<Vec<(HistoryEntry, libadwaita::ExpanderRow)>> =
        const { RefCell::new(Vec::new()) };
    static HISTORY_FILTER: RefCell<CategoryFilter> = const { RefCell::new(CategoryFilter::All) };
    static HISTORY_SEARCH: RefCell<String> = const { RefCell::new(String::new()) };
}

fn apply_history_filter() {
    HISTORY_TRACKED.with(|tracked| {
        let cat = HISTORY_FILTER.with(|f| *f.borrow());
        let query = HISTORY_SEARCH.with(|s| s.borrow().clone());
        let query_lower = query.trim().to_lowercase();

        for (entry, row) in tracked.borrow().iter() {
            let matches_cat = match cat {
                CategoryFilter::All => true,
                CategoryFilter::Voice => entry.entry_kind == "transcription",
                CategoryFilter::Meeting => entry.entry_kind == "meeting",
                CategoryFilter::PostProcess => entry.entry_kind == "post_process",
                CategoryFilter::File => entry.entry_kind == "file",
            };

            let matches_search = if query_lower.is_empty() {
                true
            } else {
                entry.title.to_lowercase().contains(&query_lower)
                    || entry
                        .transcription_text
                        .to_lowercase()
                        .contains(&query_lower)
                    || entry
                        .post_processed_text
                        .as_deref()
                        .map(|pp| pp.to_lowercase().contains(&query_lower))
                        .unwrap_or(false)
                    || entry
                        .post_process_prompt
                        .as_deref()
                        .map(|pr| pr.to_lowercase().contains(&query_lower))
                        .unwrap_or(false)
            };

            row.set_visible(matches_cat && matches_search);
        }
    });
}

/// Build the Global History preferences page.
pub fn build(ctx: &AppContext) -> gtk4::Widget {
    let page = libadwaita::PreferencesPage::new();
    page.set_title("History");
    page.set_icon_name(Some("document-open-recent-symbolic"));

    // ========================================================================
    // 1. Filter & Search Controls (Top priority for workspace exploration)
    // ========================================================================
    let filter_group = libadwaita::PreferencesGroup::new();
    filter_group.set_title("Filter &amp; Search");
    filter_group.set_hexpand(true);

    let controls_box = gtk4::Box::new(gtk4::Orientation::Vertical, 10);
    controls_box.set_margin_top(4);
    controls_box.set_margin_bottom(8);
    controls_box.set_hexpand(true);

    let search_entry = gtk4::SearchEntry::new();
    search_entry.set_placeholder_text(Some("Search transcripts, meeting minutes, or AI text…"));
    search_entry.set_hexpand(true);
    controls_box.append(&search_entry);

    // Segmented linked toggle bar with native GNOME symbolic icons
    let filter_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    filter_row.add_css_class("linked");
    filter_row.set_halign(gtk4::Align::Center);
    filter_row.set_margin_top(2);
    filter_row.set_margin_bottom(2);

    let filter_categories = [
        (CategoryFilter::All, "All", "view-grid-symbolic"),
        (
            CategoryFilter::Voice,
            "Voice",
            "audio-input-microphone-symbolic",
        ),
        (CategoryFilter::Meeting, "Meetings", "system-users-symbolic"),
        (CategoryFilter::PostProcess, "AI Polish", "starred-symbolic"),
        (CategoryFilter::File, "Files", "document-open-symbolic"),
    ];

    let mut group_btn: Option<gtk4::ToggleButton> = None;
    for (cat, label, icon_name) in filter_categories {
        let btn = gtk4::ToggleButton::new();
        btn.set_group(group_btn.as_ref());
        if group_btn.is_none() {
            group_btn = Some(btn.clone());
            btn.set_active(true);
        }

        let btn_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        let icon = gtk4::Image::from_icon_name(icon_name);
        icon.set_pixel_size(16);
        btn_box.append(&icon);
        let lbl = gtk4::Label::new(Some(label));
        btn_box.append(&lbl);
        btn.set_child(Some(&btn_box));

        btn.connect_toggled(move |b| {
            if b.is_active() {
                HISTORY_FILTER.with(|f| *f.borrow_mut() = cat);
                apply_history_filter();
            }
        });

        filter_row.append(&btn);
    }
    controls_box.append(&filter_row);

    // Open Overlay Quick Action Row
    let overlay_action_row = libadwaita::ActionRow::new();
    overlay_action_row.set_title("Quick History Overlay");
    overlay_action_row
        .set_subtitle("Open floating quick-access history palette (Shortcut: Ctrl+Alt+H)");
    overlay_action_row.set_activatable(true);

    let overlay_icon = gtk4::Image::from_icon_name("window-new-symbolic");
    overlay_action_row.add_prefix(&overlay_icon);

    let open_overlay_btn = gtk4::Button::from_icon_name("window-new-symbolic");
    open_overlay_btn.set_tooltip_text(Some("Open History Overlay"));
    open_overlay_btn.set_valign(gtk4::Align::Center);
    open_overlay_btn.add_css_class("flat");
    overlay_action_row.add_suffix(&open_overlay_btn);

    let ctx_overlay = ctx.clone();
    overlay_action_row.connect_activated(move |_| {
        crate::ui::history_palette::show_history_palette(&ctx_overlay);
    });
    let ctx_overlay_btn = ctx.clone();
    open_overlay_btn.connect_clicked(move |_| {
        crate::ui::history_palette::show_history_palette(&ctx_overlay_btn);
    });

    filter_group.add(&controls_box);
    filter_group.add(&overlay_action_row);
    page.add(&filter_group);

    // ========================================================================
    // 2. Entries Group (Main Activity Records)
    // ========================================================================
    let entries_group = libadwaita::PreferencesGroup::new();
    entries_group.set_widget_name("history-list");
    entries_group.set_title("Activity &amp; Transcripts");
    entries_group.set_description(Some(
        "Recorded voice dictations, meeting summaries, AI transformations, and media files.",
    ));
    entries_group.set_hexpand(true);
    page.add(&entries_group);

    // ========================================================================
    // 3. Retention Group (Secondary preference placed neatly at the bottom)
    // ========================================================================
    let retention_group = libadwaita::PreferencesGroup::new();
    retention_group.set_title("Storage &amp; Retention");
    retention_group.set_hexpand(true);

    let limit_adjustment = gtk4::Adjustment::new(
        ctx.settings().history_limit as f64,
        1.0,
        100.0,
        1.0,
        10.0,
        0.0,
    );
    let limit_row = libadwaita::SpinRow::new(Some(&limit_adjustment), 0.0, 0);
    limit_row.set_title("History Retention Limit");
    limit_row
        .set_subtitle("Number of entries preserved before oldest items are automatically trimmed");
    limit_row.set_snap_to_ticks(true);
    limit_row.set_numeric(true);
    let limit_ctx = ctx.clone();
    limit_adjustment.connect_value_changed(move |adj| {
        let ctx = limit_ctx.clone();
        let value = adj.value() as usize;
        glib::spawn_future_local(async move {
            if let Err(err) = history_cmds::update_history_limit(&ctx, value).await {
                ctx.report_error("update_history_limit", err);
            }
        });
    });
    retention_group.add(&limit_row);
    page.add(&retention_group);

    // Search entry handler
    search_entry.connect_search_changed(move |entry| {
        HISTORY_SEARCH.with(|s| *s.borrow_mut() = entry.text().to_string());
        apply_history_filter();
    });

    // Initial render + live refresh on history events.
    // `rows` owns the group rows so rebuilds remove exactly what they added.
    let rows = Arc::new(Mutex::new(crate::ui::pages::PageGroup::new()));
    let ctx_render = ctx.clone();
    let rows_render = rows.clone();
    refresh_entries(&ctx_render, &entries_group, &rows_render);

    let group_for_events = glib::SendWeakRef::from(entries_group.downgrade());
    let rows_for_events = rows.clone();
    let bus = ctx.bus.clone();

    bus.subscribe(move |event| {
        let ctx = ctx_render.clone();
        let group = group_for_events.clone();
        let rows = rows_for_events.clone();

        glib::MainContext::default().invoke(move || {
            let weak = group.into_weak_ref();
            let Some(group) = weak.upgrade() else {
                return;
            };
            if matches!(event, AppEvent::HistoryUpdated(_)) {
                refresh_entries(&ctx, &group, &rows);
            }
        });
    });

    page.upcast::<gtk4::Widget>()
}

fn refresh_entries(
    ctx: &AppContext,
    group: &libadwaita::PreferencesGroup,
    rows: &Arc<Mutex<crate::ui::pages::PageGroup>>,
) {
    rows.lock().unwrap_or_else(|e| e.into_inner()).clear(group);
    HISTORY_TRACKED.with(|t| t.borrow_mut().clear());

    let ctx = ctx.clone();
    let rows = rows.clone();
    let group_weak = glib::SendWeakRef::from(group.downgrade());
    crate::runtime::spawn(async move {
        let result = history_cmds::get_history_entries(&ctx, None, Some(50)).await;
        let ctx = ctx.clone();
        let group_weak = group_weak.clone();
        let rows = rows.clone();
        glib::MainContext::default().invoke(move || {
            let Some(group) = group_weak.into_weak_ref().upgrade() else {
                return;
            };
            match result {
                Ok(paginated) => {
                    if paginated.entries.is_empty() {
                        let empty_row = libadwaita::ActionRow::new();
                        empty_row.set_title("No history entries yet");
                        empty_row.set_subtitle(
                            "Voice dictations, meetings, and AI transformations will appear here.",
                        );
                        let empty_icon =
                            gtk4::Image::from_icon_name("document-open-recent-symbolic");
                        empty_row.add_prefix(&empty_icon);
                        empty_row.set_activatable(false);
                        rows.lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .add(&group, &empty_row);
                        return;
                    }

                    let mut new_tracked = Vec::new();
                    for entry in paginated.entries {
                        let row = libadwaita::ExpanderRow::new();
                        row.set_widget_name(&entry.id.to_string());

                        // Category Symbolic Icon Prefix & Kind Title
                        let (icon_name, kind_title) = match entry.entry_kind.as_str() {
                            "meeting" => ("system-users-symbolic", "Meeting Minutes"),
                            "post_process" => ("starred-symbolic", "AI Post-Processed"),
                            "file" => ("document-open-symbolic", "Media File"),
                            _ => ("audio-input-microphone-symbolic", "Voice Dictation"),
                        };
                        let kind_icon = gtk4::Image::from_icon_name(icon_name);
                        kind_icon.set_pixel_size(16);
                        row.add_prefix(&kind_icon);

                        row.set_title(&glib::markup_escape_text(&entry.title));
                        let ts = chrono::DateTime::from_timestamp(entry.timestamp, 0)
                            .map(|t| {
                                let local = t.with_timezone(&chrono::Local);
                                local.format("%Y-%m-%d %H:%M").to_string()
                            })
                            .unwrap_or_else(|| "unknown time".to_string());
                        row.set_subtitle(&format!("{} • {}", ts, kind_title));
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

                        // Content rows inside the expanded body
                        if has_distinct_post_process {
                            let pp_text = entry.post_processed_text.as_deref().unwrap_or_default();
                            let processed_row = libadwaita::ActionRow::new();
                            let section_title = if entry.entry_kind == "meeting" {
                                "Meeting Minutes &amp; Summary"
                            } else {
                                "Processed Transcript"
                            };
                            processed_row.set_title(section_title);
                            processed_row.set_subtitle(&glib::markup_escape_text(pp_text));
                            processed_row.set_subtitle_lines(0);
                            processed_row.set_activatable(false);
                            let copy_pp_btn = create_copy_button(&ctx, pp_text);
                            processed_row.add_suffix(&copy_pp_btn);
                            row.add_row(&processed_row);

                            if !entry.transcription_text.trim().is_empty() {
                                let raw_row = libadwaita::ActionRow::new();
                                raw_row.set_title("Original Audio Transcript");
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
                            let label = if entry.entry_kind == "file" {
                                "File Transcript"
                            } else {
                                "Transcript"
                            };
                            text_row.set_title(label);
                            text_row.set_subtitle(&glib::markup_escape_text(primary_text));
                            text_row.set_subtitle_lines(0);
                            text_row.set_activatable(false);
                            let copy_text_btn = create_copy_button(&ctx, primary_text);
                            text_row.add_suffix(&copy_text_btn);
                            row.add_row(&text_row);
                        }

                        // Child Management Row: Keep / Star toggle & Retry action
                        let manage_row = libadwaita::ActionRow::new();
                        manage_row.set_title("Keep this entry permanently");
                        manage_row.set_subtitle(
                            "Prevent automatic trimming when retention limit is reached",
                        );

                        let keep_switch = gtk4::Switch::new();
                        keep_switch.set_active(entry.saved);
                        keep_switch.set_valign(gtk4::Align::Center);
                        let save_ctx = ctx.clone();
                        let id = entry.id;
                        keep_switch.connect_active_notify(move |_r| {
                            let ctx = save_ctx.clone();
                            let id = id;
                            crate::runtime::spawn(async move {
                                if let Err(err) =
                                    history_cmds::toggle_history_entry_saved(&ctx, id).await
                                {
                                    ctx.report_error("toggle_history_entry_saved", err);
                                }
                            });
                        });
                        manage_row.add_suffix(&keep_switch);

                        let retry_btn = gtk4::Button::from_icon_name("view-refresh-symbolic");
                        retry_btn.set_tooltip_text(Some("Retry transcription"));
                        retry_btn.set_valign(gtk4::Align::Center);
                        retry_btn.add_css_class("flat");
                        let retry_ctx = ctx.clone();
                        let retry_id = entry.id;
                        retry_btn.connect_clicked(move |_| {
                            let ctx = retry_ctx.clone();
                            let id = retry_id;
                            crate::runtime::spawn(async move {
                                let _ =
                                    history_cmds::retry_history_entry_transcription(&ctx, id).await;
                            });
                        });
                        manage_row.add_suffix(&retry_btn);
                        row.add_row(&manage_row);

                        // Primary Action 1: Copy button for the expander row header
                        let copy_button = create_copy_button(&ctx, primary_text);
                        row.add_suffix(&copy_button);

                        // Primary Action 2: Audio Playback (if audio file exists)
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
                        play_button.add_css_class("flat");

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
                        row.add_suffix(&play_button);

                        // Primary Action 3: Delete button in header row
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
                                if let Err(err) = history_cmds::delete_history_entry(&ctx, id).await
                                {
                                    ctx.report_error("delete_history_entry", err);
                                }
                            });
                        });
                        row.add_suffix(&delete_button);

                        rows.lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .add(&group, &row);
                        new_tracked.push((entry, row));
                    }

                    HISTORY_TRACKED.with(|t| *t.borrow_mut() = new_tracked);
                    apply_history_filter();
                }
                Err(e) => {
                    let row = libadwaita::ActionRow::new();
                    row.set_title("Failed to load history");
                    row.set_subtitle(&e.to_string());
                    rows.lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .add(&group, &row);
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
