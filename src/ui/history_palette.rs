//! Quick History Overlay: a floating, keyboard-navigable palette
//! for searching, viewing, playing, and copying recent voice transcriptions,
//! AI post-processing outputs, meeting minutes, and file transcriptions.

use crate::commands::history as history_cmds;
use crate::context::{AppContext, AppEvent};
use crate::managers::history::{HistoryEntry, HistoryUpdatePayload};
use gdk4::prelude::*;
use gtk4::prelude::*;
use libadwaita::prelude::*;
use log::info;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

static HISTORY_PALETTE_WINDOW: LazyLock<Mutex<Option<glib::SendWeakRef<libadwaita::Window>>>> =
    LazyLock::new(|| Mutex::new(None));
static LAST_HISTORY_TOGGLE: LazyLock<Mutex<Option<std::time::Instant>>> =
    LazyLock::new(|| Mutex::new(None));

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
enum CategoryFilter {
    #[default]
    All,
    Voice,
    Meeting,
    PostProcess,
    File,
}

#[derive(Clone, Default)]
struct PaletteState {
    entries: Vec<(HistoryEntry, gtk4::ListBoxRow, String)>,
    current_filter: CategoryFilter,
}

fn format_entry_timestamp(timestamp: i64) -> String {
    let now = chrono::Local::now().timestamp();
    let diff = now - timestamp;
    if diff < 60 {
        "Just now".to_string()
    } else if diff < 3600 {
        format!("{}m ago", diff / 60)
    } else if diff < 86400 {
        format!("{}h ago", diff / 3600)
    } else {
        chrono::DateTime::from_timestamp(timestamp, 0)
            .map(|t| {
                let local = t.with_timezone(&chrono::Local);
                local.format("%b %e, %H:%M").to_string()
            })
            .unwrap_or_else(|| "recent".to_string())
    }
}

fn apply_palette_filter(
    state: &Rc<RefCell<PaletteState>>,
    stack: &gtk4::Stack,
    list: &gtk4::ListBox,
    raw_query: &str,
) {
    let query = raw_query.trim().to_lowercase();
    let st = state.borrow();
    let total_entries = st.entries.len();
    let filter = st.current_filter;

    let mut matched_count = 0;
    let mut first_visible: Option<gtk4::ListBoxRow> = None;

    for (entry, row, num) in &st.entries {
        let matches_category = match filter {
            CategoryFilter::All => true,
            CategoryFilter::Voice => entry.entry_kind == "transcription",
            CategoryFilter::Meeting => entry.entry_kind == "meeting",
            CategoryFilter::PostProcess => entry.entry_kind == "post_process",
            CategoryFilter::File => entry.entry_kind == "file",
        };

        let matches_search = if query.is_empty() {
            true
        } else {
            entry.title.to_lowercase().contains(&query)
                || entry.transcription_text.to_lowercase().contains(&query)
                || entry
                    .post_processed_text
                    .as_deref()
                    .map(|pp| pp.to_lowercase().contains(&query))
                    .unwrap_or(false)
                || num == &query
        };

        let visible = matches_category && matches_search;
        row.set_visible(visible);

        if visible {
            matched_count += 1;
            if first_visible.is_none() {
                first_visible = Some(row.clone());
            }
        }
    }

    if total_entries == 0 {
        stack.set_visible_child_name("empty");
    } else if matched_count == 0 {
        stack.set_visible_child_name("no_matches");
    } else {
        stack.set_visible_child_name("entries");
        if let Some(first) = first_visible {
            list.select_row(Some(&first));
        }
    }
}

/// Toggle display of the Quick History Overlay.
pub fn toggle_history_palette(ctx: &AppContext) {
    let now = std::time::Instant::now();
    if let Ok(mut last) = LAST_HISTORY_TOGGLE.lock() {
        if let Some(prev) = *last {
            if now.duration_since(prev) < Duration::from_millis(300) {
                return;
            }
        }
        *last = Some(now);
    }

    let ctx = ctx.clone();
    glib::MainContext::default().invoke(move || {
        let existing_win = {
            let mut guard = match HISTORY_PALETTE_WINDOW.lock() {
                Ok(g) => g,
                Err(poisoned) => poisoned.into_inner(),
            };
            guard.take().and_then(|w| w.into_weak_ref().upgrade())
        };

        if let Some(win) = existing_win {
            if !win.in_destruction() {
                if win.is_visible() {
                    win.close();
                } else {
                    win.present();
                    if let Ok(mut guard) = HISTORY_PALETTE_WINDOW.lock() {
                        *guard = Some(glib::SendWeakRef::from(win.downgrade()));
                    }
                }
                return;
            }
        }

        build_and_present_palette(&ctx);
    });
}

