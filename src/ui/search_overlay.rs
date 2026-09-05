//! Search & Deep Research floating overlay (`Ctrl+Alt+S`).
//!
//! Native GTK4 + libadwaita web intelligence and multi-source research palette.
//! Powered by Tavily and Firecrawl APIs with AI synthesis, interactive source cards,
//! direct browser links, markdown export, and seamless integration with Quick Notes.

use crate::context::AppContext;
use crate::settings;
use gdk4::prelude::*;
use gtk4::prelude::*;
use libadwaita::prelude::*;
use log::warn;
use std::rc::Rc;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

static SEARCH_WINDOW: LazyLock<Mutex<Option<glib::SendWeakRef<libadwaita::Window>>>> =
    LazyLock::new(|| Mutex::new(None));
static LAST_SEARCH_TOGGLE: LazyLock<Mutex<Option<std::time::Instant>>> =
    LazyLock::new(|| Mutex::new(None));

/// Toggle or show the search and deep research overlay window.
pub fn show_search_overlay(ctx: &AppContext) {
    toggle_search_overlay(ctx);
}

/// Toggle display of the search and deep research overlay window.
pub fn toggle_search_overlay(ctx: &AppContext) {
    let now = std::time::Instant::now();
    if let Ok(mut last) = LAST_SEARCH_TOGGLE.lock() {
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
            let mut guard = match SEARCH_WINDOW.lock() {
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
                    if let Ok(mut guard) = SEARCH_WINDOW.lock() {
                        *guard = Some(glib::SendWeakRef::from(win.downgrade()));
                    }
                }
                return;
            }
        }

        build_and_present_search_overlay(&ctx);
    });
}

#[derive(Clone, Default)]
struct ResearchSource {
    title: String,
    url: String,
    content: String,
}

#[derive(Clone, Default)]
struct ResearchState {
    query: String,
    is_deep: bool,
    synthesis: Option<String>,
    sources: Vec<ResearchSource>,
    full_markdown: String,
}

