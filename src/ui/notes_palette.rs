//! Quick Note & Idea Capture palette (`Ctrl+Alt+N`).
//!
//! Native GTK4 + libadwaita distraction-free quick notes overlay.
//! Features automatic debounced saving into SQLite, automatic title
//! derivation from the first line of the note, optional hashtag parsing,
//! and a collapsible sidebar complement to browse and search saved notes.

use crate::context::AppContext;
use gdk4::prelude::*;
use gtk4::prelude::*;
use libadwaita::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

static NOTE_WINDOW: LazyLock<Mutex<Option<glib::SendWeakRef<libadwaita::Window>>>> =
    LazyLock::new(|| Mutex::new(None));
static LAST_NOTE_TOGGLE: LazyLock<Mutex<Option<std::time::Instant>>> =
    LazyLock::new(|| Mutex::new(None));

/// Toggle or show the quick note capture modal.
pub fn show_notes_palette(ctx: &AppContext) {
    toggle_notes_palette(ctx);
}

/// Toggle display of the quick note capture modal.
pub fn toggle_notes_palette(ctx: &AppContext) {
    let now = std::time::Instant::now();
    if let Ok(mut last) = LAST_NOTE_TOGGLE.lock() {
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
            let mut guard = match NOTE_WINDOW.lock() {
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
                    if let Ok(mut guard) = NOTE_WINDOW.lock() {
                        *guard = Some(glib::SendWeakRef::from(win.downgrade()));
                    }
                }
                return;
            }
        }

        build_and_present_notes_palette(&ctx);
    });
}

/// Derives a clean title and optional tags from note content.
/// The first non-empty line becomes the title (trimming leading markdown `#`).
/// Any `#hashtags` found in the text are extracted into tags.
fn parse_note_content(content: &str) -> (String, Option<String>) {
    let mut lines = content.lines().map(|l| l.trim()).filter(|l| !l.is_empty());
    let title = if let Some(first) = lines.next() {
        let clean = first.trim_start_matches('#').trim();

        // If the first line is a URL, strip verbose query params for a clean title
        let display_title = if clean.starts_with("http://") || clean.starts_with("https://") {
            if let Ok(url) = reqwest::Url::parse(clean) {
                let domain = url.domain().unwrap_or("");
                let path = url.path().trim_end_matches('/');
                if !domain.is_empty() && path.len() > 1 {
                    format!("{}{}", domain, path)
                } else if !domain.is_empty() {
                    domain.to_string()
                } else {
                    clean.to_string()
                }
            } else {
                clean.to_string()
            }
        } else {
            clean.to_string()
        };

        if display_title.len() > 50 {
            format!("{}…", &display_title[..50])
        } else {
            display_title
        }
    } else {
        "Untitled Note".to_string()
    };

    let mut tags = Vec::new();
    for word in content.split_whitespace() {
        if word.starts_with('#') && word.len() > 1 {
            let tag = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '_' && c != '-');
            if !tag.is_empty() && !tags.contains(&tag.to_lowercase()) {
                tags.push(tag.to_lowercase());
            }
        }
    }

    let tags_opt = if tags.is_empty() {
        None
    } else {
        Some(tags.join(","))
    };

    (title, tags_opt)
}

/// Formats a unix timestamp into human-readable date/time.
fn format_note_timestamp(ts: i64) -> String {
    use chrono::{Local, TimeZone};
    if let Some(dt) = Local.timestamp_opt(ts, 0).single() {
        let now = Local::now();
        if dt.date_naive() == now.date_naive() {
            dt.format("%H:%M").to_string()
        } else if dt.date_naive() == now.date_naive().pred_opt().unwrap_or(dt.date_naive()) {
            format!("Yesterday {}", dt.format("%H:%M"))
        } else {
            dt.format("%d/%m/%Y").to_string()
        }
    } else {
        String::new()
    }
}

/// Formats word and character count stats for display.
fn format_note_stats(text: &str) -> String {
    let words = text.split_whitespace().count();
    let chars = text.chars().count();
    let word_str = if words == 1 {
        "1 word".to_string()
    } else {
        format!("{} words", words)
    };
    let char_str = if chars == 1 {
        "1 char".to_string()
    } else {
        format!("{} chars", chars)
    };
    format!("{} • {}", word_str, char_str)
}

type RefreshFn = Rc<dyn Fn()>;

