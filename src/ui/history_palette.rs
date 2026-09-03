//! Quick History Overlay: a floating, keyboard-navigable palette
//! for searching, viewing, and copying recent voice transcriptions,
//! AI post-processing outputs, meeting minutes, and file transcriptions.

use crate::commands::history as history_cmds;
use crate::context::{AppContext, AppEvent};
use crate::managers::history::HistoryEntry;
use gdk4::prelude::*;
use gtk4::prelude::*;
use libadwaita::prelude::*;
use log::info;
use std::cell::RefCell;
use std::sync::{LazyLock, Mutex};

static HISTORY_PALETTE_WINDOW: LazyLock<Mutex<Option<glib::SendWeakRef<libadwaita::Window>>>> =
    LazyLock::new(|| Mutex::new(None));

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CategoryFilter {
    All,
    Voice,
    Meeting,
    PostProcess,
    File,
}

thread_local! {
    static PALETTE_ENTRIES: RefCell<Vec<(HistoryEntry, gtk4::ListBoxRow, String)>> =
        const { RefCell::new(Vec::new()) };
    static PALETTE_FILTER: RefCell<CategoryFilter> = const { RefCell::new(CategoryFilter::All) };
}

/// Toggle display of the Quick History Overlay.
pub fn toggle_history_palette(ctx: &AppContext) {
    let ctx = ctx.clone();
    glib::MainContext::default().invoke(move || {
        if let Ok(guard) = HISTORY_PALETTE_WINDOW.lock() {
            if let Some(ref weak) = *guard {
                if let Some(win) = weak.clone().into_weak_ref().upgrade() {
                    if win.is_visible() {
                        win.close();
                        return;
                    }
                }
            }
        }
        show_history_palette(&ctx);
    });
}

/// Show the Quick History Overlay centered on screen.
pub fn show_history_palette(ctx: &AppContext) {
    let ctx = ctx.clone();
    glib::MainContext::default().invoke(move || {
        build_and_present_palette(&ctx);
    });
}

