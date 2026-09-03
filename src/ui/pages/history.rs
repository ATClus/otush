//! History settings page: global application history covering standard
//! voice transcriptions, meeting minutes, AI prompt processings, and file transcriptions.

use crate::commands::history as history_cmds;
use crate::context::{AppContext, AppEvent};
use crate::managers::history::HistoryEntry;
use gdk4::prelude::*;
use gtk4::prelude::*;
use libadwaita::prelude::*;
use std::cell::RefCell;

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

    // --- 1. Retention group ---
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

    // --- 2. Filter & Search Controls ---
    let filter_group = libadwaita::PreferencesGroup::new();
    filter_group.set_title("Filter &amp; Search");

    let controls_box = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
    controls_box.set_margin_top(4);
    controls_box.set_margin_bottom(8);

    let search_entry = gtk4::SearchEntry::new();
    search_entry.set_placeholder_text(Some("Search transcripts, meeting minutes, or AI text…"));
    controls_box.append(&search_entry);

    let filter_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    filter_row.set_halign(gtk4::Align::Center);

    let filter_categories = [
        (CategoryFilter::All, "All"),
        (CategoryFilter::Voice, "🎙️ Voice"),
        (CategoryFilter::Meeting, "👥 Meetings"),
        (CategoryFilter::PostProcess, "✨ AI Polish"),
        (CategoryFilter::File, "📁 Files"),
    ];

    let mut cat_buttons = Vec::new();
    for (cat, label) in filter_categories {
        let btn = gtk4::Button::with_label(label);
        btn.add_css_class("flat");
        if cat == CategoryFilter::All {
            btn.add_css_class("suggested-action");
        }
        filter_row.append(&btn);
        cat_buttons.push((cat, btn));
    }
    controls_box.append(&filter_row);

    // Open Overlay Quick Action Row
    let overlay_action_row = libadwaita::ActionRow::new();
    overlay_action_row.set_title("Quick History Overlay");
    overlay_action_row
        .set_subtitle("Open the floating quick-access history palette (Shortcut: Ctrl+Alt+H)");
    overlay_action_row.set_activatable(true);

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

    // --- 3. Entries group ---
    let entries_group = libadwaita::PreferencesGroup::new();
    entries_group.set_widget_name("history-list");
    entries_group.set_title("Global Activity &amp; Records");
    entries_group.set_description(Some(
        "All voice recordings, meeting notes, AI transformations, and media transcripts.",
    ));
    page.add(&entries_group);

    // Category button click handlers
    for (cat, btn) in cat_buttons.clone() {
        let cat_buttons = cat_buttons.clone();
        btn.connect_clicked(move |_| {
            HISTORY_FILTER.with(|f| *f.borrow_mut() = cat);
            for (c, b) in &cat_buttons {
                if *c == cat {
                    b.add_css_class("suggested-action");
                } else {
                    b.remove_css_class("suggested-action");
                }
            }
            apply_history_filter();
        });
    }

    // Search entry handler
    search_entry.connect_search_changed(move |entry| {
        HISTORY_SEARCH.with(|s| *s.borrow_mut() = entry.text().to_string());
        apply_history_filter();
    });

    // Initial render + live refresh on history events
    let ctx_render = ctx.clone();
    refresh_entries(&ctx_render, &entries_group);

    let group_for_events = glib::SendWeakRef::from(entries_group.downgrade());
    let bus = ctx.bus.clone();

    bus.subscribe(move |event| {
        let ctx = ctx_render.clone();
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
    HISTORY_TRACKED.with(|t| t.borrow_mut().clear());

    let ctx = ctx.clone();
    let group_weak = glib::SendWeakRef::from(group.downgrade());
    crate::runtime::spawn(async move {
        let result = history_cmds::get_history_entries(&ctx, None, Some(50)).await;
        let ctx = ctx.clone();
        let group_weak = group_weak.clone();
        glib::MainContext::default().invoke(move || {
            let Some(group) = group_weak.into_weak_ref().upgrade() else {
                return;
            };
            match result {
                Ok(paginated) => {
                    let mut new_tracked = Vec::new();
                    for entry in paginated.entries {
                        let row = libadwaita::ExpanderRow::new();
                        row.set_widget_name(&entry.id.to_string());

                        // Category Badge
                        let (kind_label, badge_style) = match entry.entry_kind.as_str() {
                            "meeting" => ("👥 Meeting", "accent"),
                            "post_process" => ("✨ AI Polish", "accent"),
                            "file" => ("📁 File", "dim-label"),
                            _ => ("🎙️ Voice", "accent"),
                        };
                        let kind_badge = gtk4::Label::new(Some(kind_label));
                        kind_badge.add_css_class("caption");
                        kind_badge.add_css_class(badge_style);
                        row.add_prefix(&kind_badge);

                        row.set_title(&glib::markup_escape_text(&entry.title));
                        let ts = chrono::DateTime::from_timestamp(entry.timestamp, 0)
                            .map(|t| {
                                let local = t.with_timezone(&chrono::Local);
                                local.format("%Y-%m-%d %H:%M").to_string()
                            })
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
                            let section_title = if entry.entry_kind == "meeting" {
                                "Meeting Minutes & Summary"
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

                        // Saved toggle
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

                        // Copy button for the expander row header
                        let copy_button = create_copy_button(&ctx, primary_text);
                        row.add_suffix(&copy_button);

                        // Audio Playback
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
                        new_tracked.push((entry, row));
                    }

                    HISTORY_TRACKED.with(|t| *t.borrow_mut() = new_tracked);
                    apply_history_filter();
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
