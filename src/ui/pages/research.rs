//! Web & Deep Research page in sidebar.

use crate::context::AppContext;
use crate::settings;
use gdk4::prelude::*;
use gtk4::prelude::*;
use libadwaita::prelude::*;
use log::warn;

/// Build the Research preferences page.
pub fn build(ctx: &AppContext) -> gtk4::Widget {
    let page = libadwaita::PreferencesPage::new();
    page.set_title("Search &amp; Research");
    page.set_icon_name(Some("system-search-symbolic"));

    // ========================================================================
    // 1. Web Research Query & Settings Group (Top)
    // ========================================================================
    let query_group = libadwaita::PreferencesGroup::new();
    query_group.set_title("AI Web &amp; Deep Research");
    query_group.set_description(Some(
        "Conduct live web investigations and multi-source AI research via Tavily and Firecrawl.",
    ));
    query_group.set_hexpand(true);

    // Search input row
    let input_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    input_box.set_margin_top(4);
    input_box.set_margin_bottom(6);
    input_box.set_hexpand(true);

    let search_entry = gtk4::SearchEntry::new();
    search_entry.set_hexpand(true);
    search_entry.set_placeholder_text(Some("Enter research topic or question… (Press Enter)"));
    input_box.append(&search_entry);

    let search_btn = gtk4::Button::new();
    let search_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    let search_icon = gtk4::Image::from_icon_name("system-search-symbolic");
    search_box.append(&search_icon);
    let search_lbl = gtk4::Label::new(Some("Search"));
    search_box.append(&search_lbl);
    search_btn.set_child(Some(&search_box));
    search_btn.add_css_class("suggested-action");
    input_box.append(&search_btn);
    query_group.add(&input_box);

    // Deep Research Switch Row
    let deep_switch = libadwaita::SwitchRow::new();
    deep_switch.set_title("Deep Multi-Source Investigation");
    deep_switch.set_subtitle("Synthesize multiple web sources with comprehensive AI reasoning");
    let deep_icon = gtk4::Image::from_icon_name("starred-symbolic");
    deep_switch.add_prefix(&deep_icon);
    query_group.add(&deep_switch);

    // Quick Search Overlay Action Row
    let overlay_row = libadwaita::ActionRow::new();
    overlay_row.set_title("Quick Search Overlay");
    overlay_row
        .set_subtitle("Open floating web research palette from anywhere (Shortcut: Ctrl+Alt+S)");
    overlay_row.set_activatable(true);

    let overlay_icon = gtk4::Image::from_icon_name("window-new-symbolic");
    overlay_row.add_prefix(&overlay_icon);

    let overlay_btn = gtk4::Button::from_icon_name("window-new-symbolic");
    overlay_btn.set_tooltip_text(Some("Open Quick Search Overlay"));
    overlay_btn.set_valign(gtk4::Align::Center);
    overlay_btn.add_css_class("flat");
    overlay_row.add_suffix(&overlay_btn);

    let ctx_overlay = ctx.clone();
    overlay_row.connect_activated(move |_| {
        crate::ui::search_overlay::show_search_overlay(&ctx_overlay);
    });
    let ctx_overlay_btn = ctx.clone();
    overlay_btn.connect_clicked(move |_| {
        crate::ui::search_overlay::show_search_overlay(&ctx_overlay_btn);
    });

    query_group.add(&overlay_row);
    page.add(&query_group);

    // ========================================================================
    // 2. Results & Synthesis Group (Center)
    // ========================================================================
    let results_group = libadwaita::PreferencesGroup::new();
    results_group.set_title("Research Findings &amp; Synthesis");
    results_group.set_hexpand(true);

    // Status / Progress bar
    let status_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    status_box.set_margin_top(4);
    status_box.set_margin_bottom(8);
    status_box.set_hexpand(true);

    let spinner = gtk4::Spinner::new();
    spinner.set_visible(false);
    status_box.append(&spinner);

    let status_label = gtk4::Label::new(Some("Enter a query above to start research"));
    status_label.add_css_class("caption");
    status_label.add_css_class("dim-label");
    status_box.append(&status_label);
    results_group.add(&status_box);

    // Results scrolled textview
    let scrolled = gtk4::ScrolledWindow::new();
    scrolled.set_vexpand(true);
    scrolled.set_hexpand(true);
    scrolled.set_min_content_height(350);

    let text_view = gtk4::TextView::new();
    text_view.set_editable(false);
    text_view.set_cursor_visible(false);
    text_view.set_wrap_mode(gtk4::WrapMode::Word);
    text_view.set_left_margin(12);
    text_view.set_right_margin(12);
    text_view.set_top_margin(12);
    text_view.set_bottom_margin(12);
    text_view.add_css_class("card");
    scrolled.set_child(Some(&text_view));
    results_group.add(&scrolled);

    // Action buttons bar with GNOME symbolic icons
    let action_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    action_box.set_halign(gtk4::Align::End);
    action_box.set_margin_top(8);
    action_box.set_margin_bottom(4);

    let copy_btn = gtk4::Button::new();
    let copy_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    let copy_icon = gtk4::Image::from_icon_name("edit-copy-symbolic");
    copy_icon.set_pixel_size(16);
    copy_box.append(&copy_icon);
    let copy_lbl = gtk4::Label::new(Some("Copy Findings"));
    copy_box.append(&copy_lbl);
    copy_btn.set_child(Some(&copy_box));
    copy_btn.set_sensitive(false);
    action_box.append(&copy_btn);

    let save_note_btn = gtk4::Button::new();
    let note_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    let note_icon = gtk4::Image::from_icon_name("text-editor-symbolic");
    note_icon.set_pixel_size(16);
    note_box.append(&note_icon);
    let note_lbl = gtk4::Label::new(Some("Save to Notes"));
    note_box.append(&note_lbl);
    save_note_btn.set_child(Some(&note_box));
    save_note_btn.set_sensitive(false);
    action_box.append(&save_note_btn);

    results_group.add(&action_box);
    page.add(&results_group);

    // Wire Copy button with visual feedback animation
    let tv_copy = text_view.clone();
    let status_copy = status_label.clone();
    let ctx_copy = ctx.clone();
    let copy_icon_weak = glib::SendWeakRef::from(copy_icon.downgrade());
    let copy_lbl_weak = glib::SendWeakRef::from(copy_lbl.downgrade());

    copy_btn.connect_clicked(move |_| {
        let buffer = tv_copy.buffer();
        let (start, end) = buffer.bounds();
        let text = buffer.text(&start, &end, true).to_string();
        if !text.is_empty() {
            let _ = crate::clipboard::write_clipboard_text(&ctx_copy, &text);
            if let Some(disp) = gdk4::Display::default() {
                disp.clipboard().set_text(&text);
            }
            status_copy.set_text("Copied findings to clipboard!");

            if let (Some(icon), Some(lbl)) = (
                copy_icon_weak.clone().into_weak_ref().upgrade(),
                copy_lbl_weak.clone().into_weak_ref().upgrade(),
            ) {
                icon.set_icon_name(Some("object-select-symbolic"));
                lbl.set_text("Copied!");

                let icon_reset = glib::SendWeakRef::from(icon.downgrade());
                let lbl_reset = glib::SendWeakRef::from(lbl.downgrade());
                glib::timeout_add_local_once(std::time::Duration::from_millis(1500), move || {
                    if let (Some(i), Some(l)) = (
                        icon_reset.into_weak_ref().upgrade(),
                        lbl_reset.into_weak_ref().upgrade(),
                    ) {
                        i.set_icon_name(Some("edit-copy-symbolic"));
                        l.set_text("Copy Findings");
                    }
                });
            }
        }
    });

    // Wire Save Note button
    let tv_note = text_view.clone();
    let entry_note = search_entry.clone();
    let status_note = status_label.clone();
    let ctx_note = ctx.clone();
    let note_icon_weak = glib::SendWeakRef::from(note_icon.downgrade());
    let note_lbl_weak = glib::SendWeakRef::from(note_lbl.downgrade());

    save_note_btn.connect_clicked(move |_| {
        let buffer = tv_note.buffer();
        let (start, end) = buffer.bounds();
        let content = buffer.text(&start, &end, true).to_string();
        let title = entry_note.text().to_string();
        if !content.is_empty() {
            let note_title = if title.trim().is_empty() {
                "Research Findings".to_string()
            } else {
                format!("Research: {}", title.trim())
            };
            match ctx_note
                .history
                .save_note(note_title, content, Some("research,web".to_string()))
            {
                Ok(_) => {
                    status_note.set_text("Saved to Notes library!");
                    if let (Some(icon), Some(lbl)) = (
                        note_icon_weak.clone().into_weak_ref().upgrade(),
                        note_lbl_weak.clone().into_weak_ref().upgrade(),
                    ) {
                        icon.set_icon_name(Some("object-select-symbolic"));
                        lbl.set_text("Saved!");
                        let icon_reset = glib::SendWeakRef::from(icon.downgrade());
                        let lbl_reset = glib::SendWeakRef::from(lbl.downgrade());
                        glib::timeout_add_local_once(
                            std::time::Duration::from_millis(1500),
                            move || {
                                if let (Some(i), Some(l)) = (
                                    icon_reset.into_weak_ref().upgrade(),
                                    lbl_reset.into_weak_ref().upgrade(),
                                ) {
                                    i.set_icon_name(Some("text-editor-symbolic"));
                                    l.set_text("Save to Notes");
                                }
                            },
                        );
                    }
                }
                Err(e) => status_note.set_text(&format!("Error saving note: {e}")),
            }
        }
    });

    // Search action execution
    let do_search = {
        let ctx = ctx.clone();
        let entry = search_entry.clone();
        let switch = deep_switch.clone();
        let status = status_label.clone();
        let spin = spinner.clone();
        let tv = text_view.clone();
        let c_btn = copy_btn.clone();
        let s_btn = save_note_btn.clone();

        move || {
            let query = entry.text().to_string();
            if query.trim().is_empty() {
                return;
            }

            let is_deep = switch.is_active();
            spin.set_visible(true);
            spin.start();
            status.set_text(if is_deep {
                "Conducting deep multi-source research…"
            } else {
                "Searching the web…"
            });
            c_btn.set_sensitive(false);
            s_btn.set_sensitive(false);

            let settings = settings::get_settings(&ctx);
            let tavily_key = settings
                .web_api_keys
                .get("tavily")
                .cloned()
                .unwrap_or_default();
            let firecrawl_key = settings
                .web_api_keys
                .get("firecrawl")
                .cloned()
                .unwrap_or_default();

            let tavily_base = settings
                .web_providers
                .iter()
                .find(|p| p.id == "tavily")
                .map(|p| p.base_url.clone())
                .unwrap_or_else(|| "https://api.tavily.com".to_string());

            let firecrawl_base = settings
                .web_providers
                .iter()
                .find(|p| p.id == "firecrawl")
                .map(|p| p.base_url.clone())
                .unwrap_or_else(|| "https://api.firecrawl.dev/v2".to_string());

            let status_weak = glib::SendWeakRef::from(status.downgrade());
            let spin_weak = glib::SendWeakRef::from(spin.downgrade());
            let tv_weak = glib::SendWeakRef::from(tv.downgrade());
            let c_btn_weak = glib::SendWeakRef::from(c_btn.downgrade());
            let s_btn_weak = glib::SendWeakRef::from(s_btn.downgrade());

            crate::runtime::spawn(async move {
                let mut rendered = String::new();

                // 1. Try Tavily first
                if !tavily_key.trim().is_empty() {
                    let depth = if is_deep { "advanced" } else { "basic" };
                    match crate::web_client::tavily_search(
                        &tavily_base,
                        &tavily_key,
                        &query,
                        depth,
                        if is_deep { 8 } else { 5 },
                    )
                    .await
                    {
                        Ok(resp) => {
                            if let Some(ans) = resp.answer {
                                rendered.push_str("### AI Research Synthesis:\n\n");
                                rendered.push_str(&ans);
                                rendered.push_str("\n\n---\n### Sources & Key Findings:\n\n");
                            }
                            for (i, r) in resp.results.iter().enumerate() {
                                rendered.push_str(&format!(
                                    "{}. [{}]({})\n   {}\n\n",
                                    i + 1,
                                    r.title,
                                    r.url,
                                    r.content
                                ));
                            }
                        }
                        Err(e) => {
                            warn!("Tavily search failed: {e}");
                        }
                    }
                }

                // 2. If empty and Firecrawl available, try Firecrawl
                if rendered.is_empty() && !firecrawl_key.trim().is_empty() {
                    match crate::web_client::firecrawl_search(
                        &firecrawl_base,
                        &firecrawl_key,
                        &query,
                        5,
                    )
                    .await
                    {
                        Ok(results) => {
                            rendered.push_str("### Firecrawl Web Search Results:\n\n");
                            for (i, r) in results.iter().enumerate() {
                                let title = r.title.as_deref().unwrap_or("Untitled");
                                let url = r.url.as_deref().unwrap_or("#");
                                let desc = r.description.as_deref().unwrap_or("");
                                rendered.push_str(&format!(
                                    "{}. [{}]({})\n   {}\n\n",
                                    i + 1,
                                    title,
                                    url,
                                    desc
                                ));
                            }
                        }
                        Err(e) => {
                            warn!("Firecrawl search failed: {e}");
                        }
                    }
                }

                if rendered.is_empty() {
                    if tavily_key.trim().is_empty() && firecrawl_key.trim().is_empty() {
                        rendered = "No Web Providers configured. Please enter an API key for Tavily or Firecrawl in the Providers page.".to_string();
                    } else {
                        rendered = "Search completed, but no relevant results were found. Try refining your keywords.".to_string();
                    }
                }

                glib::MainContext::default().invoke(move || {
                    if let Some(spin) = spin_weak.into_weak_ref().upgrade() {
                        spin.stop();
                        spin.set_visible(false);
                    }
                    if let Some(status) = status_weak.into_weak_ref().upgrade() {
                        status.set_text("Research completed");
                    }
                    if let Some(tv) = tv_weak.into_weak_ref().upgrade() {
                        tv.buffer().set_text(&rendered);
                    }
                    if let Some(b) = c_btn_weak.into_weak_ref().upgrade() {
                        b.set_sensitive(true);
                    }
                    if let Some(b) = s_btn_weak.into_weak_ref().upgrade() {
                        b.set_sensitive(true);
                    }
                });
            });
        }
    };

    let do_search_click = do_search.clone();
    search_btn.connect_clicked(move |_| {
        do_search_click();
    });

    search_entry.connect_activate(move |_| {
        do_search();
    });

    page.upcast::<gtk4::Widget>()
}
