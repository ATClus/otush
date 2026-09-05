//! Combined Chat & Research overlay (`Ctrl+Alt+S` opens last mode).
//!
//! Native GTK4 + libadwaita shell with a mode selector in the header:
//! - **Chat** (default): multi-turn conversation with a configured AI agent
//!   ([`crate::agents`] ReAct loop with Tavily/Firecrawl tools + local RAG).
//! - **Search**: the legacy web & deep research UI ([`super::search_mode`]),
//!   mounted unchanged into this window.
//!
//! Threading follows the project rule: widgets only on the GTK main thread,
//! backend via [`crate::runtime::spawn`], progress through
//! [`AppEvent`](crate::context::AppEvent) marshaled with
//! `glib::MainContext::default().invoke`.

use crate::context::{AppContext, AppEvent};
use crate::settings;
use gtk4::prelude::*;
use libadwaita::prelude::*;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

static CHAT_WINDOW: LazyLock<Mutex<Option<glib::SendWeakRef<libadwaita::Window>>>> =
    LazyLock::new(|| Mutex::new(None));
static LAST_CHAT_TOGGLE: LazyLock<Mutex<Option<std::time::Instant>>> =
    LazyLock::new(|| Mutex::new(None));
static LAST_MODE: LazyLock<Mutex<OverlayMode>> =
    LazyLock::new(|| Mutex::new(OverlayMode::default()));

/// Which page of the combined overlay to show.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum OverlayMode {
    #[default]
    Chat,
    Search,
}

/// Toggle or show the combined overlay (remembers the last mode).
pub fn show_chat_overlay(ctx: &AppContext) {
    toggle_chat_overlay(ctx);
}

/// Toggle display of the combined overlay window.
pub fn toggle_chat_overlay(ctx: &AppContext) {
    let now = std::time::Instant::now();
    if let Ok(mut last) = LAST_CHAT_TOGGLE.lock() {
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
            let mut guard = match CHAT_WINDOW.lock() {
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
                    if let Ok(mut guard) = CHAT_WINDOW.lock() {
                        *guard = Some(glib::SendWeakRef::from(win.downgrade()));
                    }
                }
                return;
            }
        }

        let mode = LAST_MODE.lock().map(|m| *m).unwrap_or_default();
        build_and_present_chat_overlay(&ctx, mode);
    });
}

/// Open the overlay directly in Search mode (legacy `search_overlay` path).
pub fn show_search_mode(ctx: &AppContext) {
    if let Ok(mut mode) = LAST_MODE.lock() {
        *mode = OverlayMode::Search;
    }
    show_chat_overlay(ctx);
}

/// Open the overlay directly in Chat mode (new `agent_chat` path).
pub fn show_agent_chat(ctx: &AppContext) {
    if let Ok(mut mode) = LAST_MODE.lock() {
        *mode = OverlayMode::Chat;
    }
    show_chat_overlay(ctx);
}

