//! Documents & OCR Library page.

use crate::context::AppContext;
use gdk4::prelude::*;
use gtk4::prelude::*;
use libadwaita::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

/// Build the Documents & OCR Library preferences page.
pub fn build(ctx: &AppContext) -> gtk4::Widget {
    let page = libadwaita::PreferencesPage::new();
    page.set_title("Documents &amp; OCR");
    page.set_icon_name(Some("x-office-document-symbolic"));

    // Search query state
    let search_query = Rc::new(RefCell::new(String::new()));

    // ========================================================================
    // 1. Parser & Search Controls Group (Top)
    // ========================================================================
    let parser_group = libadwaita::PreferencesGroup::new();
    parser_group.set_title("OCR &amp; Document Parser");
    parser_group.set_hexpand(true);

    let controls_box = gtk4::Box::new(gtk4::Orientation::Vertical, 10);
    controls_box.set_margin_top(4);
    controls_box.set_margin_bottom(8);
    controls_box.set_hexpand(true);

    // Search bar
    let search_entry = gtk4::SearchEntry::new();
    search_entry.set_placeholder_text(Some("Search parsed documents by title, file, or content…"));
    search_entry.set_hexpand(true);
    controls_box.append(&search_entry);
    parser_group.add(&controls_box);

    // Parse New Document Action Row
    let parse_row = libadwaita::ActionRow::new();
    parse_row.set_title("Parse Document with OCR");
    parse_row.set_subtitle(
        "Extract structured Markdown from PDFs, images, or Word documents (Shortcut: Ctrl+Alt+D)",
    );
    parse_row.set_activatable(true);

    let parse_icon = gtk4::Image::from_icon_name("document-open-symbolic");
    parse_row.add_prefix(&parse_icon);

    let parse_btn = gtk4::Button::from_icon_name("document-open-symbolic");
    parse_btn.set_tooltip_text(Some("Parse Document with OCR"));
    parse_btn.set_valign(gtk4::Align::Center);
    parse_btn.add_css_class("flat");
    parse_row.add_suffix(&parse_btn);

    let ctx_parse = ctx.clone();
    parse_row.connect_activated(move |_| {
        crate::ui::doc_parser::show_doc_parser(&ctx_parse);
    });
    let ctx_parse_btn = ctx.clone();
    parse_btn.connect_clicked(move |_| {
        crate::ui::doc_parser::show_doc_parser(&ctx_parse_btn);
    });

    parser_group.add(&parse_row);
    page.add(&parser_group);

    // ========================================================================
    // 2. Parsed Documents Library Group (Center - purely direct rows)
    // ========================================================================
    let list_group = libadwaita::PreferencesGroup::new();
    list_group.set_widget_name("docs-list");
    list_group.set_title("Parsed Documents Library");
    list_group.set_description(Some(
        "Structured Markdown documents extracted via Vision OCR and document parsers.",
    ));
    list_group.set_hexpand(true);
    page.add(&list_group);

    let rows = Arc::new(Mutex::new(crate::ui::pages::PageGroup::new()));
    let refresh_docs = {
        let ctx = ctx.clone();
        let group = list_group.clone();
        let query_state = search_query.clone();
        let rows = rows.clone();

        move || {
            rows.lock().unwrap_or_else(|e| e.into_inner()).clear(&group);

            let query = query_state.borrow().trim().to_lowercase();

            if let Ok(docs) = ctx.history.list_docs() {
                let filtered_docs: Vec<_> = docs
                    .into_iter()
                    .filter(|doc| {
                        if query.is_empty() {
                            true
                        } else {
                            doc.title.to_lowercase().contains(&query)
                                || doc.file_name.to_lowercase().contains(&query)
                                || doc.parsed_content.to_lowercase().contains(&query)
                        }
                    })
                    .collect();

                if filtered_docs.is_empty() {
                    let empty_row = libadwaita::ActionRow::new();
                    let (title, subtitle) = if query.is_empty() {
                        (
                            "No documents parsed yet",
                            "Click 'Parse Document with OCR' or press Ctrl+Alt+D to extract text from a file.",
                        )
                    } else {
                        (
                            "No matching documents",
                            "No documents match your search query. Try different keywords.",
                        )
                    };
                    empty_row.set_title(title);
                    empty_row.set_subtitle(subtitle);
                    let empty_icon = gtk4::Image::from_icon_name("x-office-document-symbolic");
                    empty_row.add_prefix(&empty_icon);
                    empty_row.set_activatable(false);
                    rows.lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .add(&group, &empty_row);
                    return;
                }

                for doc in filtered_docs {
                    let row = libadwaita::ExpanderRow::new();
                    row.set_use_markup(false);
                    row.set_title(&doc.title);

                    // Formatted timestamp + format + filename subtitle
                    let date_str = chrono::DateTime::from_timestamp(doc.created_at, 0)
                        .map(|t| {
                            let local = t.with_timezone(&chrono::Local);
                            local.format("%Y-%m-%d %H:%M").to_string()
                        })
                        .unwrap_or_else(|| "Recent".to_string());

                    let format_upper = doc.doc_type.to_uppercase();
                    row.set_subtitle(&format!(
                        "{} • {} • {}",
                        date_str, format_upper, doc.file_name
                    ));

                    // Prefix icon based on doc_type
                    let icon_name = if doc.doc_type.to_lowercase().contains("png")
                        || doc.doc_type.to_lowercase().contains("jpg")
                        || doc.doc_type.to_lowercase().contains("jpeg")
                        || doc.doc_type.to_lowercase().contains("image")
                    {
                        "image-x-generic-symbolic"
                    } else {
                        "x-office-document-symbolic"
                    };
                    let doc_icon = gtk4::Image::from_icon_name(icon_name);
                    doc_icon.set_pixel_size(16);
                    row.add_prefix(&doc_icon);

                    // Copy button in header
                    let copy_header_btn = create_copy_button(&ctx, &doc.parsed_content);
                    row.add_suffix(&copy_header_btn);

                    // Read aloud button (TTS reader mode)
                    let read_btn =
                        crate::ui::tts_controls::read_aloud_button(&ctx, &doc.parsed_content);
                    row.add_suffix(&read_btn);

                    // Delete button in header
                    let del_btn = gtk4::Button::from_icon_name("user-trash-symbolic");
                    del_btn.set_tooltip_text(Some("Delete document"));
                    del_btn.set_valign(gtk4::Align::Center);
                    del_btn.add_css_class("flat");
                    let ctx_del = ctx.clone();
                    let del_id = doc.id;
                    let group_weak = glib::SendWeakRef::from(group.downgrade());
                    let row_del_weak = glib::SendWeakRef::from(row.downgrade());
                    del_btn.connect_clicked(move |_| {
                        let _ = ctx_del.history.delete_doc(del_id);
                        if let (Some(g), Some(r)) = (
                            group_weak.clone().into_weak_ref().upgrade(),
                            row_del_weak.clone().into_weak_ref().upgrade(),
                        ) {
                            g.remove(&r);
                        }
                    });
                    row.add_suffix(&del_btn);

                    // Expanded content - full-width ActionRow (same clean pattern as Notes)
                    let content_row = libadwaita::ActionRow::new();
                    content_row.set_title("Extracted Markdown Content");
                    content_row.set_subtitle(&glib::markup_escape_text(&doc.parsed_content));
                    content_row.set_subtitle_lines(0);
                    content_row.set_activatable(false);

                    let copy_body_btn = create_copy_button(&ctx, &doc.parsed_content);
                    content_row.add_suffix(&copy_body_btn);

                    row.add_row(&content_row);

                    rows.lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .add(&group, &row);
                }
            }
        }
    };

    refresh_docs();

    // Wire search entry
    let ref_search = refresh_docs.clone();
    let query_entry = search_query.clone();
    search_entry.connect_search_changed(move |entry| {
        *query_entry.borrow_mut() = entry.text().to_string();
        ref_search();
    });

    page.upcast::<gtk4::Widget>()
}

fn create_copy_button(ctx: &AppContext, text: &str) -> gtk4::Button {
    let btn = gtk4::Button::from_icon_name("edit-copy-symbolic");
    btn.set_tooltip_text(Some("Copy document text"));
    btn.set_valign(gtk4::Align::Center);
    btn.add_css_class("flat");

    if text.trim().is_empty() {
        btn.set_sensitive(false);
        btn.set_tooltip_text(Some("Content is empty"));
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
                    btn.set_tooltip_text(Some("Copy document text"));
                }
            });
        }
    });
    btn
}