fn build_and_present_palette(ctx: &AppContext) {
    // If a palette is already open, focus it
    if let Ok(guard) = HISTORY_PALETTE_WINDOW.lock() {
        if let Some(ref weak) = *guard {
            if let Some(win) = weak.clone().into_weak_ref().upgrade() {
                win.present();
                return;
            }
        }
    }

    let window = libadwaita::Window::new();
    window.set_title(Some("Transcription History"));
    window.set_default_size(600, 480);
    window.set_modal(true);
    window.set_resizable(true);
    window.set_deletable(true);
    window.add_css_class("dialog");

    let toast_overlay = libadwaita::ToastOverlay::new();
    let main_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    toast_overlay.set_child(Some(&main_box));
    window.set_content(Some(&toast_overlay));

    // Header bar
    let header_bar = libadwaita::HeaderBar::new();
    header_bar.set_show_end_title_buttons(true);
    header_bar.set_title_widget(Some(&gtk4::Label::new(Some("History & Transcriptions"))));
    main_box.append(&header_bar);

    // Search and Category Bar
    let controls_box = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
    controls_box.set_margin_start(16);
    controls_box.set_margin_end(16);
    controls_box.set_margin_top(8);
    controls_box.set_margin_bottom(8);

    let search_entry = gtk4::SearchEntry::new();
    search_entry.set_placeholder_text(Some("Search history, or press 1-9 to copy…"));
    controls_box.append(&search_entry);

    // Category Filter Chips
    let filter_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    filter_box.set_halign(gtk4::Align::Center);

    PALETTE_FILTER.with(|f| *f.borrow_mut() = CategoryFilter::All);

    let filter_buttons = [
        (CategoryFilter::All, "All"),
        (CategoryFilter::Voice, "🎙️ Voice"),
        (CategoryFilter::Meeting, "👥 Meetings"),
        (CategoryFilter::PostProcess, "✨ AI Polish"),
        (CategoryFilter::File, "📁 Files"),
    ];

    let mut button_widgets = Vec::new();

    for (cat, label) in filter_buttons {
        let btn = gtk4::Button::with_label(label);
        btn.add_css_class("flat");
        if cat == CategoryFilter::All {
            btn.add_css_class("suggested-action");
        }
        filter_box.append(&btn);
        button_widgets.push((cat, btn));
    }
    controls_box.append(&filter_box);
    main_box.append(&controls_box);

    // Scrolled list of entries
    let scrolled = gtk4::ScrolledWindow::new();
    scrolled.set_vexpand(true);
    scrolled.set_hexpand(true);
    scrolled.set_min_content_height(300);

    let list_box = gtk4::ListBox::new();
    list_box.set_selection_mode(gtk4::SelectionMode::Single);
    list_box.add_css_class("boxed-list");
    list_box.set_margin_start(16);
    list_box.set_margin_end(16);
    list_box.set_margin_bottom(16);

    scrolled.set_child(Some(&list_box));
    main_box.append(&scrolled);

    // Populate entries
    let list_box_weak = glib::SendWeakRef::from(list_box.downgrade());
    let window_weak = glib::SendWeakRef::from(window.downgrade());
    let search_entry_weak = glib::SendWeakRef::from(search_entry.downgrade());
    let ctx_populate = ctx.clone();

    let reload_entries = {
        let ctx = ctx_populate.clone();
        let list_box_weak = list_box_weak.clone();
        let window_weak = window_weak.clone();
        let search_entry_weak = search_entry_weak.clone();

        move || {
            let ctx = ctx.clone();
            let list_box_weak = list_box_weak.clone();
            let window_weak = window_weak.clone();
            let search_entry_weak = search_entry_weak.clone();

            crate::runtime::spawn(async move {
                let res = history_cmds::get_history_entries(&ctx, None, Some(40)).await;
                glib::MainContext::default().invoke(move || {
                    let Some(list) = list_box_weak.into_weak_ref().upgrade() else {
                        return;
                    };
                    while let Some(child) = list.first_child() {
                        list.remove(&child);
                    }

                    let Ok(paginated) = res else {
                        return;
                    };

                    let mut new_entries = Vec::new();

                    for (i, entry) in paginated.entries.into_iter().enumerate() {
                        let row = libadwaita::ActionRow::new();

                        // Shortcut number badge 1-9
                        let shortcut_num = if i < 9 {
                            format!("{}", i + 1)
                        } else {
                            String::new()
                        };

                        if !shortcut_num.is_empty() {
                            let num_badge = gtk4::Label::new(Some(&format!("[{}]", shortcut_num)));
                            num_badge.add_css_class("caption");
                            num_badge.add_css_class("dim-label");
                            row.add_prefix(&num_badge);
                        }

                        // Kind Badge
                        let (kind_label, badge_style) = match entry.entry_kind.as_str() {
                            "meeting" => ("👥 Meeting", "accent"),
                            "post_process" => ("✨ AI", "accent"),
                            "file" => ("📁 File", "dim-label"),
                            _ => ("🎙️ Voice", "accent"),
                        };
                        let kind_badge = gtk4::Label::new(Some(kind_label));
                        kind_badge.add_css_class("caption");
                        kind_badge.add_css_class(badge_style);
                        row.add_prefix(&kind_badge);

                        // Title with date
                        let ts = chrono::DateTime::from_timestamp(entry.timestamp, 0)
                            .map(|t| {
                                let local = t.with_timezone(&chrono::Local);
                                local.format("%b %e, %H:%M").to_string()
                            })
                            .unwrap_or_else(|| "recent".to_string());
                        row.set_title(&format!("{} — {}", entry.title, ts));

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

                        // Copy button
                        let copy_btn = gtk4::Button::from_icon_name("edit-copy-symbolic");
                        copy_btn.set_tooltip_text(Some("Copy to clipboard"));
                        copy_btn.set_valign(gtk4::Align::Center);
                        copy_btn.add_css_class("flat");

                        let text_to_copy = primary_text.to_string();
                        let ctx_copy = ctx.clone();
                        let win_weak_copy = window_weak.clone();
                        let btn_weak_copy = glib::SendWeakRef::from(copy_btn.downgrade());

                        copy_btn.connect_clicked(move |_| {
                            let _ =
                                crate::clipboard::write_clipboard_text(&ctx_copy, &text_to_copy);
                            if let Some(display) = gdk4::Display::default() {
                                display.clipboard().set_text(&text_to_copy);
                            }
                            if let Some(btn) = btn_weak_copy.clone().into_weak_ref().upgrade() {
                                btn.set_icon_name("object-select-symbolic");
                            }
                            let win_weak_close = win_weak_copy.clone();
                            glib::timeout_add_local_once(
                                std::time::Duration::from_millis(200),
                                move || {
                                    if let Some(win) = win_weak_close.into_weak_ref().upgrade() {
                                        win.close();
                                    }
                                },
                            );
                        });
                        row.add_suffix(&copy_btn);

                        // Audio playback button
                        let audio_path = ctx.history.get_audio_file_path(&entry.file_name);
                        if audio_path.exists() {
                            let is_playing = history_cmds::is_playing_history_audio(entry.id);
                            let play_icon = if is_playing {
                                "media-playback-stop-symbolic"
                            } else {
                                "media-playback-start-symbolic"
                            };
                            let play_btn = gtk4::Button::from_icon_name(play_icon);
                            play_btn.set_tooltip_text(Some("Play recording"));
                            play_btn.set_valign(gtk4::Align::Center);
                            play_btn.add_css_class("flat");

                            let play_ctx = ctx.clone();
                            let play_id = entry.id;
                            let play_weak = glib::SendWeakRef::from(play_btn.downgrade());
                            play_btn.connect_clicked(move |_| {
                                let ctx = play_ctx.clone();
                                let play_weak = play_weak.clone();
                                crate::runtime::spawn(async move {
                                    if let Ok(playing) =
                                        history_cmds::toggle_play_history_audio(&ctx, play_id).await
                                    {
                                        glib::MainContext::default().invoke(move || {
                                            if let Some(btn) = play_weak.into_weak_ref().upgrade() {
                                                btn.set_icon_name(if playing {
                                                    "media-playback-stop-symbolic"
                                                } else {
                                                    "media-playback-start-symbolic"
                                                });
                                            }
                                        });
                                    }
                                });
                            });
                            row.add_suffix(&play_btn);
                        }

                        let list_row = gtk4::ListBoxRow::new();
                        list_row.set_child(Some(&row));
                        list.append(&list_row);

                        new_entries.push((entry, list_row, shortcut_num));
                    }

                    PALETTE_ENTRIES.with(|g| *g.borrow_mut() = new_entries);

                    // Apply current filter & query
                    let query = search_entry_weak
                        .into_weak_ref()
                        .upgrade()
                        .map(|s| s.text().to_string())
                        .unwrap_or_default();

                    apply_palette_filter(&query, &list);
                });
            });
        }
    };

    // Trigger initial load
    reload_entries();

    // Wire Category buttons
    for (cat, btn) in button_widgets.clone() {
        let btn_search = search_entry.clone();
        let btn_list = list_box.clone();
        let all_btns = button_widgets.clone();

        btn.connect_clicked(move |_| {
            PALETTE_FILTER.with(|f| *f.borrow_mut() = cat);
            for (c, b) in &all_btns {
                if *c == cat {
                    b.add_css_class("suggested-action");
                } else {
                    b.remove_css_class("suggested-action");
                }
            }
            apply_palette_filter(&btn_search.text(), &btn_list);
        });
    }

    // Wire Search Entry changed
    let search_list = list_box.clone();
    search_entry.connect_search_changed(move |entry| {
        apply_palette_filter(&entry.text(), &search_list);
    });

    // Row activation (Enter or click on row)
    let act_ctx = ctx.clone();
    let act_win_weak = glib::SendWeakRef::from(window.downgrade());

    list_box.connect_row_activated(move |_list, row| {
        let text_to_copy = PALETTE_ENTRIES.with(|guard| {
            guard
                .borrow()
                .iter()
                .find(|(_, r, _)| r == row)
                .map(|(entry, _, _)| {
                    entry
                        .post_processed_text
                        .as_deref()
                        .filter(|s| !s.trim().is_empty())
                        .unwrap_or(&entry.transcription_text)
                        .to_string()
                })
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

    // Keyboard navigation (Esc to dismiss, 1-9 quick copy)
    let key_controller = gtk4::EventControllerKey::new();
    let key_ctx = ctx.clone();
    let key_win_weak = glib::SendWeakRef::from(window.downgrade());

    key_controller.connect_key_pressed(move |_ctrl, keyval, _keycode, _state| {
        if keyval == gdk4::Key::Escape {
            if let Some(win) = key_win_weak.clone().into_weak_ref().upgrade() {
                win.close();
                return glib::Propagation::Stop;
            }
        }

        // Check 1-9 shortcuts
        if let Some(digit_char) = keyval.to_unicode() {
            if ('1'..='9').contains(&digit_char) {
                let digit_str = digit_char.to_string();
                let text_to_copy = PALETTE_ENTRIES.with(|guard| {
                    guard
                        .borrow()
                        .iter()
                        .find(|(_, r, num)| num == &digit_str && r.is_visible())
                        .map(|(entry, _, _)| {
                            entry
                                .post_processed_text
                                .as_deref()
                                .filter(|s| !s.trim().is_empty())
                                .unwrap_or(&entry.transcription_text)
                                .to_string()
                        })
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

        glib::Propagation::Proceed
    });
    window.add_controller(key_controller);

    // Event bus listener for real-time history updates while open
    let bus = ctx.bus.clone();
    let reload_fn = reload_entries.clone();
    bus.subscribe(move |event| {
        if matches!(event, AppEvent::HistoryUpdated(_)) {
            let re = reload_fn.clone();
            glib::MainContext::default().invoke(move || {
                re();
            });
        }
    });

    if let Ok(mut guard) = HISTORY_PALETTE_WINDOW.lock() {
        *guard = Some(glib::SendWeakRef::from(window.downgrade()));
    }

    info!("Quick History Overlay presented");
    window.present();
}

fn apply_palette_filter(raw_query: &str, list: &gtk4::ListBox) {
    let query = raw_query.trim().to_lowercase();
    let filter = PALETTE_FILTER.with(|f| *f.borrow());

    let mut first_visible: Option<gtk4::ListBoxRow> = None;

    PALETTE_ENTRIES.with(|guard| {
        for (entry, row, num) in guard.borrow().iter() {
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

            if visible && first_visible.is_none() {
                first_visible = Some(row.clone());
            }
        }
    });

    if let Some(first) = first_visible {
        list.select_row(Some(&first));
    }
}