fn build_and_present_chat_overlay(ctx: &AppContext, mode: OverlayMode) {
    let window = libadwaita::Window::new();
    window.set_title(Some("Chat & Research"));
    window.set_default_size(820, 600);
    window.set_modal(true);
    window.set_resizable(true);
    window.set_deletable(true);
    window.add_css_class("dialog");

    {
        let mut guard = match CHAT_WINDOW.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        *guard = Some(glib::SendWeakRef::from(window.downgrade()));
    }

    let toast_overlay = libadwaita::ToastOverlay::new();
    let main_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    toast_overlay.set_child(Some(&main_box));
    window.set_content(Some(&toast_overlay));

    // --- header: title + mode selector ---
    let header_bar = libadwaita::HeaderBar::new();
    header_bar.set_show_end_title_buttons(true);
    let window_title =
        libadwaita::WindowTitle::new("Chat & Research", "AI Agents • Tavily & Firecrawl");
    header_bar.set_title_widget(Some(&window_title));

    let mode_stack = gtk4::Stack::new();
    mode_stack.set_transition_type(gtk4::StackTransitionType::Crossfade);
    mode_stack.set_transition_duration(200);
    mode_stack.set_vexpand(true);
    mode_stack.set_hexpand(true);

    // Chat page mounts first so it can own long-lived state.
    let chat_page = build_chat_page(ctx, &toast_overlay, &window_title);
    mode_stack.add_named(&chat_page, Some("chat"));

    let search_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    search_box.set_vexpand(true);
    search_box.set_hexpand(true);
    mode_stack.add_named(&search_box, Some("search"));

    // Lazily mount search content on first selection (same cost as before,
    // just deferred until the user picks the tab).
    let search_mounted = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let switch_to = {
        let mode_stack = mode_stack.clone();
        let window_title = window_title.clone();
        let search_mounted = search_mounted.clone();
        let search_box = search_box.clone();
        let toast_overlay = toast_overlay.clone();
        let ctx = ctx.clone();
        move |mode: OverlayMode| {
            if let Ok(mut last) = LAST_MODE.lock() {
                *last = mode;
            }
            match mode {
                OverlayMode::Chat => {
                    mode_stack.set_visible_child_name("chat");
                    window_title.set_subtitle("AI Agents • Tavily & Firecrawl");
                }
                OverlayMode::Search => {
                    if !search_mounted.swap(true, std::sync::atomic::Ordering::Relaxed) {
                        super::search_mode::build_search_mode(
                            &ctx,
                            &toast_overlay,
                            &search_box,
                            &window_title,
                            || {},
                        );
                    }
                    mode_stack.set_visible_child_name("search");
                }
            }
        }
    };

    let chat_btn = gtk4::ToggleButton::with_label("Chat");
    let search_btn = gtk4::ToggleButton::with_label("Search");
    chat_btn.set_group(Some(&search_btn));
    chat_btn.add_css_class("flat");
    search_btn.add_css_class("flat");
    match mode {
        OverlayMode::Chat => chat_btn.set_active(true),
        OverlayMode::Search => search_btn.set_active(true),
    }
    let switch_chat = switch_to.clone();
    chat_btn.connect_toggled(move |btn| {
        if btn.is_active() {
            switch_chat(OverlayMode::Chat);
        }
    });
    let switch_search = switch_to.clone();
    search_btn.connect_toggled(move |btn| {
        if btn.is_active() {
            switch_search(OverlayMode::Search);
        }
    });
    let mode_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    mode_box.add_css_class("linked");
    mode_box.append(&chat_btn);
    mode_box.append(&search_btn);
    header_bar.pack_start(&mode_box);

    main_box.append(&header_bar);
    main_box.append(&mode_stack);

    // Initial mode.
    switch_to(mode);

    // Esc closes the window from anywhere in the overlay.
    let key_controller = gtk4::EventControllerKey::new();
    let win_weak = glib::SendWeakRef::from(window.downgrade());
    key_controller.connect_key_pressed(move |_, key, _, _| {
        if key == gdk4::Key::Escape {
            if let Some(w) = win_weak.clone().into_weak_ref().upgrade() {
                w.close();
                return glib::Propagation::Stop;
            }
        }
        glib::Propagation::Proceed
    });
    window.add_controller(key_controller);

    window.connect_destroy(|_| {
        let mut guard = match CHAT_WINDOW.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        *guard = None;
    });

    window.present();
}

// ============================================================================
// Chat page
// ============================================================================

/// Per-open chat UI state (GTK thread only; backend progress arrives as bus
/// events keyed by `chat_id`).
struct ChatUi {
    chat_id: Option<i64>,
    generating: bool,
    pending_steps: Vec<(String, String)>,
    /// Live assistant bubble receiving `AgentToken` deltas (`None` until the
    /// first token of the turn creates it). Stored as a weak ref: GTK
    /// widgets are `!Send` and must never be held across threads (the bus
    /// closure requires `Send`).
    live_label: Option<glib::SendWeakRef<gtk4::Label>>,
}

