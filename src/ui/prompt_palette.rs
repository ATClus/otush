//! Quick Prompt Palette: a floating, keyboard-navigable command palette
//! for choosing an AI prompt to transform the currently selected text.

use crate::context::AppContext;
use crate::settings::LLMPrompt;
use gtk4::prelude::*;
use libadwaita::prelude::*;
use log::{info, warn};
use std::sync::OnceLock;

static PALETTE_WINDOW: OnceLock<glib::SendWeakRef<libadwaita::Window>> = OnceLock::new();

/// Show the Quick Prompt Palette centered on screen.
pub fn show_prompt_palette(ctx: &AppContext) {
    // Capture selected text BEFORE focusing the palette window
    let pre_captured_text = match crate::clipboard::capture_selected_text() {
        Ok(text) if !text.trim().is_empty() => Some(text),
        _ => None,
    };

    let ctx_clone = ctx.clone();
    glib::MainContext::default().invoke(move || {
        build_and_present_palette(&ctx_clone, pre_captured_text);
    });
}

fn build_and_present_palette(ctx: &AppContext, pre_captured_text: Option<String>) {
    // If a palette is already open, focus it
    if let Some(weak) = PALETTE_WINDOW.get() {
        if let Some(win) = weak.clone().into_weak_ref().upgrade() {
            win.present();
            return;
        }
    }

    let settings = ctx.settings();
    let prompts = settings.post_process_prompts;
    if prompts.is_empty() {
        warn!("No prompts available to transform text");
        return;
    }

    let window = libadwaita::Window::new();
    window.set_title(Some("Transform Text"));
    window.set_default_size(480, 360);
    window.set_modal(true);
    window.set_resizable(false);
    window.set_deletable(true);
    window.add_css_class("dialog");

    let toast_overlay = libadwaita::ToastOverlay::new();
    let main_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    toast_overlay.set_child(Some(&main_box));
    window.set_content(Some(&toast_overlay));

    // Header bar with search entry
    let header_bar = libadwaita::HeaderBar::new();
    header_bar.set_show_end_title_buttons(true);
    header_bar.set_title_widget(Some(&gtk4::Label::new(Some("Select Prompt"))));
    main_box.append(&header_bar);

    // Search Entry
    let search_box = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    search_box.set_margin_start(16);
    search_box.set_margin_end(16);
    search_box.set_margin_top(8);
    search_box.set_margin_bottom(8);

    let search_entry = gtk4::SearchEntry::new();
    search_entry.set_placeholder_text(Some("Search prompt or press 1-9…"));
    search_box.append(&search_entry);
    main_box.append(&search_box);

    // Scrolled list of prompts
    let scrolled = gtk4::ScrolledWindow::new();
    scrolled.set_vexpand(true);
    scrolled.set_hexpand(true);
    scrolled.set_min_content_height(240);

    let list_box = gtk4::ListBox::new();
    list_box.set_selection_mode(gtk4::SelectionMode::Single);
    list_box.add_css_class("boxed-list");
    list_box.set_margin_start(16);
    list_box.set_margin_end(16);
    list_box.set_margin_bottom(16);

    let mut prompt_rows: Vec<(LLMPrompt, gtk4::ListBoxRow, String)> = Vec::new();

    for (i, prompt) in prompts.iter().enumerate() {
        let row = libadwaita::ActionRow::new();
        row.set_title(&prompt.name);

        let shortcut_num = if i < 9 {
            format!("{}", i + 1)
        } else {
            "".to_string()
        };

        if !shortcut_num.is_empty() {
            let badge = gtk4::Label::new(Some(&format!("[{}]", shortcut_num)));
            badge.add_css_class("caption");
            badge.add_css_class("dim-label");
            row.add_prefix(&badge);
        }

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

    // Select first row by default
    if let Some(first) = list_box.first_child() {
        if let Ok(row) = first.downcast::<gtk4::ListBoxRow>() {
            list_box.select_row(Some(&row));
        }
    }

    // Execute prompt on selection
    let ctx_exec = ctx.clone();
    let win_weak = glib::SendWeakRef::from(window.downgrade());
    let execute_prompt = move |selected_prompt: LLMPrompt| {
        if let Some(win) = win_weak.clone().into_weak_ref().upgrade() {
            win.close();
        }
        let ctx = ctx_exec.clone();
        let pre_captured = pre_captured_text.clone();
        crate::runtime::spawn(async move {
            execute_transform_pipeline(&ctx, selected_prompt, pre_captured).await;
        });
    };

    // Row activated (clicked / Enter)
    let p_rows_for_activate = prompt_rows.clone();
    let exec_for_activate = execute_prompt.clone();
    list_box.connect_row_activated(move |_list, row| {
        for (prompt, r, _) in &p_rows_for_activate {
            if r == row {
                exec_for_activate(prompt.clone());
                break;
            }
        }
    });

    // Search filtering
    let p_rows_filter = prompt_rows.clone();
    search_entry.connect_search_changed(move |entry| {
        let query = entry.text().to_lowercase();
        for (prompt, row, num) in &p_rows_filter {
            let matches = query.is_empty()
                || prompt.name.to_lowercase().contains(&query)
                || prompt.prompt.to_lowercase().contains(&query)
                || num == &query;
            row.set_visible(matches);
        }
    });

    // Keyboard navigation and 1-9 shortcuts
    let key_controller = gtk4::EventControllerKey::new();
    let p_rows_for_key = prompt_rows.clone();
    let list_weak = glib::SendWeakRef::from(list_box.downgrade());
    let win_weak_key = glib::SendWeakRef::from(window.downgrade());

    key_controller.connect_key_pressed(move |_ctrl, keyval, _keycode, _state| {
        if keyval == gdk4::Key::Escape {
            if let Some(win) = win_weak_key.clone().into_weak_ref().upgrade() {
                win.close();
            }
            return glib::Propagation::Stop;
        }

        // Check if user pressed 1-9 for direct item selection
        let name = keyval.name().unwrap_or_default();
        if let Ok(num) = name.parse::<usize>() {
            if (1..=9).contains(&num) {
                if let Some((prompt, _, _)) = p_rows_for_key.get(num - 1) {
                    execute_prompt(prompt.clone());
                    return glib::Propagation::Stop;
                }
            }
        }

        // Enter key activates selected row
        if keyval == gdk4::Key::Return || keyval == gdk4::Key::KP_Enter {
            if let Some(list) = list_weak.clone().into_weak_ref().upgrade() {
                if let Some(selected) = list.selected_row() {
                    for (prompt, r, _) in &p_rows_for_key {
                        if *r == selected {
                            execute_prompt(prompt.clone());
                            return glib::Propagation::Stop;
                        }
                    }
                }
            }
        }

        glib::Propagation::Proceed
    });

    window.add_controller(key_controller);

    let _ = PALETTE_WINDOW.set(glib::SendWeakRef::from(window.downgrade()));
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
        let ctx = ctx.clone();
        crate::runtime::spawn(async move {
            execute_transform_pipeline(&ctx, prompt, None).await;
        });
    }
}

/// Execute transformation pipeline on currently selected text.
pub(crate) async fn execute_transform_pipeline(
    ctx: &AppContext,
    prompt: LLMPrompt,
    pre_captured_text: Option<String>,
) {
    info!(
        "Transform selection triggered with prompt '{}'",
        prompt.name
    );

    // 1. Resolve selected text (use pre-captured text, or fallback to capturing now)
    let selected_text = match pre_captured_text {
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

    // 2. Execute LLM post-processing
    let settings = ctx.settings();
    let transformed =
        match crate::actions::post_process_text_with_prompt(&settings, &selected_text, &prompt)
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

    // 3. Paste transformed text over the original selection
    if let Err(e) = crate::clipboard::paste(ctx, transformed) {
        warn!("Failed to paste transformed text: {e}");
    }
}
