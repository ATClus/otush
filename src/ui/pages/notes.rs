//! Quick Notes & Idea Capture management page.

use crate::context::AppContext;
use gdk4::prelude::*;
use gtk4::prelude::*;
use libadwaita::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NoteFilter {
    All,
    Pinned,
}

/// Build the Notes & Ideas preferences page.
pub fn build(ctx: &AppContext) -> gtk4::Widget {
    let page = libadwaita::PreferencesPage::new();
    page.set_title("Notes &amp; Ideas");
    page.set_icon_name(Some("text-editor-symbolic"));

    // Filter state
    let active_filter = Rc::new(RefCell::new(NoteFilter::All));

    // ========================================================================
    // 1. Filter & Search Controls (Top)
    // ========================================================================
    let filter_group = libadwaita::PreferencesGroup::new();
    filter_group.set_title("Filter &amp; Search");
    filter_group.set_hexpand(true);

    let controls_box = gtk4::Box::new(gtk4::Orientation::Vertical, 10);
    controls_box.set_margin_top(4);
    controls_box.set_margin_bottom(8);
    controls_box.set_hexpand(true);

    let search_entry = gtk4::SearchEntry::new();
    search_entry.set_placeholder_text(Some("Search notes by title, content, or tag…"));
    search_entry.set_hexpand(true);
    controls_box.append(&search_entry);

    // Segmented linked filter bar with GNOME symbolic icons
    let filter_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    filter_row.add_css_class("linked");
    filter_row.set_halign(gtk4::Align::Center);
    filter_row.set_margin_top(2);
    filter_row.set_margin_bottom(2);

    let all_btn = gtk4::ToggleButton::new();
    all_btn.set_active(true);
    all_btn.set_size_request(140, -1);
    let all_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    all_box.set_halign(gtk4::Align::Center);
    let all_icon = gtk4::Image::from_icon_name("view-grid-symbolic");
    all_icon.set_pixel_size(16);
    all_box.append(&all_icon);
    let all_lbl = gtk4::Label::new(Some("All Notes"));
    all_box.append(&all_lbl);
    all_btn.set_child(Some(&all_box));
    filter_row.append(&all_btn);

    let pinned_btn = gtk4::ToggleButton::new();
    pinned_btn.set_group(Some(&all_btn));
    pinned_btn.set_size_request(140, -1);
    let pinned_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    pinned_box.set_halign(gtk4::Align::Center);
    let pinned_icon = gtk4::Image::from_icon_name("starred-symbolic");
    pinned_icon.set_pixel_size(16);
    pinned_box.append(&pinned_icon);
    let pinned_lbl = gtk4::Label::new(Some("Pinned"));
    pinned_box.append(&pinned_lbl);
    pinned_btn.set_child(Some(&pinned_box));
    filter_row.append(&pinned_btn);

    controls_box.append(&filter_row);

    // Quick Notes Palette Action Row
    let palette_row = libadwaita::ActionRow::new();
    palette_row.set_title("Quick Notes Palette");
    palette_row
        .set_subtitle("Capture thoughts and ideas instantly from anywhere (Shortcut: Ctrl+Alt+N)");
    palette_row.set_activatable(true);

    let palette_icon = gtk4::Image::from_icon_name("window-new-symbolic");
    palette_row.add_prefix(&palette_icon);

    let palette_btn = gtk4::Button::from_icon_name("window-new-symbolic");
    palette_btn.set_tooltip_text(Some("Open Quick Notes Palette"));
    palette_btn.set_valign(gtk4::Align::Center);
    palette_btn.add_css_class("flat");
    palette_row.add_suffix(&palette_btn);

    let ctx_palette = ctx.clone();
    palette_row.connect_activated(move |_| {
        crate::ui::notes_palette::show_notes_palette(&ctx_palette);
    });
    let ctx_palette_btn = ctx.clone();
    palette_btn.connect_clicked(move |_| {
        crate::ui::notes_palette::show_notes_palette(&ctx_palette_btn);
    });

    filter_group.add(&controls_box);
    filter_group.add(&palette_row);
    page.add(&filter_group);

    // ========================================================================
    // 2. Captured Notes List Group (Center - direct rows identical to history)
    // ========================================================================
    let list_group = libadwaita::PreferencesGroup::new();
    list_group.set_widget_name("notes-list");
    list_group.set_title("Captured Notes &amp; Ideas");
    list_group.set_description(Some(
        "Saved voice ideas, research snippets, and everyday notes.",
    ));
    list_group.set_hexpand(true);
    page.add(&list_group);

    let refresh_notes = {
        let ctx = ctx.clone();
        let group = list_group.clone();
        let search = search_entry.clone();
        let filter = active_filter.clone();

        move || {
            crate::ui::pages::clear_group_rows(&group);

            let query = search.text().to_string();
            let query_opt = if query.trim().is_empty() {
                None
            } else {
                Some(query.as_str())
            };

            let current_filter = *filter.borrow();

            if let Ok(notes) = ctx.history.list_notes(query_opt) {
                let filtered_notes: Vec<_> = notes
                    .into_iter()
                    .filter(|n| match current_filter {
                        NoteFilter::All => true,
                        NoteFilter::Pinned => n.pinned,
                    })
                    .collect();

                if filtered_notes.is_empty() {
                    let empty_row = libadwaita::ActionRow::new();
                    empty_row.set_title(if current_filter == NoteFilter::Pinned {
                        "No pinned notes"
                    } else {
                        "No notes found"
                    });
                    empty_row.set_subtitle(
                        "Use the Ctrl+Alt+N shortcut to capture thoughts and quick notes anytime.",
                    );
                    let empty_icon = gtk4::Image::from_icon_name("text-editor-symbolic");
                    empty_row.add_prefix(&empty_icon);
                    empty_row.set_activatable(false);
                    group.add(&empty_row);
                    crate::ui::pages::track_row(&group, &empty_row);
                    return;
                }

                for note in filtered_notes {
                    let row = libadwaita::ExpanderRow::new();
                    row.set_use_markup(false);
                    row.set_title(&note.title);

                    // Category / Pin symbolic icon prefix (16px)
                    let prefix_icon = gtk4::Image::from_icon_name(if note.pinned {
                        "starred-symbolic"
                    } else {
                        "text-editor-symbolic"
                    });
                    prefix_icon.set_pixel_size(16);
                    if note.pinned {
                        prefix_icon.add_css_class("accent");
                    }
                    row.add_prefix(&prefix_icon);

                    // Formatted timestamp and tags
                    let timestamp = if note.updated_at > 0 {
                        note.updated_at
                    } else {
                        note.created_at
                    };
                    let date_str = chrono::DateTime::from_timestamp(timestamp, 0)
                        .map(|t| {
                            let local = t.with_timezone(&chrono::Local);
                            local.format("%Y-%m-%d %H:%M").to_string()
                        })
                        .unwrap_or_else(|| "recent".to_string());

                    let subtitle = if note.tags.trim().is_empty() {
                        date_str
                    } else {
                        format!("{} • Tags: {}", date_str, note.tags)
                    };
                    row.set_subtitle(&subtitle);

                    // Note content view in expanded row (full-width, identical to history)
                    let content_row = libadwaita::ActionRow::new();
                    content_row.set_title("Note Content");
                    content_row.set_subtitle(&glib::markup_escape_text(&note.content));
                    content_row.set_subtitle_lines(0);
                    content_row.set_activatable(false);
                    let copy_content_btn = create_copy_button(&ctx, &note.content);
                    content_row.add_suffix(&copy_content_btn);
                    row.add_row(&content_row);

                    // Pin toggle button
                    let pin_btn = gtk4::Button::from_icon_name(if note.pinned {
                        "starred-symbolic"
                    } else {
                        "non-starred-symbolic"
                    });
                    pin_btn.set_tooltip_text(Some(if note.pinned {
                        "Unpin note"
                    } else {
                        "Pin note to top"
                    }));
                    pin_btn.set_valign(gtk4::Align::Center);
                    pin_btn.add_css_class("flat");

                    let ctx_pin = ctx.clone();
                    let note_id = note.id;
                    let prefix_icon_weak = glib::SendWeakRef::from(prefix_icon.downgrade());
                    pin_btn.connect_clicked(move |btn| {
                        if let Ok(pinned) = ctx_pin.history.toggle_pin_note(note_id) {
                            btn.set_icon_name(if pinned {
                                "starred-symbolic"
                            } else {
                                "non-starred-symbolic"
                            });
                            btn.set_tooltip_text(Some(if pinned {
                                "Unpin note"
                            } else {
                                "Pin note to top"
                            }));

                            if let Some(icon) = prefix_icon_weak.clone().into_weak_ref().upgrade() {
                                icon.set_icon_name(Some(if pinned {
                                    "starred-symbolic"
                                } else {
                                    "text-editor-symbolic"
                                }));
                                if pinned {
                                    icon.add_css_class("accent");
                                } else {
                                    icon.remove_css_class("accent");
                                }
                            }
                        }
                    });
                    row.add_suffix(&pin_btn);

                    // Copy button with visual feedback
                    let copy_btn = create_copy_button(&ctx, &note.content);
                    row.add_suffix(&copy_btn);

                    // Delete button
                    let del_btn = gtk4::Button::from_icon_name("user-trash-symbolic");
                    del_btn.set_tooltip_text(Some("Delete note"));
                    del_btn.set_valign(gtk4::Align::Center);
                    del_btn.add_css_class("flat");
                    let ctx_del = ctx.clone();
                    let del_id = note.id;
                    let group_weak = glib::SendWeakRef::from(group.downgrade());
                    let row_del_weak = glib::SendWeakRef::from(row.downgrade());
                    del_btn.connect_clicked(move |_| {
                        let _ = ctx_del.history.delete_note(del_id);
                        if let (Some(g), Some(r)) = (
                            group_weak.clone().into_weak_ref().upgrade(),
                            row_del_weak.clone().into_weak_ref().upgrade(),
                        ) {
                            g.remove(&r);
                        }
                    });
                    row.add_suffix(&del_btn);

                    group.add(&row);
                    crate::ui::pages::track_row(&group, &row);
                }
            }
        }
    };

    refresh_notes();

    // Wire segmented filter buttons
    let ref_all = refresh_notes.clone();
    let filter_all = active_filter.clone();
    all_btn.connect_toggled(move |btn| {
        if btn.is_active() {
            *filter_all.borrow_mut() = NoteFilter::All;
            ref_all();
        }
    });

    let ref_pinned = refresh_notes.clone();
    let filter_pinned = active_filter.clone();
    pinned_btn.connect_toggled(move |btn| {
        if btn.is_active() {
            *filter_pinned.borrow_mut() = NoteFilter::Pinned;
            ref_pinned();
        }
    });

    // Wire search entry
    let ref_search = refresh_notes.clone();
    search_entry.connect_search_changed(move |_| {
        ref_search();
    });

    page.upcast::<gtk4::Widget>()
}

fn create_copy_button(ctx: &AppContext, text: &str) -> gtk4::Button {
    let btn = gtk4::Button::from_icon_name("edit-copy-symbolic");
    btn.set_tooltip_text(Some("Copy note content"));
    btn.set_valign(gtk4::Align::Center);
    btn.add_css_class("flat");

    if text.trim().is_empty() {
        btn.set_sensitive(false);
        btn.set_tooltip_text(Some("Note is empty"));
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
                    btn.set_tooltip_text(Some("Copy note content"));
                }
            });
        }
    });
    btn
}