fn build_chat_page(
    ctx: &AppContext,
    toast_overlay: &libadwaita::ToastOverlay,
    window_title: &libadwaita::WindowTitle,
) -> gtk4::Widget {
    let page = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    page.set_vexpand(true);
    page.set_hexpand(true);

    // --- agent selector row ---
    let agent_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    agent_row.set_margin_start(16);
    agent_row.set_margin_end(16);
    agent_row.set_margin_top(12);
    agent_row.set_margin_bottom(6);

    let agent_label = gtk4::Label::new(Some("Agent:"));
    agent_label.add_css_class("dim-label");
    agent_row.append(&agent_label);

    let agent_dropdown = gtk4::DropDown::from_strings(&["Loading agents…"]);
    agent_dropdown.set_hexpand(true);
    agent_dropdown.set_tooltip_text(Some("Active AI agent (Settings → Agents to edit)"));
    agent_row.append(&agent_dropdown);

    let new_chat_btn = gtk4::Button::from_icon_name("document-new-symbolic");
    new_chat_btn.set_tooltip_text(Some("New chat"));
    new_chat_btn.add_css_class("flat");
    agent_row.append(&new_chat_btn);

    let history_btn = gtk4::Button::from_icon_name("document-open-recent-symbolic");
    history_btn.set_tooltip_text(Some("Chat history"));
    history_btn.add_css_class("flat");
    agent_row.append(&history_btn);

    let export_btn = gtk4::Button::from_icon_name("document-save-symbolic");
    export_btn.set_tooltip_text(Some("Copy chat as Markdown"));
    export_btn.add_css_class("flat");
    agent_row.append(&export_btn);

    let save_note_btn = gtk4::Button::from_icon_name("text-editor-symbolic");
    save_note_btn.set_tooltip_text(Some("Save chat to Quick Notes"));
    save_note_btn.add_css_class("flat");
    agent_row.append(&save_note_btn);

    page.append(&agent_row);

    // --- transcript ---
    let scrolled = gtk4::ScrolledWindow::new();
    scrolled.set_vexpand(true);
    scrolled.set_hexpand(true);
    let transcript = gtk4::ListBox::new();
    transcript.set_selection_mode(gtk4::SelectionMode::None);
    transcript.set_margin_start(16);
    transcript.set_margin_end(16);
    transcript.set_margin_top(6);
    transcript.set_margin_bottom(6);
    scrolled.set_child(Some(&transcript));
    page.append(&scrolled);

    // --- tool timeline (collapsible steps of the running turn) ---
    let steps_box = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    steps_box.set_margin_start(16);
    steps_box.set_margin_end(16);
    steps_box.set_visible(false);
    page.append(&steps_box);

    // --- input row ---
    let input_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    input_row.set_margin_start(16);
    input_row.set_margin_end(16);
    input_row.set_margin_top(6);
    input_row.set_margin_bottom(12);

    let entry = gtk4::SearchEntry::new();
    entry.set_hexpand(true);
    entry.set_placeholder_text(Some("Ask the agent… (Enter to send)"));
    input_row.append(&entry);

    let send_btn = gtk4::Button::with_label("Send");
    send_btn.add_css_class("suggested-action");
    input_row.append(&send_btn);

    let stop_btn = gtk4::Button::from_icon_name("process-stop-symbolic");
    stop_btn.set_tooltip_text(Some("Stop generating"));
    stop_btn.set_visible(false);
    input_row.append(&stop_btn);

    page.append(&input_row);

    let state = Arc::new(Mutex::new(ChatUi {
        chat_id: None,
        generating: false,
        pending_steps: Vec::new(),
        live_label: None,
    }));

    // Refresh the agent dropdown from settings.
    let refresh_agents = {
        let agent_dropdown = agent_dropdown.clone();
        let ctx = ctx.clone();
        let state = state.clone();
        let transcript = transcript.clone();
        let steps_box = steps_box.clone();
        move || {
            let settings = settings::get_settings(&ctx);
            let names: Vec<String> = settings
                .agents
                .iter()
                .filter(|a| a.enabled)
                .map(|a| {
                    let model = a
                        .model_override
                        .as_deref()
                        .map(str::trim)
                        .filter(|m| !m.is_empty())
                        .map(str::to_string)
                        .or_else(|| {
                            settings
                                .post_process_models
                                .get(&a.provider_id)
                                .cloned()
                                .filter(|m| !m.trim().is_empty())
                        })
                        .unwrap_or_else(|| "no model".to_string());
                    format!("{} ({})", a.name, model)
                })
                .collect();
            let store =
                gtk4::StringList::new(&names.iter().map(|s| s.as_str()).collect::<Vec<_>>());
            agent_dropdown.set_model(Some(&store));
            let selected = settings
                .selected_agent()
                .and_then(|sel| {
                    settings
                        .agents
                        .iter()
                        .filter(|a| a.enabled)
                        .position(|a| a.id == sel.id)
                })
                .unwrap_or(0) as u32;
            agent_dropdown.set_selected(selected);
            // Ensure a chat exists for the selected agent.
            ensure_chat(&ctx, &state, &transcript, &steps_box);
        }
    };
    refresh_agents();

    // Agent selection → notify settings + new chat.
    {
        let ctx = ctx.clone();
        let state = state.clone();
        let transcript = transcript.clone();
        let steps_box = steps_box.clone();
        let toast = toast_overlay.clone();
        agent_dropdown.connect_selected_notify(move |dropdown| {
            let idx = dropdown.selected() as usize;
            let settings = settings::get_settings(&ctx);
            let enabled: Vec<_> = settings.agents.iter().filter(|a| a.enabled).collect();
            if let Some(agent) = enabled.get(idx) {
                if let Err(e) = crate::commands::agents::select_agent(&ctx, &agent.id) {
                    ctx.report_error("select_agent", e);
                    toast.add_toast(libadwaita::Toast::new("Could not select agent"));
                    return;
                }
                if let Ok(mut st) = state.lock() {
                    st.chat_id = None;
                    st.generating = false;
                    st.pending_steps.clear();
                }
                clear_transcript(&transcript);
                steps_box.set_visible(false);
                ensure_chat(&ctx, &state, &transcript, &steps_box);
            }
        });
    }

    // New chat button.
    {
        let ctx = ctx.clone();
        let state = state.clone();
        let transcript = transcript.clone();
        let steps_box = steps_box.clone();
        new_chat_btn.connect_clicked(move |_| {
            switch_chat(&ctx, &state, &transcript, &steps_box, None);
        });
    }

    // History menu: recent chats for the selected agent, with delete.
    {
        let ctx = ctx.clone();
        let state = state.clone();
        let transcript = transcript.clone();
        let steps_box = steps_box.clone();
        let toast = toast_overlay.clone();
        let history_menu = gtk4::Popover::new();
        history_menu.set_has_arrow(true);
        history_menu.set_autohide(true);
        let menu_box = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
        menu_box.set_margin_start(8);
        menu_box.set_margin_end(8);
        menu_box.set_margin_top(8);
        menu_box.set_margin_bottom(8);
        history_menu.set_child(Some(&menu_box));
        history_menu.set_parent(&history_btn);
        history_btn.connect_clicked(move |_| {
            refresh_history_menu(&ctx, &menu_box, &state, &transcript, &steps_box, &toast);
            history_menu.popup();
        });
    }

    // Export button: copy the chat as Markdown.
    {
        let ctx = ctx.clone();
        let state = state.clone();
        let toast = toast_overlay.clone();
        export_btn.connect_clicked(move |_| {
            let chat_id = state.lock().map(|s| s.chat_id).unwrap_or(None);
            let Some(chat_id) = chat_id else {
                toast.add_toast(libadwaita::Toast::new("Nothing to export yet"));
                return;
            };
            match crate::commands::agents::export_chat_markdown(&ctx, chat_id) {
                Ok(md) => {
                    if let Ok(mut cb) = arboard::Clipboard::new() {
                        let _ = cb.set_text(&md);
                    }
                    toast.add_toast(libadwaita::Toast::new("Copied chat as Markdown"));
                }
                Err(e) => {
                    ctx.report_error("export_chat", e);
                    toast.add_toast(libadwaita::Toast::new("Could not export chat"));
                }
            }
        });
    }

    // Save-to-notes button.
    {
        let ctx = ctx.clone();
        let state = state.clone();
        let toast = toast_overlay.clone();
        save_note_btn.connect_clicked(move |_| {
            let chat_id = state.lock().map(|s| s.chat_id).unwrap_or(None);
            let Some(chat_id) = chat_id else {
                toast.add_toast(libadwaita::Toast::new("Nothing to save yet"));
                return;
            };
            match crate::commands::agents::export_chat_markdown(&ctx, chat_id) {
                Ok(md) => match ctx.history.save_note(
                    "Agent chat".to_string(),
                    md,
                    Some("agent,chat,ai".to_string()),
                ) {
                    Ok(_) => {
                        toast.add_toast(libadwaita::Toast::new("Saved chat to Quick Notes!"));
                    }
                    Err(e) => {
                        toast.add_toast(libadwaita::Toast::new(&format!("Error saving note: {e}")));
                    }
                },
                Err(e) => {
                    ctx.report_error("export_chat", e);
                    toast.add_toast(libadwaita::Toast::new("Could not export chat"));
                }
            }
        });
    }

    // Send path (button + Enter).
    let do_send = {
        let ctx = ctx.clone();
        let state = state.clone();
        let entry = entry.clone();
        let transcript = transcript.clone();
        let steps_box = steps_box.clone();
        let send_btn = send_btn.clone();
        let stop_btn = stop_btn.clone();
        let toast = toast_overlay.clone();
        move || {
            let text = entry.text().to_string();
            if text.trim().is_empty() {
                return;
            }
            let chat_id = {
                let st = state.lock().map(|s| s.chat_id).unwrap_or(None);
                match st {
                    Some(id) => id,
                    None => match ensure_chat(&ctx, &state, &transcript, &steps_box) {
                        Some(id) => id,
                        None => {
                            toast.add_toast(libadwaita::Toast::new(
                                "Configure an agent first (Settings → Agents)",
                            ));
                            return;
                        }
                    },
                }
            };
            append_bubble(&transcript, "You", &text, false);
            entry.set_text("");
            if let Err(e) = crate::commands::agents::send_message(&ctx, chat_id, &text) {
                ctx.report_error("send_message", e);
                toast.add_toast(libadwaita::Toast::new("Could not send message"));
                return;
            }
            if let Ok(mut st) = state.lock() {
                st.generating = true;
                st.pending_steps.clear();
                st.live_label = None;
            }
            // The live bubble is created on the first AgentToken; pre-create
            // it here so empty-token turns (tool-only replies) still show
            // progress instead of a frozen transcript.
            st_live_bubble(&state, &transcript);
            send_btn.set_sensitive(false);
            stop_btn.set_visible(true);
            steps_box.set_visible(false);
            while let Some(child) = steps_box.first_child() {
                steps_box.remove(&child);
            }
            let _ = window_title;
        }
    };
    let send_enter = do_send.clone();
    send_btn.connect_clicked(move |_| do_send());
    entry.connect_activate(move |_| send_enter());

    // Stop button.
    {
        let ctx = ctx.clone();
        let state = state.clone();
        stop_btn.connect_clicked(move |_| {
            let chat_id = state.lock().map(|s| s.chat_id).unwrap_or(None);
            if let Some(id) = chat_id {
                let _ = crate::commands::agents::stop_chat(&ctx, id);
            }
        });
    }

    // Bus subscription: steps, placeholder replacement, completion.
    {
        let ctx_bus = ctx.clone();
        let transcript_weak = glib::SendWeakRef::from(transcript.downgrade());
        let steps_weak = glib::SendWeakRef::from(steps_box.downgrade());
        let send_weak = glib::SendWeakRef::from(send_btn.downgrade());
        let stop_weak = glib::SendWeakRef::from(stop_btn.downgrade());
        let toast_weak = glib::SendWeakRef::from(toast_overlay.downgrade());
        let state = state.clone();
        let sub = ctx.bus.subscribe(move |event| {
            let ctx_bus = ctx_bus.clone();
            let state = state.clone();
            let transcript_weak = transcript_weak.clone();
            let steps_weak = steps_weak.clone();
            let send_weak = send_weak.clone();
            let stop_weak = stop_weak.clone();
            let toast_weak = toast_weak.clone();
            glib::MainContext::default().invoke(move || {
                let my_chat = state.lock().map(|s| s.chat_id).unwrap_or(None);
                match event {
                    AppEvent::AgentStep {
                        chat_id,
                        tool,
                        summary,
                    } => {
                        if Some(chat_id) != my_chat {
                            return;
                        }
                        if let Some(steps) = steps_weak.into_weak_ref().upgrade() {
                            steps.set_visible(true);
                            let row = gtk4::Label::new(Some(&format!("⚙ {tool}: {summary}")));
                            row.set_xalign(0.0);
                            row.add_css_class("caption");
                            row.add_css_class("dim-label");
                            steps.append(&row);
                        }
                    }
                    AppEvent::AgentToken { chat_id, delta } => {
                        if Some(chat_id) != my_chat {
                            return;
                        }
                        // Existing live bubble wins; otherwise create one
                        // (transcript gone = overlay closed: skip token).
                        let mut live: Option<glib::SendWeakRef<gtk4::Label>> = None;
                        if let Ok(guard) = state.lock() {
                            live = guard.live_label.clone();
                        }
                        let label = match live.and_then(|w| w.into_weak_ref().upgrade()) {
                            Some(label) => Some(label),
                            None => {
                                let transcript = transcript_weak.into_weak_ref().upgrade();
                                transcript.map(|t| {
                                    let label = append_live_bubble(&t);
                                    if let Ok(mut guard) = state.lock() {
                                        guard.live_label =
                                            Some(glib::SendWeakRef::from(label.downgrade()));
                                    }
                                    label
                                })
                            }
                        };
                        if let Some(label) = label {
                            // Append-only: deltas arrive in order on one turn.
                            let mut text = label.text().to_string();
                            text.push_str(&delta);
                            label.set_text(&text);
                        }
                    }
                    AppEvent::AgentMessageAdded { chat_id, message } => {
                        if Some(chat_id) != my_chat {
                            return;
                        }
                        // Final text replaces the live bubble (or the "…").
                        if message.role == "assistant" && message.content != "…" {
                            let mut live: Option<gtk4::Label> = None;
                            if let Ok(mut guard) = state.lock() {
                                live = guard
                                    .live_label
                                    .take()
                                    .and_then(|w| w.into_weak_ref().upgrade());
                            }
                            if let Some(label) = live {
                                label.set_text(&message.content);
                                return;
                            }
                            if let Some(transcript) = transcript_weak.into_weak_ref().upgrade() {
                                replace_last_assistant(&transcript, &message.content);
                            }
                        }
                    }
                    AppEvent::AgentDone {
                        chat_id, truncated, ..
                    } => {
                        if Some(chat_id) != my_chat {
                            return;
                        }
                        if let Ok(mut st) = state.lock() {
                            st.generating = false;
                            st.live_label = None;
                        }
                        if let Some(btn) = send_weak.into_weak_ref().upgrade() {
                            btn.set_sensitive(true);
                        }
                        if let Some(btn) = stop_weak.into_weak_ref().upgrade() {
                            btn.set_visible(false);
                        }
                        if truncated {
                            if let Some(t) = toast_weak.into_weak_ref().upgrade() {
                                t.add_toast(libadwaita::Toast::new(
                                    "Stopped at the agent's tool-step limit",
                                ));
                            }
                        }
                        let _ = &ctx_bus;
                    }
                    AppEvent::SettingsChanged { setting, .. }
                        if setting == "agents" || setting == "selected_agent_id" =>
                    {
                        // Dropdown refresh happens on next open; nothing
                        // live to update mid-chat.
                    }
                    _ => {}
                }
            });
        });
        // Keep the subscription alive with the page (leak-free: the overlay
        // is process-lifetime via CHAT_WINDOW; identical pattern to pages).
        std::mem::forget(sub);
    }

    page.upcast::<gtk4::Widget>()
}

