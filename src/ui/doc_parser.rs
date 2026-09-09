//! Document Parser & Vision OCR modal (`Ctrl+Alt+D`).
//!
//! Native GTK4 + libadwaita multimodal document intelligence palette.
//! Extracts structured Markdown, tables, and OCR text from PDFs, scans, images,
//! and Word documents using Firecrawl v2 and Vision LLMs with drag & drop support.

use crate::context::AppContext;
use crate::settings;
use gdk4::gio;
use gdk4::prelude::*;
use gtk4::prelude::*;
use libadwaita::prelude::*;
use log::warn;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

static DOC_WINDOW: LazyLock<Mutex<Option<glib::SendWeakRef<libadwaita::Window>>>> =
    LazyLock::new(|| Mutex::new(None));
static LAST_DOC_TOGGLE: LazyLock<Mutex<Option<std::time::Instant>>> =
    LazyLock::new(|| Mutex::new(None));

/// Toggle or show document parser & OCR modal.
pub fn show_doc_parser(ctx: &AppContext) {
    toggle_doc_parser(ctx);
}

/// Toggle display of the document parser & OCR modal.
pub fn toggle_doc_parser(ctx: &AppContext) {
    let now = std::time::Instant::now();
    if let Ok(mut last) = LAST_DOC_TOGGLE.lock() {
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
            let mut guard = match DOC_WINDOW.lock() {
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
                    if let Ok(mut guard) = DOC_WINDOW.lock() {
                        *guard = Some(glib::SendWeakRef::from(win.downgrade()));
                    }
                }
                return;
            }
        }

        build_and_present_doc_parser(&ctx);
    });
}

#[derive(Clone, Default)]
struct DocParserState {
    file_path: Option<PathBuf>,
    file_name: String,
    file_size_str: String,
    doc_type: String,
    extracted_md: String,
    words: usize,
    chars: usize,
}

fn format_file_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.2} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

fn is_supported_document(ext: &str) -> bool {
    matches!(
        ext.to_lowercase().as_str(),
        "pdf" | "png" | "jpg" | "jpeg" | "docx" | "webp"
    )
}

fn get_icon_name_for_ext(ext: &str) -> &'static str {
    match ext.to_lowercase().as_str() {
        "pdf" => "x-office-document-symbolic",
        "png" | "jpg" | "jpeg" | "webp" => "image-x-generic-symbolic",
        "docx" => "x-office-document-symbolic",
        _ => "text-x-generic-symbolic",
    }
}

