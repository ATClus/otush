//! Quick Prompt Palette: a floating, keyboard-navigable command palette
//! for choosing an AI prompt template or entering a freeform custom instruction
//! to transform the currently selected text.

use crate::context::AppContext;
use crate::settings::LLMPrompt;
use gdk4::prelude::*;
use gtk4::prelude::*;
use libadwaita::prelude::*;
use log::{info, warn};
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

static PALETTE_WINDOW: LazyLock<Mutex<Option<glib::SendWeakRef<libadwaita::Window>>>> =
    LazyLock::new(|| Mutex::new(None));
static LAST_PALETTE_TOGGLE: LazyLock<Mutex<Option<std::time::Instant>>> =
    LazyLock::new(|| Mutex::new(None));

/// Show the Quick Prompt Palette centered on screen.
pub fn show_prompt_palette(ctx: &AppContext) {
    toggle_prompt_palette(ctx);
}

/// Toggle display of the Quick Prompt Palette.
pub fn toggle_prompt_palette(ctx: &AppContext) {
    let now = std::time::Instant::now();
    if let Ok(mut last) = LAST_PALETTE_TOGGLE.lock() {
        if let Some(prev) = *last {
            if now.duration_since(prev) < Duration::from_millis(300) {
                return;
            }
        }
        *last = Some(now);
    }

    let ctx = ctx.clone();
    glib::MainContext::default().invoke(move || {
        // Check if window is already open and toggle close safely on the GTK main thread
        let existing_win = {
            let mut guard = match PALETTE_WINDOW.lock() {
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
                    if let Ok(mut guard) = PALETTE_WINDOW.lock() {
                        *guard = Some(glib::SendWeakRef::from(win.downgrade()));
                    }
                }
                return;
            }
        }

        let ctx_clone = ctx.clone();
        crate::runtime::spawn_blocking(move || {
            // Capture environment context off the main thread before presenting the palette window
            let pre_captured_text = match crate::clipboard::capture_selected_text() {
                Ok(text) if !text.trim().is_empty() => Some(text),
                _ => None,
            };
            let pre_captured_window = crate::template::get_active_window_title();
            let pre_captured_clipboard = match crate::clipboard::read_clipboard_text() {
                Ok(text) if !text.trim().is_empty() => Some(text),
                _ => None,
            };

            let ctx_for_main = ctx_clone.clone();
            glib::MainContext::default().invoke(move || {
                build_and_present_palette(
                    &ctx_for_main,
                    pre_captured_text,
                    pre_captured_window,
                    pre_captured_clipboard,
                );
            });
        });
    });
}

fn get_prompt_icon_name(name: &str) -> &'static str {
    let lower = name.to_lowercase();
    if lower.contains("translat") {
        "globe-symbolic"
    } else if lower.contains("summar") {
        "format-justify-left-symbolic"
    } else if lower.contains("fix") || lower.contains("grammar") || lower.contains("proof") {
        "format-text-symbolic"
    } else if lower.contains("code") || lower.contains("refactor") {
        "application-x-executable-symbolic"
    } else if lower.contains("email") || lower.contains("reply") {
        "mail-send-symbolic"
    } else {
        "starred-symbolic"
    }
}