/// Ensure a chat row exists for the selected agent; load its transcript.
fn ensure_chat(
    ctx: &AppContext,
    state: &Arc<Mutex<ChatUi>>,
    transcript: &gtk4::ListBox,
    _steps_box: &gtk4::Box,
) -> Option<i64> {
    if let Some(id) = state.lock().map(|s| s.chat_id).unwrap_or(None) {
        return Some(id);
    }
    let settings = settings::get_settings(ctx);
    let agent = settings.selected_agent()?;
    let chat = crate::commands::agents::start_chat(ctx, Some(&agent.id), None).ok()?;
    load_chat(ctx, state, transcript, chat.id);
    Some(chat.id)
}

/// Load an existing chat into the transcript (or a fresh one when `None`).
fn switch_chat(
    ctx: &AppContext,
    state: &Arc<Mutex<ChatUi>>,
    transcript: &gtk4::ListBox,
    steps_box: &gtk4::Box,
    chat_id: Option<i64>,
) {
    if let Ok(mut st) = state.lock() {
        st.chat_id = None;
        st.generating = false;
        st.pending_steps.clear();
        st.live_label = None;
    }
    clear_transcript(transcript);
    steps_box.set_visible(false);
    while let Some(child) = steps_box.first_child() {
        steps_box.remove(&child);
    }
    match chat_id {
        Some(id) => load_chat(ctx, state, transcript, id),
        None => {
            ensure_chat(ctx, state, transcript, steps_box);
        }
    }
}