fn build_and_present_doc_parser(ctx: &AppContext) {
    let window = libadwaita::Window::new();
    window.set_title(Some("Document Parser & Vision OCR"));
    window.set_default_size(780, 580);
    window.set_modal(true);
    window.set_resizable(true);
    window.set_deletable(true);
    window.add_css_class("dialog");

    {
        let mut guard = match DOC_WINDOW.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        *guard = Some(glib::SendWeakRef::from(window.downgrade()));
    }

    let toast_overlay = libadwaita::ToastOverlay::new();
    let main_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    toast_overlay.set_child(Some(&main_box));
    window.set_content(Some(&toast_overlay));

    let doc_state = Arc::new(Mutex::new(DocParserState::default()));

    // ========================================================================
    // Header Bar
    // ========================================================================
    let header_bar = libadwaita::HeaderBar::new();
    header_bar.set_show_end_title_buttons(true);

    let window_title = libadwaita::WindowTitle::new(
        "Document Intelligence & OCR",
        "PDF, Scans, Images & DOCX to Markdown",
    );
    header_bar.set_title_widget(Some(&window_title));

    // Reset / Select Another Document (Left)
    let reset_btn = gtk4::Button::from_icon_name("view-refresh-symbolic");
    reset_btn.set_tooltip_text(Some("Select Another Document (Ctrl+O)"));
    reset_btn.add_css_class("flat");
    header_bar.pack_start(&reset_btn);

    // Open File Chooser Button (Right)
    let browse_btn = gtk4::Button::from_icon_name("folder-open-symbolic");
    browse_btn.set_tooltip_text(Some("Browse Document or Image…"));
    browse_btn.add_css_class("flat");
    header_bar.pack_end(&browse_btn);

    // Save to Documents Library Button (Right)
    let save_doc_btn = gtk4::Button::from_icon_name("emblem-documents-symbolic");
    save_doc_btn.set_tooltip_text(Some("Save to Documents Library (Ctrl+Shift+S)"));
    save_doc_btn.add_css_class("flat");
    save_doc_btn.set_sensitive(false);
    header_bar.pack_end(&save_doc_btn);

    // Save as Note Button (Right)
    let save_note_btn = gtk4::Button::from_icon_name("document-new-symbolic");
    save_note_btn.set_tooltip_text(Some("Save to Quick Notes (Ctrl+S)"));
    save_note_btn.add_css_class("flat");
    save_note_btn.set_sensitive(false);
    header_bar.pack_end(&save_note_btn);

    // Copy Markdown Button (Right)
    let copy_btn = gtk4::Button::from_icon_name("edit-copy-symbolic");
    copy_btn.set_tooltip_text(Some("Copy Markdown (Ctrl+C)"));
    copy_btn.add_css_class("flat");
    copy_btn.set_sensitive(false);
    header_bar.pack_end(&copy_btn);

    main_box.append(&header_bar);

    // Progress Bar below header
    let progress_bar = gtk4::ProgressBar::new();
    progress_bar.set_visible(false);
    progress_bar.add_css_class("osd");
    main_box.append(&progress_bar);

    // ========================================================================
    // Dynamic Content Stack
    // ========================================================================
    let stack = gtk4::Stack::new();
    stack.set_transition_type(gtk4::StackTransitionType::Crossfade);
    stack.set_transition_duration(200);
    stack.set_vexpand(true);
    stack.set_hexpand(true);

    // 1. Initial State: Drop Zone & Browse
    let drop_page = libadwaita::StatusPage::new();
    drop_page.set_icon_name(Some("scanner-symbolic"));
    drop_page.set_title("Document Intelligence &amp; OCR");
    drop_page.set_description(Some(
        "Drag &amp; drop or choose any PDF, scan, image, or Word document to extract structured Markdown.",
    ));

    let drop_box = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    drop_box.set_halign(gtk4::Align::Center);
    drop_box.set_margin_top(12);

    let choose_file_btn = gtk4::Button::new();
    let choose_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    let choose_icon = gtk4::Image::from_icon_name("folder-open-symbolic");
    let choose_label = gtk4::Label::new(Some("Choose Document or Image…"));
    choose_box.append(&choose_icon);
    choose_box.append(&choose_label);
    choose_file_btn.set_child(Some(&choose_box));
    choose_file_btn.add_css_class("suggested-action");
    choose_file_btn.add_css_class("pill");
    choose_file_btn.set_halign(gtk4::Align::Center);
    drop_box.append(&choose_file_btn);

    let badges_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    badges_box.set_halign(gtk4::Align::Center);
    badges_box.set_margin_top(8);

    for (icon, text) in [
        ("x-office-document-symbolic", "PDF Documents"),
        ("image-x-generic-symbolic", "Scans & Images (PNG, JPG)"),
        ("x-office-document-symbolic", "Word Documents (DOCX)"),
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

        badges_box.append(&pill);
    }
    drop_box.append(&badges_box);
    drop_page.set_child(Some(&drop_box));
    stack.add_named(&drop_page, Some("drop_zone"));

    // 2. File Ready State (Document staged and ready for extraction)
    let ready_clamp = libadwaita::Clamp::new();
    ready_clamp.set_maximum_size(520);
    ready_clamp.set_vexpand(true);
    ready_clamp.set_valign(gtk4::Align::Center);

    let ready_card = gtk4::Box::new(gtk4::Orientation::Vertical, 16);
    ready_card.add_css_class("card");
    ready_card.set_margin_start(20);
    ready_card.set_margin_end(20);
    ready_card.set_margin_top(20);
    ready_card.set_margin_bottom(20);

    let ready_icon = gtk4::Image::from_icon_name("x-office-document-symbolic");
    ready_icon.set_pixel_size(48);
    ready_icon.set_margin_top(8);
    ready_card.append(&ready_icon);

    let ready_title = gtk4::Label::new(Some("document.pdf"));
    ready_title.add_css_class("title-2");
    ready_title.set_ellipsize(gtk4::pango::EllipsizeMode::Middle);
    ready_card.append(&ready_title);

    let ready_meta = gtk4::Label::new(Some("PDF Document • 0 B"));
    ready_meta.add_css_class("dim-label");
    ready_meta.add_css_class("caption");
    ready_card.append(&ready_meta);

    let ready_engine_lbl = gtk4::Label::new(Some(
        "Extracts tables, headings, lists, and OCR text via Firecrawl v2 & Vision AI",
    ));
    ready_engine_lbl.add_css_class("caption");
    ready_engine_lbl.add_css_class("dim-label");
    ready_engine_lbl.set_wrap(true);
    ready_engine_lbl.set_justify(gtk4::Justification::Center);
    ready_card.append(&ready_engine_lbl);

    let extract_btn = gtk4::Button::new();
    let extract_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    let extract_icon = gtk4::Image::from_icon_name("scanner-symbolic");
    let extract_label = gtk4::Label::new(Some("Extract Structured Markdown"));
    extract_box.append(&extract_icon);
    extract_box.append(&extract_label);
    extract_btn.set_child(Some(&extract_box));
    extract_btn.add_css_class("suggested-action");
    extract_btn.add_css_class("pill");
    extract_btn.set_halign(gtk4::Align::Center);
    extract_btn.set_margin_top(8);
    ready_card.append(&extract_btn);

    let pick_different_btn = gtk4::Button::with_label("Choose Different File");
    pick_different_btn.add_css_class("flat");
    pick_different_btn.set_halign(gtk4::Align::Center);
    pick_different_btn.set_margin_bottom(8);
    ready_card.append(&pick_different_btn);

    ready_clamp.set_child(Some(&ready_card));
    stack.add_named(&ready_clamp, Some("file_ready"));

    // 3. Parsing / Extraction State
    let parsing_page = libadwaita::StatusPage::new();
    parsing_page.set_title("Extracting Document Content…");
    parsing_page.set_description(Some(
        "Parsing pages, tables, and recognizing text with Vision OCR…",
    ));
    let parsing_spinner = gtk4::Spinner::new();
    parsing_spinner.set_size_request(40, 40);
    parsing_spinner.set_halign(gtk4::Align::Center);
    parsing_spinner.set_valign(gtk4::Align::Center);
    parsing_page.set_child(Some(&parsing_spinner));
    stack.add_named(&parsing_page, Some("parsing"));

    // 4. Extracted Results Canvas
    let extracted_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    extracted_box.set_vexpand(true);
    extracted_box.set_hexpand(true);

    let stats_strip = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    stats_strip.set_margin_start(20);
    stats_strip.set_margin_end(20);
    stats_strip.set_margin_top(8);
    stats_strip.set_margin_bottom(6);

    let doc_badge_lbl = gtk4::Label::new(Some("document.pdf"));
    doc_badge_lbl.add_css_class("heading");
    stats_strip.append(&doc_badge_lbl);

    let stats_spacer = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    stats_spacer.set_hexpand(true);
    stats_strip.append(&stats_spacer);

    let stats_lbl = gtk4::Label::new(Some("0 words • 0 chars"));
    stats_lbl.add_css_class("caption");
    stats_lbl.add_css_class("dim-label");
    stats_strip.append(&stats_lbl);

    extracted_box.append(&stats_strip);

    let canvas_scrolled = gtk4::ScrolledWindow::new();
    canvas_scrolled.set_vexpand(true);
    canvas_scrolled.set_hexpand(true);

    let text_view = gtk4::TextView::new();
    text_view.set_editable(true);
    text_view.set_wrap_mode(gtk4::WrapMode::Word);
    text_view.set_left_margin(24);
    text_view.set_right_margin(24);
    text_view.set_top_margin(18);
    text_view.set_bottom_margin(18);
    canvas_scrolled.set_child(Some(&text_view));
    extracted_box.append(&canvas_scrolled);

    stack.add_named(&extracted_box, Some("extracted"));
    stack.set_visible_child_name("drop_zone");

    main_box.append(&stack);

    // ========================================================================
    // File Selection Logic
    // ========================================================================
    let set_selected_file = {
        let state_ref = doc_state.clone();
        let stack_ref = stack.clone();
        let ready_icon_ref = ready_icon.clone();
        let ready_title_ref = ready_title.clone();
        let ready_meta_ref = ready_meta.clone();
        let doc_badge_ref = doc_badge_lbl.clone();
        let w_title_ref = window_title.clone();

        Rc::new(move |path: PathBuf| {
            let filename = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("document")
                .to_string();

            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("pdf")
                .to_lowercase();

            let file_size_bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            let size_str = format_file_size(file_size_bytes);

            ready_icon_ref.set_icon_name(Some(get_icon_name_for_ext(&ext)));
            ready_title_ref.set_text(&filename);
            ready_meta_ref.set_text(&format!("{size_str} • {} document", ext.to_uppercase()));
            doc_badge_ref.set_text(&filename);
            w_title_ref.set_subtitle(&format!("{filename} • {size_str}"));

            if let Ok(mut st) = state_ref.lock() {
                st.file_path = Some(path);
                st.file_name = filename;
                st.file_size_str = size_str;
                st.doc_type = ext;
            }

            stack_ref.set_visible_child_name("file_ready");
        })
    };

    // Native File Chooser trigger
    let trigger_file_chooser = {
        let win_weak = glib::SendWeakRef::from(window.downgrade());
        let set_file = set_selected_file.clone();

        Rc::new(move || {
            let win = win_weak.clone().into_weak_ref().upgrade();
            let file_chooser = gtk4::FileChooserNative::new(
                Some("Select Document or Image to Parse"),
                win.as_ref(),
                gtk4::FileChooserAction::Open,
                Some("Open"),
                Some("Cancel"),
            );

            let filter = gtk4::FileFilter::new();
            filter.set_name(Some(
                "Documents & Scans (*.pdf, *.png, *.jpg, *.jpeg, *.docx, *.webp)",
            ));
            filter.add_mime_type("application/pdf");
            filter.add_mime_type("image/png");
            filter.add_mime_type("image/jpeg");
            filter.add_mime_type("image/webp");
            filter.add_mime_type(
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            );
            filter.add_pattern("*.pdf");
            filter.add_pattern("*.png");
            filter.add_pattern("*.jpg");
            filter.add_pattern("*.jpeg");
            filter.add_pattern("*.docx");
            filter.add_pattern("*.webp");
            file_chooser.add_filter(&filter);

            let on_select = set_file.clone();
            file_chooser.connect_response(move |chooser, response| {
                if response == gtk4::ResponseType::Accept {
                    if let Some(file) = chooser.file() {
                        if let Some(path) = file.path() {
                            on_select(path);
                        }
                    }
                }
            });

            file_chooser.show();
        })
    };

    // Wire file chooser buttons
    let fc1 = trigger_file_chooser.clone();
    choose_file_btn.connect_clicked(move |_| fc1());

    let fc2 = trigger_file_chooser.clone();
    browse_btn.connect_clicked(move |_| fc2());

    let fc3 = trigger_file_chooser.clone();
    pick_different_btn.connect_clicked(move |_| fc3());

    // Wire Native Drag & Drop onto Window
    let drop_target = gtk4::DropTarget::new(gio::File::static_type(), gdk4::DragAction::COPY);
    let on_drop = set_selected_file.clone();
    drop_target.connect_drop(move |_, value, _, _| {
        if let Ok(file) = value.get::<gio::File>() {
            if let Some(path) = file.path() {
                let ext = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or_default();
                if is_supported_document(ext) {
                    on_drop(path);
                    return true;
                }
            }
        }
        false
    });
    window.add_controller(drop_target);

    // ========================================================================
    // Extraction Execution Logic
    // ========================================================================
    let do_extract = {
        let ctx = ctx.clone();
        let state_ref = doc_state.clone();
        let stack_ref = stack.clone();
        let progress_ref = progress_bar.clone();
        let spinner_ref = parsing_spinner.clone();
        let w_title_ref = window_title.clone();
        let tv_ref = text_view.clone();
        let stats_lbl_ref = stats_lbl.clone();
        let c_btn_ref = copy_btn.clone();
        let s_note_ref = save_note_btn.clone();
        let s_doc_ref = save_doc_btn.clone();
        let toast_ref = toast_overlay.clone();

        Rc::new(move || {
            let (path_opt, filename) = if let Ok(st) = state_ref.lock() {
                (st.file_path.clone(), st.file_name.clone())
            } else {
                (None, String::new())
            };

            let path = match path_opt {
                Some(p) => p,
                None => return,
            };

            let file_bytes = match std::fs::read(&path) {
                Ok(b) => b,
                Err(e) => {
                    toast_ref
                        .add_toast(libadwaita::Toast::new(&format!("Error reading file: {e}")));
                    return;
                }
            };

            // Transition to parsing state
            stack_ref.set_visible_child_name("parsing");
            progress_ref.set_visible(true);
            progress_ref.pulse();
            spinner_ref.start();
            w_title_ref.set_subtitle(&format!("Extracting {filename}…"));

            let settings = settings::get_settings(&ctx);
            let firecrawl_key = settings
                .web_api_keys
                .get("firecrawl")
                .cloned()
                .unwrap_or_default();
            let firecrawl_base = settings
                .web_providers
                .iter()
                .find(|p| p.id == "firecrawl")
                .map(|p| (p.base_url.clone(), p.timeout_seconds))
                .unwrap_or_else(|| ("https://api.firecrawl.dev/v2".to_string(), 120));

            let filename_str = filename.clone();
            let state_weak = state_ref.clone();
            let stack_weak = glib::SendWeakRef::from(stack_ref.downgrade());
            let prog_weak = glib::SendWeakRef::from(progress_ref.downgrade());
            let spin_weak = glib::SendWeakRef::from(spinner_ref.downgrade());
            let w_title_weak = glib::SendWeakRef::from(w_title_ref.downgrade());
            let tv_weak = glib::SendWeakRef::from(tv_ref.downgrade());
            let stats_weak = glib::SendWeakRef::from(stats_lbl_ref.downgrade());
            let c_weak = glib::SendWeakRef::from(c_btn_ref.downgrade());
            let sn_weak = glib::SendWeakRef::from(s_note_ref.downgrade());
            let sd_weak = glib::SendWeakRef::from(s_doc_ref.downgrade());
            let toast_weak = glib::SendWeakRef::from(toast_ref.downgrade());

            crate::runtime::spawn(async move {
                let mut extracted_md = String::new();

                // 1. Try Firecrawl /parse
                if !firecrawl_key.trim().is_empty() {
                    let timeout = crate::web_client::clamp_timeout_secs(firecrawl_base.1 as u64);
                    match crate::web_client::firecrawl_parse_document_with_timeout(
                        &firecrawl_base.0,
                        &firecrawl_key,
                        &filename_str,
                        file_bytes.clone(),
                        timeout.max(std::time::Duration::from_secs(60)),
                    )
                    .await
                    {
                        Ok(md) => {
                            extracted_md = md;
                        }
                        Err(e) => {
                            warn!("Firecrawl parse failed: {e}");
                        }
                    }
                }

                // 2. Multimodal Vision OCR fallback for images
                if extracted_md.is_empty()
                    && (filename_str.ends_with(".png")
                        || filename_str.ends_with(".jpg")
                        || filename_str.ends_with(".jpeg")
                        || filename_str.ends_with(".webp"))
                {
                    if let Some(p) = settings
                        .post_process_providers
                        .iter()
                        .find(|p| p.id == "openai" || p.id == "gemini")
                    {
                        let key = settings
                            .post_process_api_keys
                            .get(&p.id)
                            .cloned()
                            .unwrap_or_default();
                        if !key.trim().is_empty() {
                            let mime = if filename_str.ends_with(".png") {
                                "image/png"
                            } else if filename_str.ends_with(".webp") {
                                "image/webp"
                            } else {
                                "image/jpeg"
                            };
                            let model = if p.id == "gemini" {
                                "gemini-2.0-flash"
                            } else {
                                "gpt-4o"
                            };
                            match crate::llm_client::send_vision_ocr(
                                p,
                                key,
                                model,
                                &file_bytes,
                                mime,
                                None,
                            )
                            .await
                            {
                                Ok(text) => {
                                    extracted_md = text;
                                }
                                Err(e) => {
                                    warn!("Vision OCR failed: {e}");
                                }
                            }
                        }
                    }
                }

                if extracted_md.is_empty() {
                    extracted_md = "Could not extract document content. Please verify that Firecrawl or a Vision LLM (OpenAI/Gemini) is configured with an active API key in Cloud Providers.".to_string();
                }

                glib::MainContext::default().invoke(move || {
                    if let Some(p) = prog_weak.into_weak_ref().upgrade() {
                        p.set_visible(false);
                    }
                    if let Some(s) = spin_weak.into_weak_ref().upgrade() {
                        s.stop();
                    }

                    let words = extracted_md.split_whitespace().count();
                    let chars = extracted_md.chars().count();
                    let word_str = if words == 1 {
                        "1 word"
                    } else {
                        &format!("{words} words")
                    };
                    let char_str = if chars == 1 {
                        "1 char"
                    } else {
                        &format!("{chars} chars")
                    };

                    if let Ok(mut st) = state_weak.lock() {
                        st.extracted_md = extracted_md.clone();
                        st.words = words;
                        st.chars = chars;
                    }

                    if let Some(tv) = tv_weak.into_weak_ref().upgrade() {
                        tv.buffer().set_text(&extracted_md);
                    }

                    if let Some(sl) = stats_weak.into_weak_ref().upgrade() {
                        sl.set_text(&format!("{word_str} • {char_str}"));
                    }

                    if let Some(wt) = w_title_weak.into_weak_ref().upgrade() {
                        wt.set_subtitle(&format!("Extracted {word_str} • {char_str}"));
                    }

                    // Enable actions
                    if let Some(cb) = c_weak.into_weak_ref().upgrade() {
                        cb.set_sensitive(true);
                    }
                    if let Some(sn) = sn_weak.into_weak_ref().upgrade() {
                        sn.set_sensitive(true);
                    }
                    if let Some(sd) = sd_weak.into_weak_ref().upgrade() {
                        sd.set_sensitive(true);
                    }

                    if let Some(stk) = stack_weak.into_weak_ref().upgrade() {
                        stk.set_visible_child_name("extracted");
                    }

                    if let Some(t) = toast_weak.into_weak_ref().upgrade() {
                        t.add_toast(libadwaita::Toast::new("Document extraction completed!"));
                    }
                });
            });
        })
    };

    let extract_act = do_extract.clone();
    extract_btn.connect_clicked(move |_| extract_act());

    // Reset button
    let stack_reset = stack.clone();
    let state_reset = doc_state.clone();
    let w_title_reset = window_title.clone();
    let c_reset = copy_btn.clone();
    let sn_reset = save_note_btn.clone();
    let sd_reset = save_doc_btn.clone();
    reset_btn.connect_clicked(move |_| {
        stack_reset.set_visible_child_name("drop_zone");
        w_title_reset.set_subtitle("PDF, Scans, Images & DOCX to Markdown");
        c_reset.set_sensitive(false);
        sn_reset.set_sensitive(false);
        sd_reset.set_sensitive(false);
        if let Ok(mut st) = state_reset.lock() {
            *st = DocParserState::default();
        }
    });

    // Copy Markdown button
    let state_copy = doc_state.clone();
    let toast_copy = toast_overlay.clone();
    copy_btn.connect_clicked(move |_| {
        let md = state_copy
            .lock()
            .map(|st| st.extracted_md.clone())
            .unwrap_or_default();
        if !md.is_empty() {
            if let Ok(mut cb) = arboard::Clipboard::new() {
                let _ = cb.set_text(&md);
            }
            toast_copy.add_toast(libadwaita::Toast::new("Copied Markdown to clipboard"));
        }
    });

    // Save as Note button
    let ctx_note = ctx.clone();
    let state_note = doc_state.clone();
    let toast_note = toast_overlay.clone();
    save_note_btn.connect_clicked(move |_| {
        let (md, filename) = if let Ok(st) = state_note.lock() {
            (st.extracted_md.clone(), st.file_name.clone())
        } else {
            (String::new(), String::new())
        };

        if !md.is_empty() {
            let title = format!("Doc OCR: {filename}");
            match ctx_note
                .history
                .save_note(title, md, Some("document,ocr,ai".to_string()))
            {
                Ok(_) => {
                    toast_note.add_toast(libadwaita::Toast::new("Saved document to Quick Notes!"));
                }
                Err(e) => {
                    toast_note
                        .add_toast(libadwaita::Toast::new(&format!("Error saving note: {e}")));
                }
            }
        }
    });

    // Save to Docs Library button
    let ctx_doc = ctx.clone();
    let state_doc = doc_state.clone();
    let toast_doc = toast_overlay.clone();
    save_doc_btn.connect_clicked(move |_| {
        let (md, filename, doc_type) = if let Ok(st) = state_doc.lock() {
            (
                st.extracted_md.clone(),
                st.file_name.clone(),
                st.doc_type.clone(),
            )
        } else {
            (String::new(), String::new(), "doc".to_string())
        };

        if !md.is_empty() {
            match ctx_doc
                .history
                .save_doc(filename.clone(), filename, md, doc_type)
            {
                Ok(_) => {
                    toast_doc.add_toast(libadwaita::Toast::new("Saved to Documents Library!"));
                }
                Err(e) => {
                    toast_doc.add_toast(libadwaita::Toast::new(&format!("Error saving doc: {e}")));
                }
            }
        }
    });

    // Keyboard Shortcuts:
    // Ctrl+O = Open File Chooser
    // Enter / Ctrl+Return = Extract
    // Ctrl+C = Copy Markdown
    // Ctrl+S = Save to Notes
    // Ctrl+Shift+S = Save to Docs Library
    // Escape = Close
    let key_controller = gtk4::EventControllerKey::new();
    let win_weak = glib::SendWeakRef::from(window.downgrade());
    let fc_key = trigger_file_chooser.clone();
    let extract_key = do_extract.clone();
    let stack_key = stack.clone();
    let c_key = copy_btn.clone();
    let sn_key = save_note_btn.clone();
    let sd_key = save_doc_btn.clone();

    key_controller.connect_key_pressed(move |_, key, _, state_mod| {
        if key == gdk4::Key::Escape {
            if let Some(w) = win_weak.clone().into_weak_ref().upgrade() {
                w.close();
                return glib::Propagation::Stop;
            }
        } else if state_mod.contains(gdk4::ModifierType::CONTROL_MASK) {
            if state_mod.contains(gdk4::ModifierType::SHIFT_MASK) {
                if (key == gdk4::Key::s || key == gdk4::Key::S) && sd_key.is_sensitive() {
                    sd_key.emit_clicked();
                    return glib::Propagation::Stop;
                }
            } else {
                match key {
                    gdk4::Key::o | gdk4::Key::O => {
                        fc_key();
                        return glib::Propagation::Stop;
                    }
                    gdk4::Key::c | gdk4::Key::C => {
                        if c_key.is_sensitive() {
                            c_key.emit_clicked();
                            return glib::Propagation::Stop;
                        }
                    }
                    gdk4::Key::s | gdk4::Key::S => {
                        if sn_key.is_sensitive() {
                            sn_key.emit_clicked();
                            return glib::Propagation::Stop;
                        }
                    }
                    gdk4::Key::Return
                        if stack_key.visible_child_name().as_deref() == Some("file_ready") =>
                    {
                        extract_key();
                        return glib::Propagation::Stop;
                    }
                    _ => {}
                }
            }
        } else if key == gdk4::Key::Return
            && stack_key.visible_child_name().as_deref() == Some("file_ready")
        {
            extract_key();
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    window.add_controller(key_controller);

    window.connect_destroy(|_| {
        let mut guard = match DOC_WINDOW.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        *guard = None;
    });

    window.present();
}