/// Show the Quick History Overlay centered on screen.
pub fn show_history_palette(ctx: &AppContext) {
    toggle_history_palette(ctx);
}

fn build_and_present_palette(ctx: &AppContext) {
    let window = libadwaita::Window::new();
    window.set_title(Some("Transcription History"));
    window.set_default_size(680, 520);
    window.set_modal(true);
    window.set_resizable(true);
    window.set_deletable(true);
    window.add_css_class("dialog");

    {
        let mut guard = match HISTORY_PALETTE_WINDOW.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        *guard = Some(glib::SendWeakRef::from(window.downgrade()));
    }

    let toast_overlay = libadwaita::ToastOverlay::new();
    let main_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    toast_overlay.set_child(Some(&main_box));
    window.set_content(Some(&toast_overlay));

    let state = Rc::new(RefCell::new(PaletteState::default()));

    // ========================================================================
    // Header Bar
    // ========================================================================
    let header_bar = libadwaita::HeaderBar::new();
    header_bar.set_show_end_title_buttons(true);

    let window_title = libadwaita::WindowTitle::new(
        "Transcription History",
        "Recent dictations, meetings & AI outputs",
    );
    header_bar.set_title_widget(Some(&window_title));
    main_box.append(&header_bar);

    // ========================================================================
    // Search and Filter Bar
    // ========================================================================
    let controls_box = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
    controls_box.set_margin_start(16);
    controls_box.set_margin_end(16);
    controls_box.set_margin_top(8);
    controls_box.set_margin_bottom(8);

    let search_entry = gtk4::SearchEntry::new();
    search_entry.set_placeholder_text(Some("Search history, or press 1-9 to copy…"));
    controls_box.append(&search_entry);

    // Category Filter Chips (Segmented control)
    let filter_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    filter_box.add_css_class("linked");
    filter_box.set_halign(gtk4::Align::Center);

    let filter_buttons = [
        (CategoryFilter::All, "All"),
        (CategoryFilter::Voice, "Voice"),
        (CategoryFilter::Meeting, "Meetings"),
        (CategoryFilter::PostProcess, "AI Polish"),
        (CategoryFilter::File, "Files"),
    ];

    let mut first_btn: Option<gtk4::ToggleButton> = None;
    let mut button_widgets = Vec::new();

    for (cat, label) in filter_buttons {
        let btn = gtk4::ToggleButton::with_label(label);
        if let Some(ref first) = first_btn {
            btn.set_group(Some(first));
        } else {
            btn.set_active(true);
            first_btn = Some(btn.clone());
        }
        filter_box.append(&btn);
        button_widgets.push((cat, btn));
    }
    controls_box.append(&filter_box);
    main_box.append(&controls_box);

    // ========================================================================
    // Multi-State View (Stack)
    // ========================================================================
    let stack = gtk4::Stack::new();
    stack.set_transition_type(gtk4::StackTransitionType::Crossfade);
    stack.set_transition_duration(200);
    stack.set_vexpand(true);
    stack.set_hexpand(true);

    // State 1: Entries List
    let scrolled = gtk4::ScrolledWindow::new();
    scrolled.set_vexpand(true);
    scrolled.set_hexpand(true);

    let list_box = gtk4::ListBox::new();
    list_box.set_selection_mode(gtk4::SelectionMode::Single);
    list_box.add_css_class("boxed-list");
    list_box.set_margin_start(16);
    list_box.set_margin_end(16);
    list_box.set_margin_bottom(16);

    scrolled.set_child(Some(&list_box));
    stack.add_named(&scrolled, Some("entries"));

    // State 2: No Search Matches
    let no_matches_page = libadwaita::StatusPage::new();
    no_matches_page.set_icon_name(Some("system-search-symbolic"));
    no_matches_page.set_title("No Matching Transcriptions");
    no_matches_page.set_description(Some("Try different keywords or clear the category filter."));
    stack.add_named(&no_matches_page, Some("no_matches"));

    // State 3: Empty History
    let empty_page = libadwaita::StatusPage::new();
    empty_page.set_icon_name(Some("document-open-recent-symbolic"));
    empty_page.set_title("No Transcription History");
    empty_page.set_description(Some(
        "Voice dictations, meeting minutes, and AI transformations will appear here.",
    ));
    stack.add_named(&empty_page, Some("empty"));

    main_box.append(&stack);

    // ========================================================================
    // Populate & Reload Entries
    // ========================================================================
    let reload_entries = {
        let ctx = ctx.clone();
        let list_box = list_box.clone();
        let window_title = window_title.clone();
        let search_entry = search_entry.clone();
        let stack = stack.clone();
        let toast = toast_overlay.clone();
        let state = state.clone();
        let window_weak = glib::SendWeakRef::from(window.downgrade());

        Rc::new(move || {
            let ctx = ctx.clone();
            let list_box = list_box.clone();
            let window_title = window_title.clone();
            let search_entry = search_entry.clone();
            let stack = stack.clone();
            let toast = toast.clone();
            let state = state.clone();
            let win_weak_outer = window_weak.clone();

            glib::MainContext::default().spawn_local(async move {
                let res = history_cmds::get_history_entries(&ctx, None, Some(50)).await;
                while let Some(child) = list_box.first_child() {
                    list_box.remove(&child);
                }

                let Ok(paginated) = res else {
                    return;
                };

                let total_count = paginated.entries.len();
                let count_str = if total_count == 1 {
                    "1 entry".to_string()
                } else {
                    format!("{total_count} entries")
                };
                window_title.set_subtitle(&format!("{count_str} • Press 1-9 or ↵ to copy"));

                let mut new_entries = Vec::new();

                for (i, entry) in paginated.entries.into_iter().enumerate() {
                    let row = libadwaita::ActionRow::new();

                    // Shortcut number badge 1-9
                    let shortcut_num = if i < 9 {
                        format!("{}", i + 1)
                    } else {
                        String::new()
                    };

                    let prefix_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
                    prefix_box.set_valign(gtk4::Align::Center);

                    if !shortcut_num.is_empty() {
                        let num_badge = gtk4::Label::new(Some(&shortcut_num));
                        num_badge.add_css_class("caption");
                        num_badge.add_css_class("dim-label");
                        num_badge.set_width_chars(2);
                        num_badge.set_xalign(0.5);
                        prefix_box.append(&num_badge);
                    }

                    // Kind Icon
                    let (icon_name, kind_title) = match entry.entry_kind.as_str() {
                        "meeting" => ("system-users-symbolic", "Meeting"),
                        "post_process" => ("starred-symbolic", "AI Polish"),
                        "file" => ("folder-symbolic", "File"),
                        _ => ("audio-input-microphone-symbolic", "Voice"),
                    };
                    let kind_icon = gtk4::Image::from_icon_name(icon_name);
                    kind_icon.set_pixel_size(16);
                    kind_icon.add_css_class("dim-label");
                    kind_icon.set_tooltip_text(Some(kind_title));
                    prefix_box.append(&kind_icon);

                    row.add_prefix(&prefix_box);

                    // Title with relative timestamp
                    let time_str = format_entry_timestamp(entry.timestamp);
                    row.set_use_markup(false);
                    row.set_title(&format!("{} • {}", entry.title, time_str));

                    // Primary Text Preview
                    let primary_text = entry
                        .post_processed_text
                        .as_deref()
                        .filter(|s| !s.trim().is_empty())
                        .unwrap_or(&entry.transcription_text);

                    let preview = if primary_text.len() > 100 {
                        format!("{}…", &primary_text[..100])
                    } else {
                        primary_text.to_string()
                    };
                    row.set_subtitle(&glib::markup_escape_text(&preview));
                    row.set_subtitle_lines(2);
                    row.set_activatable(true);

                    let text_to_copy = primary_text.to_string();

                    // Suffix button: Audio playback (if exists)
                    let audio_path = ctx.history.get_audio_file_path(&entry.file_name);
                    if audio_path.exists() {
                        let is_playing = history_cmds::is_playing_history_audio(entry.id);
                        let play_icon = if is_playing {
                            "media-playback-stop-symbolic"
                        } else {
                            "media-playback-start-symbolic"
                        };
                        let play_btn = gtk4::Button::from_icon_name(play_icon);
                        play_btn.set_tooltip_text(Some("Play audio recording"));
                        play_btn.set_valign(gtk4::Align::Center);
                        play_btn.add_css_class("flat");

                        let play_ctx = ctx.clone();
                        let play_id = entry.id;
                        let play_btn_click = play_btn.clone();
                        play_btn.connect_clicked(move |_| {
                            let ctx = play_ctx.clone();
                            let btn = play_btn_click.clone();
                            glib::MainContext::default().spawn_local(async move {
                                if let Ok(playing) =
                                    history_cmds::toggle_play_history_audio(&ctx, play_id).await
                                {
                                    btn.set_icon_name(if playing {
                                        "media-playback-stop-symbolic"
                                    } else {
                                        "media-playback-start-symbolic"
                                    });
                                }
                            });
                        });
                        row.add_suffix(&play_btn);
                    }

                    // Suffix button: Save to Quick Notes
                    let save_note_btn = gtk4::Button::from_icon_name("text-editor-symbolic");
                    save_note_btn.set_tooltip_text(Some("Save to Quick Notes"));
                    save_note_btn.set_valign(gtk4::Align::Center);
                    save_note_btn.add_css_class("flat");

                    let ctx_note = ctx.clone();
                    let note_title = format!("From History: {}", entry.title);
                    let note_text = text_to_copy.clone();
                    let toast_note = toast.clone();
                    save_note_btn.connect_clicked(move |_| {
                        match ctx_note.history.save_note(
                            note_title.clone(),
                            note_text.clone(),
                            Some("history,transcription".to_string()),
                        ) {
                            Ok(_) => {
                                toast_note
                                    .add_toast(libadwaita::Toast::new("Saved to Quick Notes!"));
                            }
                            Err(e) => {
                                toast_note
                                    .add_toast(libadwaita::Toast::new(&format!("Error: {e}")));
                            }
                        }
                    });
                    row.add_suffix(&save_note_btn);

                    // Suffix button: Copy to clipboard
                    let copy_btn = gtk4::Button::from_icon_name("edit-copy-symbolic");
                    copy_btn.set_tooltip_text(Some("Copy to clipboard"));
                    copy_btn.set_valign(gtk4::Align::Center);
                    copy_btn.add_css_class("flat");

                    let text_for_copy = text_to_copy.clone();
                    let ctx_copy = ctx.clone();
                    let win_weak_copy = win_weak_outer.clone();
                    let copy_btn_click = copy_btn.clone();

                    copy_btn.connect_clicked(move |_| {
                        let _ = crate::clipboard::write_clipboard_text(&ctx_copy, &text_for_copy);
                        if let Some(display) = gdk4::Display::default() {
                            display.clipboard().set_text(&text_for_copy);
                        }
                        copy_btn_click.set_icon_name("object-select-symbolic");
                        let win_weak_close = win_weak_copy.clone();
                        glib::timeout_add_local_once(
                            std::time::Duration::from_millis(150),
                            move || {
                                if let Some(win) = win_weak_close.into_weak_ref().upgrade() {
                                    win.close();
                                }
                            },
                        );
                    });
                    row.add_suffix(&copy_btn);

                    // Suffix button: Delete entry
                    let del_btn = gtk4::Button::from_icon_name("user-trash-symbolic");
                    del_btn.set_tooltip_text(Some("Delete from history"));
                    del_btn.set_valign(gtk4::Align::Center);
                    del_btn.add_css_class("flat");

                    let list_row = gtk4::ListBoxRow::new();
                    list_row.set_child(Some(&row));
                    list_box.append(&list_row);

                    let ctx_del = ctx.clone();
                    let entry_id = entry.id;
                    let row_del = list_row.clone();
                    let list_del = list_box.clone();
                    let toast_del = toast.clone();
                    let state_del = state.clone();

                    del_btn.connect_clicked(move |_| {
                        let ctx = ctx_del.clone();
                        let row = row_del.clone();
                        let list = list_del.clone();
                        let toast = toast_del.clone();
                        let state = state_del.clone();

                        glib::MainContext::default().spawn_local(async move {
                            if history_cmds::delete_history_entry(&ctx, entry_id)
                                .await
                                .is_ok()
                            {
                                list.remove(&row);
                                state
                                    .borrow_mut()
                                    .entries
                                    .retain(|(e, _, _)| e.id != entry_id);
                                toast.add_toast(libadwaita::Toast::new("Entry deleted"));
                            }
                        });
                    });
                    row.add_suffix(&del_btn);

                    new_entries.push((entry, list_row, shortcut_num));
                }

                state.borrow_mut().entries = new_entries;
                apply_palette_filter(&state, &stack, &list_box, &search_entry.text());
            });
        })
    };

    // Trigger initial load
    reload_entries();

    // ========================================================================
    // Category Chips Wiring
    // ========================================================================
    for (cat, btn) in button_widgets {
        let btn_search = search_entry.clone();
        let state_filter = state.clone();
        let stack_filter = stack.clone();
        let list_filter = list_box.clone();

        btn.connect_toggled(move |b| {
            if b.is_active() {
                state_filter.borrow_mut().current_filter = cat;
                apply_palette_filter(
                    &state_filter,
                    &stack_filter,
                    &list_filter,
                    &btn_search.text(),
                );
            }
        });
    }

    // ========================================================================
    // Search Entry Changed Wiring
    // ========================================================================
    let state_search = state.clone();
    let stack_search = stack.clone();
    let list_search = list_box.clone();
    search_entry.connect_search_changed(move |entry| {
        apply_palette_filter(&state_search, &stack_search, &list_search, &entry.text());
    });

    // ========================================================================
    // Row Activated (Enter or click on row)
    // ========================================================================
    let act_ctx = ctx.clone();
    let act_win_weak = glib::SendWeakRef::from(window.downgrade());
    let state_activate = state.clone();

    list_box.connect_row_activated(move |_list, row| {
        let text_to_copy = state_activate
            .borrow()
            .entries
            .iter()
            .find(|(_, r, _)| r == row)
            .map(|(entry, _, _)| {
                entry
                    .post_processed_text
                    .as_deref()
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or(&entry.transcription_text)
                    .to_string()
            });

        if let Some(text) = text_to_copy {
            let _ = crate::clipboard::write_clipboard_text(&act_ctx, &text);
            if let Some(display) = gdk4::Display::default() {
                display.clipboard().set_text(&text);
            }
            if let Some(win) = act_win_weak.clone().into_weak_ref().upgrade() {
                win.close();
            }
        }
    });

    // ========================================================================
    // Keyboard Navigation & Shortcuts
    // ========================================================================
    let key_controller = gtk4::EventControllerKey::new();
    let key_ctx = ctx.clone();
    let key_win_weak = glib::SendWeakRef::from(window.downgrade());
    let key_search_entry = search_entry.clone();
    let state_key = state.clone();

    key_controller.connect_key_pressed(move |_ctrl, keyval, _keycode, state_mod| {
        if keyval == gdk4::Key::Escape {
            if let Some(win) = key_win_weak.clone().into_weak_ref().upgrade() {
                win.close();
                return glib::Propagation::Stop;
            }
        }

        let is_alt = state_mod.contains(gdk4::ModifierType::ALT_MASK);
        let is_empty_query = key_search_entry.text().trim().is_empty();

        // Check 1-9 shortcuts ONLY when Alt is held OR search entry is empty
        if is_alt || is_empty_query {
            if let Some(digit_char) = keyval.to_unicode() {
                if ('1'..='9').contains(&digit_char) {
                    let digit_str = digit_char.to_string();
                    let text_to_copy = state_key
                        .borrow()
                        .entries
                        .iter()
                        .find(|(_, r, num)| num == &digit_str && r.is_visible())
                        .map(|(entry, _, _)| {
                            entry
                                .post_processed_text
                                .as_deref()
                                .filter(|s| !s.trim().is_empty())
                                .unwrap_or(&entry.transcription_text)
                                .to_string()
                        });

                    if let Some(text) = text_to_copy {
                        let _ = crate::clipboard::write_clipboard_text(&key_ctx, &text);
                        if let Some(display) = gdk4::Display::default() {
                            display.clipboard().set_text(&text);
                        }
                        if let Some(win) = key_win_weak.clone().into_weak_ref().upgrade() {
                            win.close();
                            return glib::Propagation::Stop;
                        }
                    }
                }
            }
        }

        glib::Propagation::Proceed
    });
    window.add_controller(key_controller);

    window.connect_destroy(|_| {
        let mut guard = match HISTORY_PALETTE_WINDOW.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        *guard = None;
    });

    // Event bus listener for real-time history updates while open.
    // The subscription is removed on destroy so repeated open/close cycles
    // cannot accumulate callbacks on the bus.
    let trigger_reload_btn = gtk4::Button::new();
    let reload_fn = reload_entries.clone();
    trigger_reload_btn.connect_clicked(move |_| {
        reload_fn();
    });

    let trigger_weak = glib::SendWeakRef::from(trigger_reload_btn.downgrade());
    let bus = ctx.bus.clone();
    let subscription = bus.subscribe(move |event| {
        if matches!(
            event,
            AppEvent::HistoryUpdated(
                HistoryUpdatePayload::Added { .. } | HistoryUpdatePayload::Updated { .. }
            )
        ) {
            let trigger = trigger_weak.clone();
            glib::MainContext::default().invoke(move || {
                if let Some(btn) = trigger.into_weak_ref().upgrade() {
                    btn.emit_clicked();
                }
            });
        }
    });
    let subscription = std::sync::Arc::new(std::sync::Mutex::new(Some(subscription)));
    window.connect_destroy(move |_| {
        if let Ok(mut guard) = subscription.lock() {
            if let Some(sub) = guard.take() {
                sub.unsubscribe();
            }
        }
    });

    search_entry.grab_focus();
    info!("Quick History Overlay presented");
    window.present();
}