/// Point `state` at `chat_id` and render its persisted transcript.
fn load_chat(
    ctx: &AppContext,
    state: &Arc<Mutex<ChatUi>>,
    transcript: &gtk4::ListBox,
    chat_id: i64,
) {
    if let Ok(mut st) = state.lock() {
        st.chat_id = Some(chat_id);
        st.live_label = None;
    }
    let settings = settings::get_settings(ctx);
    let agent_name = settings
        .agents
        .iter()
        .find(|a| {
            ctx.history
                .list_agent_chats(None)
                .map(|chats| {
                    chats
                        .iter()
                        .find(|c| c.id == chat_id)
                        .map(|c| c.agent_id == a.id)
                        .unwrap_or(false)
                })
                .unwrap_or(false)
        })
        .map(|a| a.name.clone())
        .or_else(|| settings.selected_agent().map(|a| a.name.clone()))
        .unwrap_or_else(|| "Assistant".to_string());
    if let Ok(messages) = crate::commands::agents::list_messages(ctx, chat_id) {
        for msg in messages {
            match msg.role.as_str() {
                "user" => append_bubble(transcript, "You", &msg.content, false),
                "assistant" if msg.content != "…" => {
                    append_bubble(transcript, &agent_name, &msg.content, true)
                }
                _ => {}
            }
        }
    }
}

