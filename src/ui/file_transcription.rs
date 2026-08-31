//! File transcription dialog with drag & drop support, live progress, and multi-format exports.

use crate::commands::transcription as stt_cmds;
use crate::context::AppContext;
use gdk4::prelude::*;
use gtk4::prelude::*;
use libadwaita::prelude::*;
use log::{error, warn};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Present the file transcription dialog for a given file (or open file picker if None).
pub fn show_file_transcription_dialog(ctx: &AppContext, initial_file: Option<PathBuf>) {
    let ctx_clone = ctx.clone();
    glib::MainContext::default().invoke(move || {
        build_and_present_dialog(&ctx_clone, initial_file);
    });
}

fn build_and_present_dialog(ctx: &AppContext, initial_file: Option<PathBuf>) {
    let window = libadwaita::Window::new();
    window.set_title(Some("Transcribe Audio/Video File"));
    window.set_default_size(580, 480);
    window.set_modal(true);
    window.set_resizable(true);
    window.add_css_class("dialog");

    let toast_overlay = libadwaita::ToastOverlay::new();
    let main_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    toast_overlay.set_child(Some(&main_box));
    window.set_content(Some(&toast_overlay));

    // Header bar
    let header_bar = libadwaita::HeaderBar::new();
    header_bar.set_show_end_title_buttons(true);
    header_bar.set_title_widget(Some(&gtk4::Label::new(Some("Transcribe Media File"))));
    main_box.append(&header_bar);

    let scrolled = gtk4::ScrolledWindow::new();
    scrolled.set_vexpand(true);
    scrolled.set_hexpand(true);
    main_box.append(&scrolled);

    let content_box = gtk4::Box::new(gtk4::Orientation::Vertical, 16);
    content_box.set_margin_start(24);
    content_box.set_margin_end(24);
    content_box.set_margin_top(16);
    content_box.set_margin_bottom(24);
    scrolled.set_child(Some(&content_box));

    // 1. File Selection Group / Drop Box
    let file_group = libadwaita::PreferencesGroup::new();
    file_group.set_title("Media File");
    file_group.set_description(Some(
        "Supported formats: MP3, WAV, M4A, MP4, FLAC, OGG, AAC, WebM, MKV.",
    ));
    content_box.append(&file_group);

    let file_row = libadwaita::ActionRow::new();
    file_row.set_title("Selected File");
    file_row.set_subtitle(
        initial_file
            .as_ref()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("No file selected — drag and drop or browse below"),
    );

    let browse_button = gtk4::Button::from_icon_name("document-open-symbolic");
    browse_button.set_tooltip_text(Some("Browse for media file…"));
    browse_button.set_valign(gtk4::Align::Center);
    browse_button.add_css_class("flat");
    file_row.add_suffix(&browse_button);
    file_group.add(&file_row);

    // Selected file state
    let selected_file_path: Arc<Mutex<Option<PathBuf>>> = Arc::new(Mutex::new(initial_file));

    // 2. Options Group (AI Post-Processing)
    let options_group = libadwaita::PreferencesGroup::new();
    options_group.set_title("Processing Options");
    content_box.append(&options_group);

    let settings = ctx.settings();
    let prompts = settings.post_process_prompts.clone();
    let prompt_strings: Vec<String> = std::iter::once("None (Raw Transcription)".to_string())
        .chain(prompts.iter().map(|p| p.name.clone()))
        .collect();
    let prompt_str_refs: Vec<&str> = prompt_strings.iter().map(|s| s.as_str()).collect();

    let prompt_model = gtk4::StringList::new(&prompt_str_refs);
    let prompt_combo = libadwaita::ComboRow::new();
    prompt_combo.set_title("AI Post-Processing");
    prompt_combo.set_subtitle("Apply formatting, summary, or translation via LLM");
    prompt_combo.set_model(Some(&prompt_model));
    prompt_combo.set_selected(0);
    options_group.add(&prompt_combo);

    // 3. Progress & Status
    let progress_bar = gtk4::ProgressBar::new();
    progress_bar.set_show_text(true);
    progress_bar.set_fraction(0.0);
    progress_bar.set_visible(false);
    content_box.append(&progress_bar);

    let status_label = gtk4::Label::new(None);
    status_label.add_css_class("dim-label");
    status_label.set_visible(false);
    content_box.append(&status_label);

    // 4. Action buttons (Start / Transcribe)
    let action_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
    action_box.set_halign(gtk4::Align::End);
    content_box.append(&action_box);

    let start_button = gtk4::Button::with_label("Start Transcription");
    start_button.add_css_class("suggested-action");
    action_box.append(&start_button);

    // 5. Results Section (Initially hidden)
    let results_group = libadwaita::PreferencesGroup::new();
    results_group.set_title("Transcription Result");
    results_group.set_visible(false);
    content_box.append(&results_group);

    let result_text_view = gtk4::TextView::new();
    result_text_view.set_wrap_mode(gtk4::WrapMode::Word);
    result_text_view.set_editable(false);
    result_text_view.set_cursor_visible(false);
    result_text_view.set_margin_start(8);
    result_text_view.set_margin_end(8);
    result_text_view.set_margin_top(8);
    result_text_view.set_margin_bottom(8);

    let result_scrolled = gtk4::ScrolledWindow::new();
    result_scrolled.set_min_content_height(140);
    result_scrolled.set_max_content_height(240);
    result_scrolled.set_child(Some(&result_text_view));

    let result_frame = gtk4::Frame::new(None);
    result_frame.set_child(Some(&result_scrolled));
    results_group.add(&result_frame);

    // Export Buttons Bar
    let export_bar = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    export_bar.set_margin_top(12);
    export_bar.set_halign(gtk4::Align::End);
    results_group.add(&export_bar);

    let copy_btn = gtk4::Button::with_label("Copy Text");
    copy_btn.set_icon_name("edit-copy-symbolic");
    export_bar.append(&copy_btn);

    let srt_btn = gtk4::Button::with_label("Save .SRT");
    export_bar.append(&srt_btn);

    let vtt_btn = gtk4::Button::with_label("Save .VTT");
    export_bar.append(&vtt_btn);

    let txt_btn = gtk4::Button::with_label("Save .TXT");
    export_bar.append(&txt_btn);

    let json_btn = gtk4::Button::with_label("Save .JSON");
    export_bar.append(&json_btn);

    let md_btn = gtk4::Button::with_label("Save .MD");
    export_bar.append(&md_btn);

    // Current transcription document state for exporting
    let current_document: Arc<Mutex<Option<crate::audio_toolkit::TranscriptDocument>>> =
        Arc::new(Mutex::new(None));

    // Wire Browse File Button
    let win_weak_browse = glib::SendWeakRef::from(window.downgrade());
    let file_path_for_browse = selected_file_path.clone();
    let file_row_weak = glib::SendWeakRef::from(file_row.downgrade());

    browse_button.connect_clicked(move |_| {
        let win = win_weak_browse.clone().into_weak_ref().upgrade();
        let file_chooser = gtk4::FileChooserNative::new(
            Some("Select Audio or Video File"),
            win.as_ref(),
            gtk4::FileChooserAction::Open,
            Some("Open"),
            Some("Cancel"),
        );

        let filter = gtk4::FileFilter::new();
        filter.set_name(Some(
            "Media Files (*.mp3, *.wav, *.m4a, *.mp4, *.flac, *.ogg, *.aac)",
        ));
        filter.add_mime_type("audio/*");
        filter.add_mime_type("video/*");
        filter.add_pattern("*.mp3");
        filter.add_pattern("*.wav");
        filter.add_pattern("*.m4a");
        filter.add_pattern("*.mp4");
        filter.add_pattern("*.flac");
        filter.add_pattern("*.ogg");
        filter.add_pattern("*.aac");
        filter.add_pattern("*.webm");
        filter.add_pattern("*.mkv");
        file_chooser.add_filter(&filter);

        let file_path_clone = file_path_for_browse.clone();
        let file_row_clone = file_row_weak.clone();

        file_chooser.connect_response(move |chooser, response| {
            if response == gtk4::ResponseType::Accept {
                if let Some(file) = chooser.file() {
                    if let Some(path) = file.path() {
                        if let Some(row) = file_row_clone.clone().into_weak_ref().upgrade() {
                            let name = path
                                .file_name()
                                .and_then(|n| n.to_str())
                                .unwrap_or("media file");
                            row.set_subtitle(name);
                        }
                        *file_path_clone.lock().unwrap() = Some(path);
                    }
                }
            }
        });

        file_chooser.show();
    });

    // Wire Drag & Drop onto Window
    let drop_target = gtk4::DropTarget::new(gio::File::static_type(), gdk4::DragAction::COPY);
    let file_path_drop = selected_file_path.clone();
    let file_row_drop = glib::SendWeakRef::from(file_row.downgrade());

    drop_target.connect_drop(move |_, value, _, _| {
        if let Ok(file) = value.get::<gio::File>() {
            if let Some(path) = file.path() {
                let ext = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|s| s.to_lowercase())
                    .unwrap_or_default();
                let supported = matches!(
                    ext.as_str(),
                    "mp3" | "wav" | "m4a" | "mp4" | "flac" | "ogg" | "aac" | "webm" | "mkv"
                );
                if supported {
                    let name = path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("media file")
                        .to_string();
                    if let Some(row) = file_row_drop.clone().into_weak_ref().upgrade() {
                        row.set_subtitle(&name);
                    }
                    *file_path_drop.lock().unwrap() = Some(path);
                    return true;
                }
            }
        }
        false
    });
    window.add_controller(drop_target);

    // Wire Start Button
    let ctx_start = ctx.clone();
    let file_path_start = selected_file_path.clone();
    let prompts_list = prompts.clone();
    let combo_weak = glib::SendWeakRef::from(prompt_combo.downgrade());
    let progress_weak = glib::SendWeakRef::from(progress_bar.downgrade());
    let status_weak = glib::SendWeakRef::from(status_label.downgrade());
    let start_weak = glib::SendWeakRef::from(start_button.downgrade());
    let results_weak = glib::SendWeakRef::from(results_group.downgrade());
    let textview_weak = glib::SendWeakRef::from(result_text_view.downgrade());
    let doc_store = current_document.clone();
    let toast_weak = glib::SendWeakRef::from(toast_overlay.downgrade());

    start_button.connect_clicked(move |_| {
        let path_opt = file_path_start.lock().unwrap().clone();
        let Some(path) = path_opt else {
            if let Some(toast_ov) = toast_weak.clone().into_weak_ref().upgrade() {
                toast_ov.add_toast(libadwaita::Toast::new("Please choose a file to transcribe"));
            }
            return;
        };

        let selected_idx = combo_weak
            .clone()
            .into_weak_ref()
            .upgrade()
            .map(|c| c.selected())
            .unwrap_or(0) as usize;

        let prompt_id = if selected_idx > 0 && selected_idx <= prompts_list.len() {
            Some(prompts_list[selected_idx - 1].id.clone())
        } else {
            None
        };

        if let Some(btn) = start_weak.clone().into_weak_ref().upgrade() {
            btn.set_sensitive(false);
        }
        if let Some(pbar) = progress_weak.clone().into_weak_ref().upgrade() {
            pbar.set_visible(true);
            pbar.set_fraction(0.05);
            pbar.set_text(Some("0%"));
        }
        if let Some(lbl) = status_weak.clone().into_weak_ref().upgrade() {
            lbl.set_visible(true);
            lbl.set_text("Decoding media file…");
        }
        if let Some(res) = results_weak.clone().into_weak_ref().upgrade() {
            res.set_visible(false);
        }

        let ctx = ctx_start.clone();
        let pbar_for_cb = progress_weak.clone();
        let status_for_cb = status_weak.clone();

        let progress_cb: Arc<dyn Fn(f32) + Send + Sync> = Arc::new(move |frac| {
            let pbar_weak = pbar_for_cb.clone();
            let status_weak = status_for_cb.clone();
            glib::MainContext::default().invoke(move || {
                if let Some(pbar) = pbar_weak.into_weak_ref().upgrade() {
                    pbar.set_fraction(frac as f64);
                    pbar.set_text(Some(&format!("{:.0}%", frac * 100.0)));
                }
                if let Some(lbl) = status_weak.into_weak_ref().upgrade() {
                    if frac < 0.3 {
                        lbl.set_text("Decoding audio track…");
                    } else if frac < 0.95 {
                        lbl.set_text("Transcribing speech with AI model…");
                    } else {
                        lbl.set_text("Finalizing transcript…");
                    }
                }
            });
        });

        let start_weak_fin = start_weak.clone();
        let progress_weak_fin = progress_weak.clone();
        let status_weak_fin = status_weak.clone();
        let results_weak_fin = results_weak.clone();
        let textview_weak_fin = textview_weak.clone();
        let doc_store_fin = doc_store.clone();
        let toast_weak_fin = toast_weak.clone();

        crate::runtime::spawn(async move {
            let result =
                stt_cmds::transcribe_media_file(&ctx, &path, prompt_id, Some(progress_cb)).await;

            glib::MainContext::default().invoke(move || {
                if let Some(btn) = start_weak_fin.into_weak_ref().upgrade() {
                    btn.set_sensitive(true);
                }
                if let Some(pbar) = progress_weak_fin.into_weak_ref().upgrade() {
                    pbar.set_fraction(1.0);
                    pbar.set_text(Some("100%"));
                }
                if let Some(lbl) = status_weak_fin.into_weak_ref().upgrade() {
                    lbl.set_text("Completed!");
                }

                match result {
                    Ok(doc) => {
                        let display_text = if let Some(ref post) = doc.summary_or_post_processed {
                            post.clone()
                        } else {
                            doc.segments
                                .iter()
                                .map(|s| s.text.as_str())
                                .collect::<Vec<_>>()
                                .join(" ")
                        };

                        if let Some(tv) = textview_weak_fin.into_weak_ref().upgrade() {
                            tv.buffer().set_text(&display_text);
                        }
                        if let Some(res) = results_weak_fin.into_weak_ref().upgrade() {
                            res.set_visible(true);
                        }
                        *doc_store_fin.lock().unwrap() = Some(doc);

                        if let Some(toast_ov) = toast_weak_fin.into_weak_ref().upgrade() {
                            toast_ov.add_toast(libadwaita::Toast::new(
                                "Transcription finished and saved to History",
                            ));
                        }
                    }
                    Err(e) => {
                        error!("File transcription failed: {e}");
                        if let Some(toast_ov) = toast_weak_fin.into_weak_ref().upgrade() {
                            toast_ov.add_toast(libadwaita::Toast::new(&format!(
                                "Transcription failed: {}",
                                e
                            )));
                        }
                    }
                }
            });
        });
    });

    // Wire Copy Button
    let doc_copy = current_document.clone();
    let ctx_copy = ctx.clone();
    let toast_copy = toast_overlay.clone();
    copy_btn.connect_clicked(move |_| {
        if let Some(doc) = doc_copy.lock().unwrap().as_ref() {
            let text = if let Some(ref post) = doc.summary_or_post_processed {
                post.clone()
            } else {
                crate::audio_toolkit::export_to_txt(&doc.segments, false)
            };
            if let Err(e) = crate::clipboard::write_clipboard_text(&ctx_copy, &text) {
                warn!("Failed to copy to clipboard: {e}");
            } else {
                toast_copy.add_toast(libadwaita::Toast::new("Transcript copied to clipboard"));
            }
        }
    });

    // Helper for export buttons
    let setup_export_button =
        |btn: &gtk4::Button,
         ext: &'static str,
         doc_store: Arc<Mutex<Option<crate::audio_toolkit::TranscriptDocument>>>,
         parent_win: glib::SendWeakRef<libadwaita::Window>,
         toast: libadwaita::ToastOverlay| {
            btn.connect_clicked(move |_| {
                let doc_opt = doc_store.lock().unwrap().clone();
                let Some(doc) = doc_opt else { return };

                let content = match ext {
                    "srt" => crate::audio_toolkit::export_to_srt(&doc.segments),
                    "vtt" => crate::audio_toolkit::export_to_vtt(&doc.segments),
                    "txt" => {
                        if let Some(ref post) = doc.summary_or_post_processed {
                            post.clone()
                        } else {
                            crate::audio_toolkit::export_to_txt(&doc.segments, false)
                        }
                    }
                    "json" => crate::audio_toolkit::export_to_json(&doc).unwrap_or_default(),
                    "md" => crate::audio_toolkit::export_to_markdown(&doc),
                    _ => String::new(),
                };

                let win = parent_win.clone().into_weak_ref().upgrade();
                let chooser = gtk4::FileChooserNative::new(
                    Some(&format!(
                        "Save .{} Subtitles/Transcript",
                        ext.to_uppercase()
                    )),
                    win.as_ref(),
                    gtk4::FileChooserAction::Save,
                    Some("Save"),
                    Some("Cancel"),
                );

                let base_name = Path::new(&doc.title)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("transcript");
                chooser.set_current_name(&format!("{}.{}", base_name, ext));

                let toast_clone = toast.clone();
                chooser.connect_response(move |dialog, response| {
                    if response == gtk4::ResponseType::Accept {
                        if let Some(file) = dialog.file() {
                            if let Some(target_path) = file.path() {
                                if let Err(e) = std::fs::write(&target_path, &content) {
                                    error!("Failed to write export file: {e}");
                                    toast_clone.add_toast(libadwaita::Toast::new(&format!(
                                        "Failed to save file: {}",
                                        e
                                    )));
                                } else {
                                    toast_clone.add_toast(libadwaita::Toast::new(&format!(
                                        "Saved to {}",
                                        target_path.display()
                                    )));
                                }
                            }
                        }
                    }
                });

                chooser.show();
            });
        };

    let win_weak_export = glib::SendWeakRef::from(window.downgrade());
    setup_export_button(
        &srt_btn,
        "srt",
        current_document.clone(),
        win_weak_export.clone(),
        toast_overlay.clone(),
    );
    setup_export_button(
        &vtt_btn,
        "vtt",
        current_document.clone(),
        win_weak_export.clone(),
        toast_overlay.clone(),
    );
    setup_export_button(
        &txt_btn,
        "txt",
        current_document.clone(),
        win_weak_export.clone(),
        toast_overlay.clone(),
    );
    setup_export_button(
        &json_btn,
        "json",
        current_document.clone(),
        win_weak_export.clone(),
        toast_overlay.clone(),
    );
    setup_export_button(
        &md_btn,
        "md",
        current_document.clone(),
        win_weak_export.clone(),
        toast_overlay.clone(),
    );

    window.present();
}