fn build_and_present_search_overlay(ctx: &AppContext) {
    let window = libadwaita::Window::new();
    window.set_title(Some("Web & Deep Research"));
    window.set_default_size(780, 580);
    window.set_modal(true);
    window.set_resizable(true);
    window.set_deletable(true);
    window.add_css_class("dialog");

    {
        let mut guard = match SEARCH_WINDOW.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        *guard = Some(glib::SendWeakRef::from(window.downgrade()));
    }

    let toast_overlay = libadwaita::ToastOverlay::new();
    let main_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    toast_overlay.set_child(Some(&main_box));
    window.set_content(Some(&toast_overlay));

    let research_state = Arc::new(Mutex::new(ResearchState::default()));

    // ========================================================================
    // Header Bar
    // ========================================================================
    let header_bar = libadwaita::HeaderBar::new();
    header_bar.set_show_end_title_buttons(true);

    let window_title =
        libadwaita::WindowTitle::new("Web & Deep Research", "Tavily & Firecrawl Intelligence");
    header_bar.set_title_widget(Some(&window_title));

    // Clear Button (Left)
    let clear_btn = gtk4::Button::from_icon_name("view-refresh-symbolic");
    clear_btn.set_tooltip_text(Some("Reset Search (Ctrl+L)"));
    clear_btn.add_css_class("flat");
    header_bar.pack_start(&clear_btn);

    // Deep Research Mode Button (Right)
    let deep_toggle = gtk4::ToggleButton::new();
    deep_toggle.set_icon_name("starred-symbolic");
    deep_toggle.set_tooltip_text(Some("Deep Multi-Source Research (Ctrl+D)"));
    deep_toggle.add_css_class("flat");
    header_bar.pack_end(&deep_toggle);

    // Copy Results Button (Right)
    let copy_btn = gtk4::Button::from_icon_name("edit-copy-symbolic");
    copy_btn.set_tooltip_text(Some("Copy Findings as Markdown (Ctrl+C)"));
    copy_btn.add_css_class("flat");
    copy_btn.set_sensitive(false);
    header_bar.pack_end(&copy_btn);

    // Save as Note Button (Right)
    let save_note_btn = gtk4::Button::from_icon_name("document-new-symbolic");
    save_note_btn.set_tooltip_text(Some("Save to Quick Notes (Ctrl+S)"));
    save_note_btn.add_css_class("flat");
    save_note_btn.set_sensitive(false);
    header_bar.pack_end(&save_note_btn);

    main_box.append(&header_bar);

    // ========================================================================
    // Search Bar & Control Strip
    // ========================================================================
    let search_strip = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    search_strip.set_margin_start(16);
    search_strip.set_margin_end(16);
    search_strip.set_margin_top(12);
    search_strip.set_margin_bottom(6);

    let input_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    input_row.set_hexpand(true);

    let search_entry = gtk4::SearchEntry::new();
    search_entry.set_hexpand(true);
    search_entry.set_placeholder_text(Some("Search web or enter research question… (Press Enter)"));
    input_row.append(&search_entry);

    let search_btn = gtk4::Button::new();
    let search_btn_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    let search_icon = gtk4::Image::from_icon_name("system-search-symbolic");
    let search_lbl = gtk4::Label::new(Some("Research"));
    search_btn_box.append(&search_icon);
    search_btn_box.append(&search_lbl);
    search_btn.set_child(Some(&search_btn_box));
    search_btn.add_css_class("suggested-action");
    input_row.append(&search_btn);

    search_strip.append(&input_row);

    // Subtle Progress Bar for loading
    let progress_bar = gtk4::ProgressBar::new();
    progress_bar.set_visible(false);
    progress_bar.add_css_class("osd");
    search_strip.append(&progress_bar);

    main_box.append(&search_strip);

    let separator = gtk4::Separator::new(gtk4::Orientation::Horizontal);
    main_box.append(&separator);

    // ========================================================================
    // Dynamic Content Stack (Welcome / Loading / No Providers / Results)
    // ========================================================================
    let stack = gtk4::Stack::new();
    stack.set_transition_type(gtk4::StackTransitionType::Crossfade);
    stack.set_transition_duration(200);
    stack.set_vexpand(true);
    stack.set_hexpand(true);

    // 1. Welcome / Initial State
    let welcome_page = libadwaita::StatusPage::new();
    welcome_page.set_icon_name(Some("system-search-symbolic"));
    welcome_page.set_title("Web &amp; Deep Research");
    welcome_page.set_description(Some(
        "Enter a topic or research question to investigate across real-time web sources and AI models.",
    ));

    let hints_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    hints_box.set_halign(gtk4::Align::Center);
    hints_box.set_margin_top(8);

    for (icon, text) in [
        ("system-search-symbolic", "↵ Enter to Search"),
        ("starred-symbolic", "Ctrl+D for Deep Mode"),
        ("document-new-symbolic", "Ctrl+S to Save into Notes"),
    ] {
        let pill = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        pill.add_css_class("card");
        pill.set_margin_start(4);
        pill.set_margin_end(4);
        pill.set_margin_top(4);
        pill.set_margin_bottom(4);

        let img = gtk4::Image::from_icon_name(icon);
        img.set_pixel_size(14);
        pill.append(&img);

        let lbl = gtk4::Label::new(Some(text));
        lbl.add_css_class("caption");
        lbl.add_css_class("dim-label");
        pill.append(&lbl);

        hints_box.append(&pill);
    }
    welcome_page.set_child(Some(&hints_box));
    stack.add_named(&welcome_page, Some("welcome"));

    // 2. Loading State
    let loading_page = libadwaita::StatusPage::new();
    loading_page.set_title("Investigating the Web…");
    loading_page.set_description(Some(
        "Querying live intelligence sources and synthesizing findings…",
    ));
    let loading_spinner = gtk4::Spinner::new();
    loading_spinner.set_size_request(36, 36);
    loading_spinner.set_halign(gtk4::Align::Center);
    loading_spinner.set_valign(gtk4::Align::Center);
    loading_page.set_child(Some(&loading_spinner));
    stack.add_named(&loading_page, Some("loading"));

    // 3. No Providers Configured State
    let no_providers_page = libadwaita::StatusPage::new();
    no_providers_page.set_icon_name(Some("dialog-warning-symbolic"));
    no_providers_page.set_title("No Web Providers Configured");
    no_providers_page.set_description(Some(
        "Please configure an API key for Tavily or Firecrawl in Cloud Providers to enable real-time search.",
    ));
    stack.add_named(&no_providers_page, Some("no_providers"));

    // 4. No Results Found State
    let no_results_page = libadwaita::StatusPage::new();
    no_results_page.set_icon_name(Some("edit-find-symbolic"));
    no_results_page.set_title("No Relevant Results Found");
    no_results_page.set_description(Some(
        "We couldn't find any sources matching your query. Try different terms or toggle Deep Research.",
    ));
    stack.add_named(&no_results_page, Some("no_results"));

    // 5. Results View (Synthesis Card + Interactive Sources List)
    let results_scrolled = gtk4::ScrolledWindow::new();
    results_scrolled.set_vexpand(true);
    results_scrolled.set_hexpand(true);

    let results_box = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    results_box.set_margin_start(18);
    results_box.set_margin_end(18);
    results_box.set_margin_top(14);
    results_box.set_margin_bottom(16);
    results_scrolled.set_child(Some(&results_box));

    stack.add_named(&results_scrolled, Some("results"));
    stack.set_visible_child_name("welcome");

    main_box.append(&stack);

    // ========================================================================
    // Search Execution Implementation
    // ========================================================================
    let do_search = {
        let ctx = ctx.clone();
        let s_entry = search_entry.clone();
        let d_toggle = deep_toggle.clone();
        let s_btn = search_btn.clone();
        let w_title = window_title.clone();
        let st_stack = stack.clone();
        let l_spinner = loading_spinner.clone();
        let l_page = loading_page.clone();
        let r_box = results_box.clone();
        let p_bar = progress_bar.clone();
        let c_btn = copy_btn.clone();
        let s_note_btn = save_note_btn.clone();
        let toast_ref = toast_overlay.clone();
        let state_ref = research_state.clone();

        Rc::new(move || {
            let query = s_entry.text().to_string();
            if query.trim().is_empty() {
                return;
            }

            let is_deep = d_toggle.is_active();
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

            if tavily_key.trim().is_empty() && firecrawl_key.trim().is_empty() {
                st_stack.set_visible_child_name("no_providers");
                c_btn.set_sensitive(false);
                s_note_btn.set_sensitive(false);
                w_title.set_subtitle("Providers Unconfigured");
                return;
            }

            // Set UI to loading state
            s_btn.set_sensitive(false);
            p_bar.set_visible(true);
            p_bar.pulse();
            l_spinner.start();
            l_page.set_description(Some(if is_deep {
                "Conducting exhaustive multi-source AI research and analysis…"
            } else {
                "Querying live web sources and extracting findings…"
            }));
            st_stack.set_visible_child_name("loading");
            w_title.set_subtitle(if is_deep {
                "Deep Multi-Source Investigation…"
            } else {
                "Searching the Web…"
            });

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

            let query_str = query.clone();
            let state_weak = state_ref.clone();
            let s_btn_weak = glib::SendWeakRef::from(s_btn.downgrade());
            let p_bar_weak = glib::SendWeakRef::from(p_bar.downgrade());
            let l_spin_weak = glib::SendWeakRef::from(l_spinner.downgrade());
            let st_stack_weak = glib::SendWeakRef::from(st_stack.downgrade());
            let w_title_weak = glib::SendWeakRef::from(w_title.downgrade());
            let r_box_weak = glib::SendWeakRef::from(r_box.downgrade());
            let c_btn_weak = glib::SendWeakRef::from(c_btn.downgrade());
            let s_note_weak = glib::SendWeakRef::from(s_note_btn.downgrade());
            let toast_weak = glib::SendWeakRef::from(toast_ref.downgrade());

            crate::runtime::spawn(async move {
                let mut synthesis_opt = None;
                let mut sources_list = Vec::new();
                let mut markdown = String::new();

                // 1. Tavily Search
                if !tavily_key.trim().is_empty() {
                    let depth = if is_deep { "advanced" } else { "basic" };
                    match crate::web_client::tavily_search(
                        &tavily_base,
                        &tavily_key,
                        &query_str,
                        depth,
                        if is_deep { 8 } else { 5 },
                    )
                    .await
                    {
                        Ok(resp) => {
                            if let Some(ans) = resp.answer {
                                synthesis_opt = Some(ans.clone());
                                markdown.push_str("### AI Research Synthesis\n\n");
                                markdown.push_str(&ans);
                                markdown.push_str("\n\n---\n### Sources & Key Evidence\n\n");
                            }
                            for (i, r) in resp.results.iter().enumerate() {
                                sources_list.push(ResearchSource {
                                    title: r.title.clone(),
                                    url: r.url.clone(),
                                    content: r.content.clone(),
                                });
                                markdown.push_str(&format!(
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

                // 2. Firecrawl Fallback / Complement
                if sources_list.is_empty() && !firecrawl_key.trim().is_empty() {
                    match crate::web_client::firecrawl_search(
                        &firecrawl_base,
                        &firecrawl_key,
                        &query_str,
                        5,
                    )
                    .await
                    {
                        Ok(results) => {
                            markdown.push_str("### Firecrawl Web Search Results\n\n");
                            for (i, r) in results.iter().enumerate() {
                                let title = r.title.as_deref().unwrap_or("Untitled").to_string();
                                let url = r.url.as_deref().unwrap_or("#").to_string();
                                let desc = r.description.as_deref().unwrap_or("").to_string();
                                sources_list.push(ResearchSource {
                                    title: title.clone(),
                                    url: url.clone(),
                                    content: desc.clone(),
                                });
                                markdown.push_str(&format!(
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

                glib::MainContext::default().invoke(move || {
                    if let Some(btn) = s_btn_weak.into_weak_ref().upgrade() {
                        btn.set_sensitive(true);
                    }
                    if let Some(bar) = p_bar_weak.into_weak_ref().upgrade() {
                        bar.set_visible(false);
                    }
                    if let Some(spin) = l_spin_weak.into_weak_ref().upgrade() {
                        spin.stop();
                    }

                    let has_results = !sources_list.is_empty() || synthesis_opt.is_some();

                    if !has_results {
                        if let Some(st) = st_stack_weak.into_weak_ref().upgrade() {
                            st.set_visible_child_name("no_results");
                        }
                        if let Some(wt) = w_title_weak.into_weak_ref().upgrade() {
                            wt.set_subtitle("No results found");
                        }
                        if let Some(cb) = c_btn_weak.into_weak_ref().upgrade() {
                            cb.set_sensitive(false);
                        }
                        if let Some(sb) = s_note_weak.into_weak_ref().upgrade() {
                            sb.set_sensitive(false);
                        }
                        return;
                    }

                    // Update stored state
                    if let Ok(mut st) = state_weak.lock() {
                        st.query = query_str;
                        st.is_deep = is_deep;
                        st.synthesis = synthesis_opt.clone();
                        st.sources = sources_list.clone();
                        st.full_markdown = markdown;
                    }

                    // Enable action buttons
                    if let Some(cb) = c_btn_weak.into_weak_ref().upgrade() {
                        cb.set_sensitive(true);
                    }
                    if let Some(sb) = s_note_weak.into_weak_ref().upgrade() {
                        sb.set_sensitive(true);
                    }

                    if let Some(wt) = w_title_weak.into_weak_ref().upgrade() {
                        let count = sources_list.len();
                        let src_lbl = if count == 1 { "source" } else { "sources" };
                        wt.set_subtitle(&format!("{count} {src_lbl} • Tavily & Firecrawl"));
                    }

                    // Rebuild Results Box
                    if let Some(box_container) = r_box_weak.into_weak_ref().upgrade() {
                        while let Some(child) = box_container.first_child() {
                            box_container.remove(&child);
                        }

                        // 1. AI Synthesis Card (if available)
                        if let Some(ref synth) = synthesis_opt {
                            let synth_card = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
                            synth_card.add_css_class("card");
                            synth_card.set_margin_bottom(6);

                            let card_header = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
                            card_header.set_margin_start(14);
                            card_header.set_margin_end(14);
                            card_header.set_margin_top(12);

                            let spark_img = gtk4::Image::from_icon_name("starred-symbolic");
                            spark_img.set_pixel_size(16);
                            card_header.append(&spark_img);

                            let card_title = gtk4::Label::new(Some("AI Research Synthesis"));
                            card_title.add_css_class("heading");
                            card_title.set_hexpand(true);
                            card_title.set_halign(gtk4::Align::Start);
                            card_header.append(&card_title);

                            let copy_synth_btn = gtk4::Button::from_icon_name("edit-copy-symbolic");
                            copy_synth_btn.set_tooltip_text(Some("Copy Synthesis"));
                            copy_synth_btn.add_css_class("flat");
                            let synth_text = synth.clone();
                            let toast_clone = toast_weak.clone();
                            copy_synth_btn.connect_clicked(move |_| {
                                if let Ok(mut cb) = arboard::Clipboard::new() {
                                    let _ = cb.set_text(&synth_text);
                                }
                                if let Some(t) = toast_clone.clone().into_weak_ref().upgrade() {
                                    t.add_toast(libadwaita::Toast::new(
                                        "Copied synthesis to clipboard",
                                    ));
                                }
                            });
                            card_header.append(&copy_synth_btn);

                            synth_card.append(&card_header);

                            let synth_body = gtk4::Label::new(Some(synth));
                            synth_body.set_wrap(true);
                            synth_body.set_wrap_mode(gtk4::pango::WrapMode::Word);
                            synth_body.set_selectable(true);
                            synth_body.set_xalign(0.0);
                            synth_body.set_margin_start(14);
                            synth_body.set_margin_end(14);
                            synth_body.set_margin_bottom(14);
                            synth_card.append(&synth_body);

                            box_container.append(&synth_card);
                        }

                        // 2. Sources Section Header
                        let sources_header = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
                        sources_header.set_margin_top(8);
                        sources_header.set_margin_bottom(4);

                        let sources_title = gtk4::Label::new(Some(&format!(
                            "Sources & Evidence ({})",
                            sources_list.len()
                        )));
                        sources_title.add_css_class("heading");
                        sources_title.add_css_class("dim-label");
                        sources_title.set_halign(gtk4::Align::Start);
                        sources_header.append(&sources_title);
                        box_container.append(&sources_header);

                        // 3. Sources List Box (Boxed-list pattern)
                        let sources_list_box = gtk4::ListBox::new();
                        sources_list_box.add_css_class("boxed-list");
                        sources_list_box.set_selection_mode(gtk4::SelectionMode::None);

                        for (idx, source) in sources_list.iter().enumerate() {
                            let row = libadwaita::ActionRow::new();
                            row.set_title(&format!("{}. {}", idx + 1, source.title));
                            row.set_subtitle(&source.url);

                            let globe_icon = gtk4::Image::from_icon_name("globe-symbolic");
                            row.add_prefix(&globe_icon);

                            let actions_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);

                            // Open Link button
                            let open_btn = gtk4::Button::from_icon_name("external-link-symbolic");
                            open_btn.set_tooltip_text(Some("Open in Browser"));
                            open_btn.add_css_class("flat");
                            let url_open = source.url.clone();
                            open_btn.connect_clicked(move |_| {
                                let _ = opener::open(&url_open);
                            });
                            actions_row.append(&open_btn);

                            // Copy Link button
                            let copy_link_btn = gtk4::Button::from_icon_name("edit-copy-symbolic");
                            copy_link_btn.set_tooltip_text(Some("Copy URL"));
                            copy_link_btn.add_css_class("flat");
                            let url_copy = source.url.clone();
                            let toast_c2 = toast_weak.clone();
                            copy_link_btn.connect_clicked(move |_| {
                                if let Ok(mut cb) = arboard::Clipboard::new() {
                                    let _ = cb.set_text(&url_copy);
                                }
                                if let Some(t) = toast_c2.clone().into_weak_ref().upgrade() {
                                    t.add_toast(libadwaita::Toast::new("Copied URL to clipboard"));
                                }
                            });
                            actions_row.append(&copy_link_btn);

                            row.add_suffix(&actions_row);

                            // Expanded content snippet container
                            let row_container = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
                            row_container.append(&row);

                            if !source.content.is_empty() {
                                let snippet_lbl = gtk4::Label::new(Some(&source.content));
                                snippet_lbl.set_wrap(true);
                                snippet_lbl.set_wrap_mode(gtk4::pango::WrapMode::Word);
                                snippet_lbl.set_selectable(true);
                                snippet_lbl.set_xalign(0.0);
                                snippet_lbl.add_css_class("caption");
                                snippet_lbl.add_css_class("dim-label");
                                snippet_lbl.set_margin_start(42);
                                snippet_lbl.set_margin_end(16);
                                snippet_lbl.set_margin_bottom(10);
                                row_container.append(&snippet_lbl);
                            }

                            sources_list_box.append(&row_container);
                        }

                        box_container.append(&sources_list_box);
                    }

                    if let Some(st) = st_stack_weak.into_weak_ref().upgrade() {
                        st.set_visible_child_name("results");
                    }
                });
            });
        })
    };

    // Wire Search Triggers
    let search_act = do_search.clone();
    search_btn.connect_clicked(move |_| {
        search_act();
    });

    let search_enter = do_search.clone();
    search_entry.connect_activate(move |_| {
        search_enter();
    });

    // Wire Deep Research Toggle
    let w_title_deep = window_title.clone();
    let do_search_deep = do_search.clone();
    let s_entry_deep = search_entry.clone();
    deep_toggle.connect_toggled(move |btn| {
        let is_deep = btn.is_active();
        if is_deep {
            btn.add_css_class("accent");
            w_title_deep.set_subtitle("Deep Multi-Source Mode Active");
        } else {
            btn.remove_css_class("accent");
            w_title_deep.set_subtitle("Tavily & Firecrawl Intelligence");
        }
        if !s_entry_deep.text().trim().is_empty() {
            do_search_deep();
        }
    });

    // Wire Clear / Reset Button
    let s_entry_clear = search_entry.clone();
    let st_stack_clear = stack.clone();
    let c_btn_clear = copy_btn.clone();
    let s_note_clear = save_note_btn.clone();
    let w_title_clear = window_title.clone();
    let state_clear = research_state.clone();
    clear_btn.connect_clicked(move |_| {
        s_entry_clear.set_text("");
        st_stack_clear.set_visible_child_name("welcome");
        c_btn_clear.set_sensitive(false);
        s_note_clear.set_sensitive(false);
        w_title_clear.set_subtitle("Tavily & Firecrawl Intelligence");
        if let Ok(mut st) = state_clear.lock() {
            *st = ResearchState::default();
        }
        s_entry_clear.grab_focus();
    });

    // Wire Copy Results Button
    let toast_copy = toast_overlay.clone();
    let state_copy = research_state.clone();
    copy_btn.connect_clicked(move |_| {
        let md = state_copy
            .lock()
            .map(|st| st.full_markdown.clone())
            .unwrap_or_default();
        if !md.is_empty() {
            if let Ok(mut cb) = arboard::Clipboard::new() {
                let _ = cb.set_text(&md);
            }
            toast_copy.add_toast(libadwaita::Toast::new(
                "Copied full research findings as Markdown",
            ));
        }
    });

    // Wire Save to Notes Button
    let ctx_note = ctx.clone();
    let toast_note = toast_overlay.clone();
    let state_note = research_state.clone();
    save_note_btn.connect_clicked(move |_| {
        let (md, query) = if let Ok(st) = state_note.lock() {
            (st.full_markdown.clone(), st.query.clone())
        } else {
            (String::new(), String::new())
        };
        if !md.is_empty() {
            let note_title = if query.trim().is_empty() {
                "Research Findings".to_string()
            } else {
                format!("Research: {}", query.trim())
            };
            match ctx_note
                .history
                .save_note(note_title, md, Some("research,web,ai".to_string()))
            {
                Ok(_) => {
                    toast_note.add_toast(libadwaita::Toast::new("Saved findings to Quick Notes!"));
                }
                Err(e) => {
                    toast_note
                        .add_toast(libadwaita::Toast::new(&format!("Error saving note: {e}")));
                }
            }
        }
    });

    // Keyboard Shortcuts:
    // Escape = Close window
    // Ctrl+D = Toggle Deep Research
    // Ctrl+C = Copy Results as Markdown
    // Ctrl+S = Save to Notes
    // Ctrl+L = Focus search entry
    let key_controller = gtk4::EventControllerKey::new();
    let win_weak = glib::SendWeakRef::from(window.downgrade());
    let deep_key = deep_toggle.clone();
    let copy_key = copy_btn.clone();
    let save_key = save_note_btn.clone();
    let s_entry_key = search_entry.clone();

    key_controller.connect_key_pressed(move |_, key, _, state_mod| {
        if key == gdk4::Key::Escape {
            if let Some(w) = win_weak.clone().into_weak_ref().upgrade() {
                w.close();
                return glib::Propagation::Stop;
            }
        } else if state_mod.contains(gdk4::ModifierType::CONTROL_MASK) {
            match key {
                gdk4::Key::d | gdk4::Key::D => {
                    deep_key.set_active(!deep_key.is_active());
                    return glib::Propagation::Stop;
                }
                gdk4::Key::c | gdk4::Key::C => {
                    if copy_key.is_sensitive() {
                        copy_key.emit_clicked();
                        return glib::Propagation::Stop;
                    }
                }
                gdk4::Key::s | gdk4::Key::S => {
                    if save_key.is_sensitive() {
                        save_key.emit_clicked();
                        return glib::Propagation::Stop;
                    }
                }
                gdk4::Key::l | gdk4::Key::L => {
                    s_entry_key.grab_focus();
                    s_entry_key.select_region(0, -1);
                    return glib::Propagation::Stop;
                }
                _ => {}
            }
        }
        glib::Propagation::Proceed
    });
    window.add_controller(key_controller);

    window.connect_destroy(|_| {
        let mut guard = match SEARCH_WINDOW.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        *guard = None;
    });

    search_entry.grab_focus();
    window.present();
}