/// Rebuild the history popover: recent chats for the selected agent, each
/// with a delete button. Opening a chat loads it; deleting the open chat
/// starts a fresh one.
fn refresh_history_menu(
    ctx: &AppContext,
    menu_box: &gtk4::Box,
    state: &Arc<Mutex<ChatUi>>,
    transcript: &gtk4::ListBox,
    steps_box: &gtk4::Box,
    toast: &libadwaita::ToastOverlay,
) {
    while let Some(child) = menu_box.first_child() {
        menu_box.remove(&child);
    }
    let settings = settings::get_settings(ctx);
    let agent = settings.selected_agent();
    let chats = agent
        .as_ref()
        .and_then(|a| crate::commands::agents::list_chats(ctx, Some(&a.id)).ok())
        .unwrap_or_default();
    let current = state.lock().map(|s| s.chat_id).unwrap_or(None);
    if chats.is_empty() {
        let empty = gtk4::Label::new(Some("No saved chats yet"));
        empty.add_css_class("dim-label");
        empty.add_css_class("caption");
        menu_box.append(&empty);
        return;
    }
    for chat in chats.into_iter().take(20) {
        let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        row.set_hexpand(true);
        let open_btn = gtk4::Button::with_label(&chat.title);
        open_btn.set_hexpand(true);
        open_btn.add_css_class("flat");
        if Some(chat.id) == current {
            open_btn.set_sensitive(false);
        }
        let ctx_open = ctx.clone();
        let state_open = state.clone();
        let transcript_open = transcript.clone();
        let steps_open = steps_box.clone();
        let chat_id = chat.id;
        open_btn.connect_clicked(move |_| {
            if state_open.lock().map(|s| s.generating).unwrap_or(false) {
                return;
            }
            switch_chat(
                &ctx_open,
                &state_open,
                &transcript_open,
                &steps_open,
                Some(chat_id),
            );
        });
        row.append(&open_btn);
        // Rename: small inline dialog (entry + save).
        let rename_btn = gtk4::Button::from_icon_name("document-edit-symbolic");
        rename_btn.add_css_class("flat");
        rename_btn.set_tooltip_text(Some("Rename chat"));
        let ctx_rename = ctx.clone();
        let menu_rename = menu_box.clone();
        let state_rename = state.clone();
        let transcript_rename = transcript.clone();
        let steps_rename = steps_box.clone();
        let toast_rename = toast.clone();
        let old_title = chat.title.clone();
        rename_btn.connect_clicked(move |_| {
            show_rename_dialog(
                &ctx_rename,
                chat_id,
                &old_title,
                &menu_rename,
                &state_rename,
                &transcript_rename,
                &steps_rename,
                &toast_rename,
            );
        });
        row.append(&rename_btn);
        let delete_btn = gtk4::Button::from_icon_name("user-trash-symbolic");
        delete_btn.add_css_class("flat");
        delete_btn.set_tooltip_text(Some("Delete chat"));
        let ctx_delete = ctx.clone();
        let state_delete = state.clone();
        let transcript_delete = transcript.clone();
        let steps_delete = steps_box.clone();
        let menu_delete = menu_box.clone();
        let toast_delete = toast.clone();
        let is_current = Some(chat.id) == current;
        delete_btn.connect_clicked(move |_| {
            if let Err(e) = crate::commands::agents::delete_chat(&ctx_delete, chat_id) {
                ctx_delete.report_error("delete_chat", e);
                toast_delete.add_toast(libadwaita::Toast::new("Could not delete chat"));
                return;
            }
            if is_current {
                switch_chat(
                    &ctx_delete,
                    &state_delete,
                    &transcript_delete,
                    &steps_delete,
                    None,
                );
            }
            refresh_history_menu(
                &ctx_delete,
                &menu_delete,
                &state_delete,
                &transcript_delete,
                &steps_delete,
                &toast_delete,
            );
        });
        row.append(&delete_btn);
        menu_box.append(&row);
    }
}