struct NoteSessionState {
    current_id: Option<i64>,
    pinned: bool,
    dirty: bool,
    save_timeout: Option<glib::SourceId>,
}

fn build_and_present_notes_palette(ctx: &AppContext) {
    let window = libadwaita::Window::new();
    window.set_title(Some("Quick Notes"));
    window.set_default_size(680, 480);
    window.set_modal(true);
    window.set_resizable(true);
    window.set_deletable(true);
    window.add_css_class("dialog");

    if let Ok(mut guard) = NOTE_WINDOW.lock() {
        *guard = Some(glib::SendWeakRef::from(window.downgrade()));
    }

    let toast_overlay = libadwaita::ToastOverlay::new();
    let main_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    toast_overlay.set_child(Some(&main_box));
    window.set_content(Some(&toast_overlay));

    // ========================================================================
    // Header Bar
    // ========================================================================
    let header_bar = libadwaita::HeaderBar::new();
    header_bar.set_show_end_title_buttons(true);

    // Sidebar Toggle (Saved Notes Complement)
    let sidebar_toggle = gtk4::ToggleButton::new();
    sidebar_toggle.set_icon_name("view-list-bullet-symbolic");
    sidebar_toggle.set_tooltip_text(Some("Saved Notes (Ctrl+O)"));
    sidebar_toggle.add_css_class("flat");
    header_bar.pack_start(&sidebar_toggle);

    // New Note Button
    let new_note_btn = gtk4::Button::from_icon_name("document-new-symbolic");
    new_note_btn.set_tooltip_text(Some("New Note (Ctrl+N)"));
    new_note_btn.add_css_class("flat");
    header_bar.pack_start(&new_note_btn);

    // Window Title (First line of note or "New Note")
    let window_title = libadwaita::WindowTitle::new("New Note", "Auto-save enabled");
    header_bar.set_title_widget(Some(&window_title));

    // Delete Note Button (farthest right visually, nearest close button)
    let delete_btn = gtk4::Button::from_icon_name("user-trash-symbolic");
    delete_btn.set_tooltip_text(Some("Delete note"));
    delete_btn.add_css_class("flat");
    header_bar.pack_end(&delete_btn);

    // Copy Content Button
    let copy_btn = gtk4::Button::from_icon_name("edit-copy-symbolic");
    copy_btn.set_tooltip_text(Some("Copy Note (Ctrl+C)"));
    copy_btn.add_css_class("flat");
    header_bar.pack_end(&copy_btn);

    // Star / Pin Button
    let pin_btn = gtk4::Button::from_icon_name("non-starred-symbolic");
    pin_btn.set_tooltip_text(Some("Pin note"));
    pin_btn.add_css_class("flat");
    header_bar.pack_end(&pin_btn);

    main_box.append(&header_bar);

    // ========================================================================
    // Body Layout (Collapsible Sidebar Complement + Pure Input Area)
    // ========================================================================
    let body_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    body_box.set_vexpand(true);
    body_box.set_hexpand(true);
    main_box.append(&body_box);

    // 1. Saved Notes Sidebar Complement (Hidden by default to focus on input)
    let sidebar_revealer = gtk4::Revealer::new();
    sidebar_revealer.set_transition_type(gtk4::RevealerTransitionType::SlideRight);
    sidebar_revealer.set_transition_duration(200);
    sidebar_revealer.set_reveal_child(false);
    sidebar_revealer.set_visible(false);

    let sidebar_box = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
    sidebar_box.set_size_request(240, -1);
    sidebar_box.set_margin_start(8);
    sidebar_box.set_margin_end(8);
    sidebar_box.set_margin_top(8);
    sidebar_box.set_margin_bottom(8);

    let search_entry = gtk4::SearchEntry::new();
    search_entry.set_placeholder_text(Some("Search saved notes…"));
    sidebar_box.append(&search_entry);

    let notes_scrolled = gtk4::ScrolledWindow::new();
    notes_scrolled.set_vexpand(true);
    notes_scrolled.set_hexpand(true);

    let notes_list_box = gtk4::ListBox::new();
    notes_list_box.add_css_class("navigation-sidebar");
    notes_list_box.set_selection_mode(gtk4::SelectionMode::Single);
    notes_list_box.set_activate_on_single_click(true);
    notes_scrolled.set_child(Some(&notes_list_box));
    sidebar_box.append(&notes_scrolled);

    sidebar_revealer.set_child(Some(&sidebar_box));
    body_box.append(&sidebar_revealer);

    let separator = gtk4::Separator::new(gtk4::Orientation::Vertical);
    separator.set_visible(false);
    body_box.append(&separator);

    // 2. Pure Note Input Canvas (Distraction-free focus, takes full width)
    let editor_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    editor_box.set_vexpand(true);
    editor_box.set_hexpand(true);

    let editor_scrolled = gtk4::ScrolledWindow::new();
    editor_scrolled.set_vexpand(true);
    editor_scrolled.set_hexpand(true);

    let text_view = gtk4::TextView::new();
    text_view.set_wrap_mode(gtk4::WrapMode::Word);
    text_view.set_left_margin(24);
    text_view.set_right_margin(24);
    text_view.set_top_margin(20);
    text_view.set_bottom_margin(20);
    editor_scrolled.set_child(Some(&text_view));
    editor_box.append(&editor_scrolled);

    // Subtle bottom status bar
    let status_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    status_box.set_margin_start(16);
    status_box.set_margin_end(16);
    status_box.set_margin_top(4);
    status_box.set_margin_bottom(6);

    let status_label = gtk4::Label::new(Some("Ready (first line becomes title)"));
    status_label.add_css_class("dim-label");
    status_label.add_css_class("caption");
    status_box.append(&status_label);

    let spacer = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    status_box.append(&spacer);

    let stats_label = gtk4::Label::new(Some("0 words • 0 chars"));
    stats_label.add_css_class("dim-label");
    stats_label.add_css_class("caption");
    status_box.append(&stats_label);

    editor_box.append(&status_box);
    body_box.append(&editor_box);

    // ========================================================================
    // State and Auto-Save Implementation
    // ========================================================================
    let buffer = text_view.buffer();
    let suppress_changed = Rc::new(Cell::new(false));
    let state = Rc::new(RefCell::new(NoteSessionState {
        current_id: None,
        pinned: false,
        dirty: false,
        save_timeout: None,
    }));

    // Forward declaration of list refresh
    let refresh_sidebar_notes: Rc<RefCell<Option<RefreshFn>>> = Rc::new(RefCell::new(None));

    let do_flush_save = {
        let ctx = ctx.clone();
        let buf = buffer.clone();
        let st = state.clone();
        let w_title = window_title.clone();
        let s_label = status_label.clone();
        let refresh_holder = refresh_sidebar_notes.clone();

        Rc::new(move || {
            let mut s = match st.try_borrow_mut() {
                Ok(g) => g,
                Err(_) => return,
            };
            if !s.dirty {
                return;
            }
            if let Some(src) = s.save_timeout.take() {
                src.remove();
            }

            let (start, end) = buf.bounds();
            let content = buf.text(&start, &end, true).to_string();

            if content.trim().is_empty() {
                s.dirty = false;
                s_label.set_text("Ready");
                w_title.set_subtitle("Draft");
                return;
            }

            let (title, tags) = parse_note_content(&content);

            match s.current_id {
                Some(id) => {
                    if let Ok(()) = ctx.history.update_note(id, title.clone(), content, tags) {
                        s.dirty = false;
                        w_title.set_title(&title);
                        w_title.set_subtitle("Saved");
                        s_label.set_text("✓ Saved");
                    }
                }
                None => {
                    if let Ok(saved) = ctx.history.save_note(title.clone(), content, tags) {
                        s.current_id = Some(saved.id);
                        s.pinned = saved.pinned;
                        s.dirty = false;
                        w_title.set_title(&title);
                        w_title.set_subtitle("Saved");
                        s_label.set_text("✓ Saved");
                    }
                }
            }
            drop(s);
            if let Some(ref refresh) = *refresh_holder.borrow() {
                refresh();
            }
        })
    };

    // Construct the sidebar refresh closure
    {
        let ctx = ctx.clone();
        let s_entry = search_entry.clone();
        let st = state.clone();

        let notes_data: Rc<RefCell<Vec<(crate::managers::history::SuiteNote, gtk4::ListBoxRow)>>> =
            Rc::new(RefCell::new(Vec::new()));

        let notes_data_refresh = notes_data.clone();
        let list_box_refresh = notes_list_box.clone();
        let refresh_fn = Rc::new(move || {
            notes_data_refresh.borrow_mut().clear();
            while let Some(child) = list_box_refresh.first_child() {
                list_box_refresh.remove(&child);
            }

            let query = s_entry.text().to_string();
            let query_opt = if query.trim().is_empty() {
                None
            } else {
                Some(query.as_str())
            };

            if let Ok(notes) = ctx.history.list_notes(query_opt) {
                if notes.is_empty() {
                    let empty_row = gtk4::ListBoxRow::new();
                    empty_row.set_selectable(false);
                    empty_row.set_activatable(false);
                    let empty_lbl = gtk4::Label::new(Some("No saved notes found"));
                    empty_lbl.add_css_class("dim-label");
                    empty_lbl.add_css_class("caption");
                    empty_lbl.set_margin_top(16);
                    empty_lbl.set_margin_bottom(16);
                    empty_row.set_child(Some(&empty_lbl));
                    list_box_refresh.append(&empty_row);
                    return;
                }

                let active_id = st.try_borrow().ok().and_then(|s| s.current_id);

                for note in notes {
                    let row = gtk4::ListBoxRow::new();
                    row.add_css_class("sidebar-row");
                    row.set_activatable(true);
                    row.set_selectable(true);

                    let item_box = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
                    item_box.set_margin_start(10);
                    item_box.set_margin_end(10);
                    item_box.set_margin_top(8);
                    item_box.set_margin_bottom(8);

                    // Title + Star + Date
                    let title_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
                    if note.pinned {
                        let star = gtk4::Image::from_icon_name("starred-symbolic");
                        star.set_pixel_size(12);
                        title_row.append(&star);
                    }
                    let title_lbl = gtk4::Label::new(Some(&note.title));
                    title_lbl.add_css_class("heading");
                    title_lbl.set_halign(gtk4::Align::Start);
                    title_lbl.set_ellipsize(gtk4::pango::EllipsizeMode::End);
                    title_lbl.set_hexpand(true);
                    title_row.append(&title_lbl);

                    let date_lbl = gtk4::Label::new(Some(&format_note_timestamp(note.updated_at)));
                    date_lbl.add_css_class("caption");
                    date_lbl.add_css_class("dim-label");
                    title_row.append(&date_lbl);

                    item_box.append(&title_row);

                    // Snippet of note body
                    let snippet = note
                        .content
                        .lines()
                        .skip(1)
                        .find(|l| !l.trim().is_empty())
                        .unwrap_or("");
                    if !snippet.is_empty() {
                        let snippet_lbl = gtk4::Label::new(Some(snippet));
                        snippet_lbl.add_css_class("caption");
                        snippet_lbl.add_css_class("dim-label");
                        snippet_lbl.set_halign(gtk4::Align::Start);
                        snippet_lbl.set_ellipsize(gtk4::pango::EllipsizeMode::End);
                        item_box.append(&snippet_lbl);
                    }

                    row.set_child(Some(&item_box));

                    if Some(note.id) == active_id {
                        list_box_refresh.select_row(Some(&row));
                    }

                    // Gesture click for instant response
                    let click = gtk4::GestureClick::new();
                    let row_weak = row.downgrade();
                    click.connect_released(move |_, _, _, _| {
                        if let Some(r) = row_weak.upgrade() {
                            r.activate();
                        }
                    });
                    row.add_controller(click);

                    notes_data_refresh.borrow_mut().push((note, row.clone()));
                    list_box_refresh.append(&row);
                }
            }
        });

        // Row activation handler (click or Enter key)
        let list_data_act = notes_data.clone();
        let list_st = state.clone();
        let list_buf = buffer.clone();
        let list_w_title = window_title.clone();
        let list_s_label = status_label.clone();
        let list_stats_lbl = stats_label.clone();
        let list_p_btn = pin_btn.clone();
        let list_tv = text_view.clone();
        let list_flush = do_flush_save.clone();
        let list_box_ref = notes_list_box.clone();
        let list_suppress = suppress_changed.clone();

        notes_list_box.connect_row_activated(move |_list, activated_row| {
            let found = list_data_act
                .borrow()
                .iter()
                .find(|(_, r)| r == activated_row)
                .map(|(n, _)| n.clone());

            if let Some(note) = found {
                if let Ok(s) = list_st.try_borrow() {
                    if s.current_id == Some(note.id) {
                        list_tv.grab_focus();
                        return;
                    }
                }

                // Flush pending edits on current note before switching
                list_flush();

                list_suppress.set(true);
                list_buf.set_text(&note.content);
                list_suppress.set(false);

                if let Ok(mut s) = list_st.try_borrow_mut() {
                    s.current_id = Some(note.id);
                    s.pinned = note.pinned;
                    s.dirty = false;
                }

                list_w_title.set_title(&note.title);
                list_w_title.set_subtitle("Saved");
                list_s_label.set_text("✓ Saved");
                list_p_btn.set_icon_name(if note.pinned {
                    "starred-symbolic"
                } else {
                    "non-starred-symbolic"
                });
                list_stats_lbl.set_text(&format_note_stats(&note.content));

                // Visually highlight this row in the sidebar
                for (n, r) in list_data_act.borrow().iter() {
                    if n.id == note.id {
                        list_box_ref.select_row(Some(r));
                        break;
                    }
                }

                list_tv.grab_focus();
            }
        });

        *refresh_sidebar_notes.borrow_mut() = Some(refresh_fn.clone());

        // Connect SearchEntry
        let search_refresh = refresh_fn.clone();
        search_entry.connect_search_changed(move |_| {
            search_refresh();
        });

        // Connect sidebar toggle to show/hide revealer and separator cleanly
        let rev = sidebar_revealer.clone();
        let sep = separator.clone();
        let s_entry_focus = search_entry.clone();
        let refresh_on_toggle = refresh_fn;
        sidebar_toggle.connect_toggled(move |btn| {
            let active = btn.is_active();
            if active {
                rev.set_visible(true);
                sep.set_visible(true);
                rev.set_reveal_child(true);
                refresh_on_toggle();
                s_entry_focus.grab_focus();
            } else {
                rev.set_reveal_child(false);
                rev.set_visible(false);
                sep.set_visible(false);
            }
        });
    }

    // Connect text buffer changed for debounced auto-save and stats
    {
        let buf = buffer.clone();
        let st = state.clone();
        let w_title = window_title.clone();
        let s_label = status_label.clone();
        let stats_lbl = stats_label.clone();
        let flush = do_flush_save.clone();
        let suppress_changed_watch = suppress_changed.clone();

        buffer.connect_changed(move |_| {
            if suppress_changed_watch.get() {
                return;
            }

            let (start, end) = buf.bounds();
            let text = buf.text(&start, &end, true).to_string();

            stats_lbl.set_text(&format_note_stats(&text));

            // Dynamic title from first line
            let (title, _) = parse_note_content(&text);
            if !text.trim().is_empty() {
                w_title.set_title(&title);
            } else {
                w_title.set_title("New Note");
            }

            if let Ok(mut s) = st.try_borrow_mut() {
                s.dirty = true;
                s_label.set_text("Saving…");
                w_title.set_subtitle("Saving…");

                // Cancel prior timer
                if let Some(src) = s.save_timeout.take() {
                    src.remove();
                }

                // Debounced auto-save (500ms after last keystroke)
                let debounced = flush.clone();
                let src_id = glib::timeout_add_local_once(Duration::from_millis(500), move || {
                    debounced();
                });
                s.save_timeout = Some(src_id);
            }
        });
    }

    // New Note Action
    let create_new_note = {
        let buf = buffer.clone();
        let st = state.clone();
        let w_title = window_title.clone();
        let s_label = status_label.clone();
        let stats_lbl = stats_label.clone();
        let p_btn = pin_btn.clone();
        let tv = text_view.clone();
        let flush = do_flush_save.clone();
        let list_box = notes_list_box.clone();
        let suppress_new = suppress_changed.clone();

        Rc::new(move || {
            flush();
            suppress_new.set(true);
            buf.set_text("");
            suppress_new.set(false);

            if let Ok(mut s) = st.try_borrow_mut() {
                s.current_id = None;
                s.pinned = false;
                s.dirty = false;
            }

            w_title.set_title("New Note");
            w_title.set_subtitle("Auto-save enabled");
            s_label.set_text("Ready (first line becomes title)");
            stats_lbl.set_text(&format_note_stats(""));
            p_btn.set_icon_name("non-starred-symbolic");
            list_box.unselect_all();
            tv.grab_focus();
        })
    };

    let new_btn_act = create_new_note.clone();
    new_note_btn.connect_clicked(move |_| {
        new_btn_act();
    });

    // Star / Pin Action
    {
        let ctx_pin = ctx.clone();
        let st_pin = state.clone();
        let p_btn_ref = pin_btn.clone();
        let toast_pin = toast_overlay.clone();
        let refresh_holder = refresh_sidebar_notes.clone();
        let flush_pin = do_flush_save.clone();

        pin_btn.connect_clicked(move |_| {
            if let Ok(s) = st_pin.try_borrow() {
                if s.current_id.is_none() && s.dirty {
                    drop(s);
                    flush_pin();
                }
            }
            let opt_id = st_pin.try_borrow().ok().and_then(|s| s.current_id);
            if let Some(id) = opt_id {
                if let Ok(new_pinned) = ctx_pin.history.toggle_pin_note(id) {
                    if let Ok(mut s) = st_pin.try_borrow_mut() {
                        s.pinned = new_pinned;
                    }
                    p_btn_ref.set_icon_name(if new_pinned {
                        "starred-symbolic"
                    } else {
                        "non-starred-symbolic"
                    });
                    toast_pin.add_toast(libadwaita::Toast::new(if new_pinned {
                        "Note pinned"
                    } else {
                        "Note unpinned"
                    }));
                    if let Some(ref refresh) = *refresh_holder.borrow() {
                        refresh();
                    }
                }
            } else {
                toast_pin.add_toast(libadwaita::Toast::new("Note must have content to pin"));
            }
        });
    }

    // Copy Content Action
    {
        let buf_copy = buffer.clone();
        let toast_copy = toast_overlay.clone();
        copy_btn.connect_clicked(move |_| {
            let (start, end) = buf_copy.bounds();
            let content = buf_copy.text(&start, &end, true).to_string();
            if !content.is_empty() {
                if let Ok(mut cb) = arboard::Clipboard::new() {
                    let _ = cb.set_text(&content);
                }
                toast_copy.add_toast(libadwaita::Toast::new("Copied note to clipboard"));
            }
        });
    }

    // Delete Note Action
    {
        let ctx_del = ctx.clone();
        let st_del = state.clone();
        let new_note_del = create_new_note.clone();
        let toast_del = toast_overlay.clone();
        let refresh_holder = refresh_sidebar_notes.clone();

        delete_btn.connect_clicked(move |_| {
            let opt_id = st_del.try_borrow().ok().and_then(|s| s.current_id);
            if let Some(id) = opt_id {
                if let Ok(mut s) = st_del.try_borrow_mut() {
                    s.dirty = false;
                }
                if let Ok(()) = ctx_del.history.delete_note(id) {
                    new_note_del();
                    toast_del.add_toast(libadwaita::Toast::new("Note deleted"));
                    if let Some(ref refresh) = *refresh_holder.borrow() {
                        refresh();
                    }
                }
            } else {
                new_note_del();
            }
        });
    }

    // Initial sidebar population
    if let Some(ref refresh) = *refresh_sidebar_notes.borrow() {
        refresh();
    }

    // Keyboard Shortcuts:
    // Escape = flush save and close
    // Ctrl+O = toggle sidebar complement
    // Ctrl+N = new note
    // Ctrl+S = immediate flush save
    let key_controller = gtk4::EventControllerKey::new();
    let flush_key = do_flush_save.clone();
    let win_weak = glib::SendWeakRef::from(window.downgrade());
    let toggle_side = sidebar_toggle.clone();
    let new_key = create_new_note.clone();

    key_controller.connect_key_pressed(move |_, key, _, state_mod| {
        if key == gdk4::Key::Escape {
            flush_key();
            if let Some(w) = win_weak.clone().into_weak_ref().upgrade() {
                w.close();
                return glib::Propagation::Stop;
            }
        } else if state_mod.contains(gdk4::ModifierType::CONTROL_MASK) {
            match key {
                gdk4::Key::o | gdk4::Key::O => {
                    toggle_side.set_active(!toggle_side.is_active());
                    return glib::Propagation::Stop;
                }
                gdk4::Key::n | gdk4::Key::N => {
                    new_key();
                    return glib::Propagation::Stop;
                }
                gdk4::Key::s | gdk4::Key::S => {
                    flush_key();
                    return glib::Propagation::Stop;
                }
                _ => {}
            }
        }
        glib::Propagation::Proceed
    });
    window.add_controller(key_controller);

    // Save on window close request
    let flush_on_close = do_flush_save.clone();
    window.connect_close_request(move |_| {
        flush_on_close();
        glib::Propagation::Proceed
    });

    window.connect_destroy(|_| {
        if let Ok(mut guard) = NOTE_WINDOW.lock() {
            *guard = None;
        }
    });

    // Immediate focus on input area
    text_view.grab_focus();
    window.present();
}