fn build_and_present_palette(
    ctx: &AppContext,
    pre_captured_text: Option<String>,
    pre_captured_window: Option<String>,
    pre_captured_clipboard: Option<String>,
) {
    let settings = ctx.settings();
    let prompts = settings.post_process_prompts;

    let window = libadwaita::Window::new();
    window.set_title(Some("Transform Selection"));
    window.set_default_size(560, 480);
    window.set_modal(true);
    window.set_resizable(false);
    window.set_deletable(true);
    window.add_css_class("dialog");

    {
        let mut guard = match PALETTE_WINDOW.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
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

    let window_title = libadwaita::WindowTitle::new("Transform Selection", "AI Prompt Palette");
    header_bar.set_title_widget(Some(&window_title));
    main_box.append(&header_bar);

    // ========================================================================
    // Selection Preview Strip (Visual feedback of captured text)
    // ========================================================================
    let preview_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    preview_box.set_margin_start(16);
    preview_box.set_margin_end(16);
    preview_box.set_margin_top(10);
    preview_box.set_margin_bottom(4);
    preview_box.add_css_class("card");

    let preview_icon = gtk4::Image::new();
    preview_icon.set_margin_start(10);
    preview_icon.set_margin_top(8);
    preview_icon.set_margin_bottom(8);

    let preview_label = gtk4::Label::new(None);
    preview_label.set_hexpand(true);
    preview_label.set_xalign(0.0);
    preview_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    preview_label.set_margin_top(8);
    preview_label.set_margin_bottom(8);

    let stats_badge = gtk4::Label::new(None);
    stats_badge.add_css_class("caption");
    stats_badge.add_css_class("dim-label");
    stats_badge.set_margin_end(10);
    stats_badge.set_margin_top(8);
    stats_badge.set_margin_bottom(8);

    if let Some(ref text) = pre_captured_text {
        preview_icon.set_icon_name(Some("edit-select-symbolic"));
        let first_line = text.lines().find(|l| !l.trim().is_empty()).unwrap_or(text);
        let preview_snippet = if first_line.len() > 60 {
            format!("“{}…”", &first_line[..60])
        } else {
            format!("“{}”", first_line.trim())
        };
        preview_label.set_text(&preview_snippet);

        let words = text.split_whitespace().count();
        let word_lbl = if words == 1 { "word" } else { "words" };
        stats_badge.set_text(&format!("{words} {word_lbl}"));
    } else {
        preview_icon.set_icon_name(Some("edit-paste-symbolic"));
        preview_label.set_text("No text selected • Will transform clipboard text");
        preview_label.add_css_class("dim-label");
        stats_badge.set_text("Clipboard");
    }

    preview_box.append(&preview_icon);
    preview_box.append(&preview_label);
    preview_box.append(&stats_badge);
    main_box.append(&preview_box);

    // ========================================================================
    // Search / Custom Instruction Entry
    // ========================================================================
    let search_box = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    search_box.set_margin_start(16);
    search_box.set_margin_end(16);
    search_box.set_margin_top(6);
    search_box.set_margin_bottom(8);

    let search_entry = gtk4::SearchEntry::new();
    search_entry.set_placeholder_text(Some("Search template, type custom prompt, or press 1-9…"));
    search_box.append(&search_entry);
    main_box.append(&search_box);

    // ========================================================================
    // Scrolled List of Prompts (Boxed List pattern)
    // ========================================================================
    let scrolled = gtk4::ScrolledWindow::new();
    scrolled.set_vexpand(true);
    scrolled.set_hexpand(true);
    scrolled.set_min_content_height(280);

    let list_box = gtk4::ListBox::new();
    list_box.set_selection_mode(gtk4::SelectionMode::Single);
    list_box.add_css_class("boxed-list");
    list_box.set_margin_start(16);
    list_box.set_margin_end(16);
    list_box.set_margin_bottom(16);

    // Dynamic Custom Instruction Row (visible when user types in search entry)
    let custom_action_row = libadwaita::ActionRow::new();
    custom_action_row.set_title("Run custom instruction");
    custom_action_row.set_subtitle("Send custom instruction to AI");
    let custom_icon = gtk4::Image::from_icon_name("system-run-symbolic");
    custom_action_row.add_prefix(&custom_icon);
    let custom_badge = gtk4::Label::new(Some("↵ Enter"));
    custom_badge.add_css_class("caption");
    custom_badge.add_css_class("dim-label");
    custom_action_row.add_suffix(&custom_badge);
    custom_action_row.set_activatable(true);

    let custom_list_row = gtk4::ListBoxRow::new();
    custom_list_row.set_child(Some(&custom_action_row));
    custom_list_row.set_visible(false);
    list_box.append(&custom_list_row);

    let mut prompt_rows: Vec<(LLMPrompt, gtk4::ListBoxRow, String)> = Vec::new();

    for (i, prompt) in prompts.iter().enumerate() {
        let row = libadwaita::ActionRow::new();
        row.set_use_markup(false);
        row.set_title(&prompt.name);

        let shortcut_num = if i < 9 {
            format!("{}", i + 1)
        } else {
            String::new()
        };

        // Icon prefix
        let icon_name = get_prompt_icon_name(&prompt.name);
        let prefix_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        let icon_img = gtk4::Image::from_icon_name(icon_name);
        icon_img.set_pixel_size(16);
        prefix_box.append(&icon_img);

        if !shortcut_num.is_empty() {
            let badge = gtk4::Label::new(Some(&format!("[{}]", shortcut_num)));
            badge.add_css_class("caption");
            badge.add_css_class("dim-label");
            prefix_box.append(&badge);
        }
        row.add_prefix(&prefix_box);

        let preview = prompt
            .prompt
            .lines()
            .find(|l| !l.trim().is_empty() && !l.starts_with('<'))
            .unwrap_or(&prompt.prompt);
        let truncated_preview = if preview.len() > 60 {
            format!("{}…", &preview[..60])
        } else {
            preview.to_string()
        };
        row.set_subtitle(&truncated_preview);
        row.set_activatable(true);

        let list_box_row = gtk4::ListBoxRow::new();
        list_box_row.set_child(Some(&row));
        list_box.append(&list_box_row);

        prompt_rows.push((prompt.clone(), list_box_row, shortcut_num));
    }

    scrolled.set_child(Some(&list_box));
    main_box.append(&scrolled);

    // Select first template row by default if available
    if let Some((_, first_row, _)) = prompt_rows.first() {
        list_box.select_row(Some(first_row));
    }

    // ========================================================================
    // Execution Dispatcher
    // ========================================================================
    let ctx_exec = ctx.clone();
    let win_weak = glib::SendWeakRef::from(window.downgrade());
    let pre_cap_text = pre_captured_text.clone();
    let pre_cap_win = pre_captured_window.clone();
    let pre_cap_clip = pre_captured_clipboard.clone();

    let execute_prompt = move |prompt_to_run: LLMPrompt| {
        if let Some(win) = win_weak.clone().into_weak_ref().upgrade() {
            win.close();
        }
        let ctx = ctx_exec.clone();
        let text = pre_cap_text.clone();
        let win_title = pre_cap_win.clone();
        let clip = pre_cap_clip.clone();
        crate::runtime::spawn(async move {
            execute_transform_pipeline(&ctx, prompt_to_run, text, win_title, clip).await;
        });
    };

    // Row activated (clicked or Enter)
    let p_rows_for_activate = prompt_rows.clone();
    let custom_row_for_activate = custom_list_row.clone();
    let search_entry_for_activate = search_entry.clone();
    let exec_for_activate = execute_prompt.clone();

    list_box.connect_row_activated(move |_list, row| {
        if *row == custom_row_for_activate {
            let query = search_entry_for_activate.text().trim().to_string();
            if !query.is_empty() {
                let custom_prompt = LLMPrompt {
                    id: "custom_ad_hoc".to_string(),
                    name: query.clone(),
                    prompt: query,
                    preferred_provider_id: None,
                };
                exec_for_activate(custom_prompt);
                return;
            }
        }

        for (prompt, r, _) in &p_rows_for_activate {
            if r == row {
                exec_for_activate(prompt.clone());
                break;
            }
        }
    });

    // Search filtering and dynamic custom row update
    let p_rows_filter = prompt_rows.clone();
    let custom_row_filter = custom_list_row.clone();
    let custom_action_filter = custom_action_row.clone();
    let list_weak_filter = glib::SendWeakRef::from(list_box.downgrade());

    search_entry.connect_search_changed(move |entry| {
        let raw_query = entry.text().trim().to_string();
        let query = raw_query.to_lowercase();

        if raw_query.is_empty() {
            custom_row_filter.set_visible(false);
            for (_, row, _) in &p_rows_filter {
                row.set_visible(true);
            }
            if let Some(list) = list_weak_filter.clone().into_weak_ref().upgrade() {
                if let Some((_, first_row, _)) = p_rows_filter.first() {
                    list.select_row(Some(first_row));
                }
            }
        } else {
            custom_row_filter.set_visible(true);
            let truncated_preview = if raw_query.len() > 60 {
                format!("“{}…”", &raw_query[..60])
            } else {
                format!("“{}”", raw_query)
            };
            custom_action_filter.set_subtitle(&truncated_preview);

            let mut matched_template_count = 0;
            for (prompt, row, num) in &p_rows_filter {
                let matches = prompt.name.to_lowercase().contains(&query)
                    || prompt.prompt.to_lowercase().contains(&query)
                    || num == &query;
                row.set_visible(matches);
                if matches {
                    matched_template_count += 1;
                }
            }

            // If no templates match, automatically select custom row; otherwise select first match
            if let Some(list) = list_weak_filter.clone().into_weak_ref().upgrade() {
                if matched_template_count == 0 {
                    list.select_row(Some(&custom_row_filter));
                } else if let Some((_, first_match, _)) =
                    p_rows_filter.iter().find(|(_, r, _)| r.is_visible())
                {
                    list.select_row(Some(first_match));
                }
            }
        }
    });

    // Keyboard navigation, 1-9 shortcuts, and Ctrl+Enter
    let key_controller = gtk4::EventControllerKey::new();
    let p_rows_for_key = prompt_rows.clone();
    let custom_row_for_key = custom_list_row.clone();
    let search_entry_for_key = search_entry.clone();
    let list_weak_key = glib::SendWeakRef::from(list_box.downgrade());
    let win_weak_key = glib::SendWeakRef::from(window.downgrade());
    let exec_for_key = execute_prompt.clone();

    key_controller.connect_key_pressed(move |_ctrl, keyval, _keycode, state| {
        if keyval == gdk4::Key::Escape {
            if let Some(win) = win_weak_key.clone().into_weak_ref().upgrade() {
                win.close();
            }
            return glib::Propagation::Stop;
        }

        let is_ctrl = state.contains(gdk4::ModifierType::CONTROL_MASK);
        let is_alt = state.contains(gdk4::ModifierType::ALT_MASK);

        // Ctrl+Enter: directly run custom prompt from search entry
        if is_ctrl && (keyval == gdk4::Key::Return || keyval == gdk4::Key::KP_Enter) {
            let query = search_entry_for_key.text().trim().to_string();
            if !query.is_empty() {
                let custom_prompt = LLMPrompt {
                    id: "custom_ad_hoc".to_string(),
                    name: query.clone(),
                    prompt: query,
                    preferred_provider_id: None,
                };
                exec_for_key(custom_prompt);
                return glib::Propagation::Stop;
            }
        }

        // Check if user pressed 1-9
        let is_empty_query = search_entry_for_key.text().trim().is_empty();
        if is_alt || is_empty_query {
            let name = keyval.name().unwrap_or_default();
            if let Ok(num) = name.parse::<usize>() {
                if (1..=9).contains(&num) {
                    if let Some((prompt, _, _)) = p_rows_for_key.get(num - 1) {
                        exec_for_key(prompt.clone());
                        return glib::Propagation::Stop;
                    }
                }
            }
        }

        // Enter key activates selected row or fallback custom query
        if keyval == gdk4::Key::Return || keyval == gdk4::Key::KP_Enter {
            let query = search_entry_for_key.text().trim().to_string();

            if let Some(list) = list_weak_key.clone().into_weak_ref().upgrade() {
                if let Some(selected) = list.selected_row() {
                    if selected == custom_row_for_key && custom_row_for_key.is_visible() {
                        if !query.is_empty() {
                            let custom_prompt = LLMPrompt {
                                id: "custom_ad_hoc".to_string(),
                                name: query.clone(),
                                prompt: query,
                                preferred_provider_id: None,
                            };
                            exec_for_key(custom_prompt);
                            return glib::Propagation::Stop;
                        }
                    } else if selected.is_visible() {
                        for (prompt, r, _) in &p_rows_for_key {
                            if *r == selected {
                                exec_for_key(prompt.clone());
                                return glib::Propagation::Stop;
                            }
                        }
                    }
                }
            }

            // Fallback: execute custom query if non-empty
            if !query.is_empty() {
                let custom_prompt = LLMPrompt {
                    id: "custom_ad_hoc".to_string(),
                    name: query.clone(),
                    prompt: query,
                    preferred_provider_id: None,
                };
                exec_for_key(custom_prompt);
                return glib::Propagation::Stop;
            }
        }

        glib::Propagation::Proceed
    });

    window.add_controller(key_controller);

    window.connect_destroy(|_| {
        let mut guard = match PALETTE_WINDOW.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        *guard = None;
    });

    search_entry.grab_focus();
    window.present();
}

/// Directly execute a specific prompt on selected text without opening the palette.
pub fn execute_prompt_by_id(ctx: &AppContext, prompt_id: &str) {
    let settings = ctx.settings();
    if let Some(prompt) = settings
        .post_process_prompts
        .iter()
        .find(|p| p.id == prompt_id)
        .cloned()
    {
        let pre_captured_text = match crate::clipboard::capture_selected_text() {
            Ok(text) if !text.trim().is_empty() => Some(text),
            _ => None,
        };
        let pre_captured_window = crate::template::get_active_window_title();
        let pre_captured_clipboard = match crate::clipboard::read_clipboard_text() {
            Ok(text) if !text.trim().is_empty() => Some(text),
            _ => None,
        };

        let ctx = ctx.clone();
        crate::runtime::spawn(async move {
            execute_transform_pipeline(
                &ctx,
                prompt,
                pre_captured_text,
                pre_captured_window,
                pre_captured_clipboard,
            )
            .await;
        });
    }
}

/// Execute transformation pipeline on currently selected text with full dynamic environment context.
pub(crate) async fn execute_transform_pipeline(
    ctx: &AppContext,
    prompt: LLMPrompt,
    pre_captured_text: Option<String>,
    pre_captured_window: Option<String>,
    pre_captured_clipboard: Option<String>,
) {
    info!(
        "Transform selection triggered with prompt '{}'",
        prompt.name
    );

    // 1. Resolve selected text (use pre-captured text, or fallback to capturing now)
    let selected_text = match pre_captured_text.clone() {
        Some(text) if !text.trim().is_empty() => text,
        _ => match crate::clipboard::capture_selected_text() {
            Ok(text) if !text.trim().is_empty() => text,
            Ok(_) => {
                warn!("No text was selected to transform");
                return;
            }
            Err(e) => {
                warn!("Failed to capture selected text: {e}");
                return;
            }
        },
    };

    info!(
        "Captured selected text ({} chars). Sending to LLM...",
        selected_text.len()
    );

    crate::overlay::show_processing_overlay(ctx);

    // 2. Gather dynamic template context (${output}, ${selected_text}, ${clipboard}, ${active_window}, ${date}, ${language})
    let template_ctx = crate::template::TemplateContext::gather(
        ctx,
        &selected_text,
        Some(selected_text.clone()),
        pre_captured_window,
        pre_captured_clipboard,
    );

    // 3. Execute LLM post-processing with template context
    let settings = ctx.settings();
    let transformed = match crate::actions::post_process_text_with_prompt_and_context(
        &settings,
        &selected_text,
        &prompt,
        &template_ctx,
    )
    .await
    {
        Some(result) if !result.trim().is_empty() => result,
        _ => {
            warn!("LLM post-processing returned empty text or failed");
            crate::overlay::hide_recording_overlay(ctx);
            return;
        }
    };

    info!("Transform completed successfully. Pasting replacement text...");

    // CRITICAL: Hide the overlay before injecting paste so the target window holds focus
    crate::overlay::hide_recording_overlay(ctx);

    // Short grace delay to ensure window manager / compositor restores focus to the target app
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // 4. Paste transformed text over the original selection
    if let Err(e) = crate::clipboard::paste(ctx, transformed) {
        warn!("Failed to paste transformed text: {e}");
    }
}