/// Rename dialog for one chat: modal entry pre-filled with the old title.
#[allow(clippy::too_many_arguments)]
fn show_rename_dialog(
    ctx: &AppContext,
    chat_id: i64,
    old_title: &str,
    menu_box: &gtk4::Box,
    state: &Arc<Mutex<ChatUi>>,
    transcript: &gtk4::ListBox,
    steps_box: &gtk4::Box,
    toast: &libadwaita::ToastOverlay,
) {
    let dialog = libadwaita::Window::new();
    dialog.set_title(Some("Rename chat"));
    dialog.set_modal(true);
    dialog.set_default_size(360, 180);
    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    content.set_margin_start(20);
    content.set_margin_end(20);
    content.set_margin_top(20);
    content.set_margin_bottom(20);
    let entry = libadwaita::EntryRow::new();
    entry.set_title("Title");
    entry.set_text(old_title);
    content.append(&entry);
    let buttons = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    buttons.set_halign(gtk4::Align::End);
    let cancel_btn = gtk4::Button::with_label("Cancel");
    cancel_btn.add_css_class("flat");
    let save_btn = gtk4::Button::with_label("Save");
    save_btn.add_css_class("suggested-action");
    buttons.append(&cancel_btn);
    buttons.append(&save_btn);
    content.append(&buttons);
    dialog.set_content(Some(&content));

    let dialog_weak = glib::SendWeakRef::from(dialog.downgrade());
    cancel_btn.connect_clicked(move |_| {
        if let Some(d) = dialog_weak.clone().into_weak_ref().upgrade() {
            d.close();
        }
    });
    let ctx_save = ctx.clone();
    let menu_save = menu_box.clone();
    let state_save = state.clone();
    let transcript_save = transcript.clone();
    let steps_save = steps_box.clone();
    let toast_save = toast.clone();
    let dialog_save = glib::SendWeakRef::from(dialog.downgrade());
    let entry_save = entry.clone();
    save_btn.connect_clicked(move |_| {
        let title = entry_save.text().to_string();
        if title.trim().is_empty() {
            return;
        }
        if let Err(e) = crate::commands::agents::rename_chat(&ctx_save, chat_id, &title) {
            ctx_save.report_error("rename_chat", e);
            toast_save.add_toast(libadwaita::Toast::new("Could not rename chat"));
            return;
        }
        refresh_history_menu(
            &ctx_save,
            &menu_save,
            &state_save,
            &transcript_save,
            &steps_save,
            &toast_save,
        );
        if let Some(d) = dialog_save.clone().into_weak_ref().upgrade() {
            d.close();
        }
    });
    let save_activate = save_btn.clone();
    entry.connect_activate(move |_| {
        save_activate.emit_clicked();
    });
    dialog.present();
}

fn clear_transcript(transcript: &gtk4::ListBox) {
    while let Some(child) = transcript.first_child() {
        transcript.remove(&child);
    }
}

fn append_bubble(transcript: &gtk4::ListBox, who: &str, text: &str, is_agent: bool) {
    let row = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    row.set_margin_top(6);
    row.set_margin_bottom(6);

    let name = gtk4::Label::new(Some(who));
    name.set_xalign(0.0);
    name.add_css_class("caption");
    name.add_css_class("dim-label");
    row.append(&name);

    let card = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    card.add_css_class("card");
    card.set_margin_bottom(2);
    let body = gtk4::Label::new(Some(text));
    body.set_wrap(true);
    body.set_wrap_mode(gtk4::pango::WrapMode::Word);
    body.set_selectable(true);
    body.set_xalign(0.0);
    body.set_margin_start(12);
    body.set_margin_end(12);
    body.set_margin_top(10);
    body.set_margin_bottom(10);
    card.append(&body);

    if is_agent {
        let actions = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
        actions.set_halign(gtk4::Align::End);
        let copy_btn = gtk4::Button::from_icon_name("edit-copy-symbolic");
        copy_btn.add_css_class("flat");
        copy_btn.set_tooltip_text(Some("Copy response"));
        let text_owned = text.to_string();
        copy_btn.connect_clicked(move |_| {
            if let Ok(mut cb) = arboard::Clipboard::new() {
                let _ = cb.set_text(&text_owned);
            }
        });
        actions.append(&copy_btn);
        card.append(&actions);
    }

    row.append(&card);
    transcript.append(&row);
}

/// Append an empty agent bubble and return its body label for token fills.
fn append_live_bubble(transcript: &gtk4::ListBox) -> gtk4::Label {
    let row = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    row.set_margin_top(6);
    row.set_margin_bottom(6);

    let name = gtk4::Label::new(Some("Assistant"));
    name.set_xalign(0.0);
    name.add_css_class("caption");
    name.add_css_class("dim-label");
    row.append(&name);

    let card = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    card.add_css_class("card");
    card.set_margin_bottom(2);
    let body = gtk4::Label::new(Some(""));
    body.set_wrap(true);
    body.set_wrap_mode(gtk4::pango::WrapMode::Word);
    body.set_selectable(true);
    body.set_xalign(0.0);
    body.set_margin_start(12);
    body.set_margin_end(12);
    body.set_margin_top(10);
    body.set_margin_bottom(10);
    card.append(&body);
    row.append(&card);
    transcript.append(&row);
    body
}

/// Ensure the live bubble exists (called right after send, before tokens).
fn st_live_bubble(state: &Arc<Mutex<ChatUi>>, transcript: &gtk4::ListBox) {
    let exists = state
        .lock()
        .map(|st| st.live_label.is_some())
        .unwrap_or(false);
    if !exists {
        let label = append_live_bubble(transcript);
        if let Ok(mut st) = state.lock() {
            st.live_label = Some(glib::SendWeakRef::from(label.downgrade()));
        }
    }
}

/// Replace the last assistant bubble text (placeholder "…" → final answer).
fn replace_last_assistant(transcript: &gtk4::ListBox, text: &str) {
    let mut last_body: Option<gtk4::Label> = None;
    let mut child = transcript.first_child();
    while let Some(row) = child {
        child = row.next_sibling();
        // Each top-level child is the vertical box; the card's label is
        // nested two levels down. Walk cheaply: last label wins.
        collect_labels(&row, &mut last_body);
    }
    if let Some(label) = last_body {
        label.set_text(text);
    } else {
        append_bubble(transcript, "Assistant", text, true);
    }
}

fn collect_labels(widget: &gtk4::Widget, out: &mut Option<gtk4::Label>) {
    if let Ok(label) = widget.clone().downcast::<gtk4::Label>() {
        if label.is_selectable() {
            *out = Some(label);
        }
        return;
    }
    let mut child = widget.first_child();
    while let Some(c) = child {
        child = c.next_sibling();
        collect_labels(&c, out);
    }
}
