//! Shared Text-to-Speech UI helpers: per-text "Read aloud" button with live
//! stop state, and a compact reader mini-player bound to
//! [`crate::context::AppEvent::TtsStateChanged`].
//!
//! Threading follows the project rule: backend work via
//! [`crate::runtime::spawn`], widget updates on the GTK main thread through
//! `glib::MainContext::default().invoke`.

use crate::commands::tts as tts_cmds;
use crate::context::{AppContext, AppEvent, TtsSource, TtsState};
use gtk4::prelude::*;
use std::sync::{Arc, Mutex};

/// Build a "Read aloud" button for `text` (reader mode).
///
/// While this button's utterance plays, its icon flips to stop and clicking
/// it stops playback. Other utterances do not affect this button (each
/// `speak_*` call invalidates the previous queue, so at most one plays).
pub fn read_aloud_button(ctx: &AppContext, text: &str) -> gtk4::Button {
    let btn = gtk4::Button::from_icon_name("media-playback-start-symbolic");
    btn.set_tooltip_text(Some("Read aloud"));
    btn.set_valign(gtk4::Align::Center);
    btn.add_css_class("flat");

    if tts_cmds::speakable_text(text).is_empty() {
        btn.set_sensitive(false);
        btn.set_tooltip_text(Some("Nothing to read"));
        return btn;
    }

    let text = text.to_string();
    let btn_weak = glib::SendWeakRef::from(btn.downgrade());
    let ctx_click = ctx.clone();
    btn.connect_clicked(move |_| {
        let ctx = ctx_click.clone();
        let text = text.clone();
        let btn_weak = btn_weak.clone();
        if tts_cmds::is_speaking() {
            tts_cmds::stop_speaking(&ctx, TtsSource::Reader);
            return;
        }
        crate::runtime::spawn(async move {
            let res = tts_cmds::speak_text(&ctx, text).await;
            if let Err(e) = res {
                let message = e.to_string();
                glib::MainContext::default().invoke(move || {
                    if let Some(btn) = btn_weak.into_weak_ref().upgrade() {
                        btn.set_tooltip_text(Some(&format!("Read failed: {message}")));
                    }
                });
                ctx.report_error("read_aloud", e);
            }
        });
    });

    // Flip icon while speaking.
    let btn_weak_bus = glib::SendWeakRef::from(btn.downgrade());
    let _sub = ctx.bus.subscribe(move |event| {
        let btn_weak_bus = btn_weak_bus.clone();
        if let AppEvent::TtsStateChanged(state) = event {
            let speaking = matches!(
                state,
                TtsState::Started { .. }
                    | TtsState::ChunkProgress { .. }
                    | TtsState::Resumed { .. }
            );
            glib::MainContext::default().invoke(move || {
                if let Some(btn) = btn_weak_bus.into_weak_ref().upgrade() {
                    btn.set_icon_name(if speaking {
                        "media-playback-stop-symbolic"
                    } else {
                        "media-playback-start-symbolic"
                    });
                    if !speaking {
                        btn.set_tooltip_text(Some("Read aloud"));
                    }
                }
            });
        }
    });
    // Leak the subscription with the button's lifetime: the bus holds a
    // `'static` callback, so tie teardown to widget destruction.
    let sub_holder = Arc::new(Mutex::new(Some(_sub)));
    btn.connect_destroy(move |_| {
        if let Some(sub) = sub_holder.lock().unwrap_or_else(|e| e.into_inner()).take() {
            sub.unsubscribe();
        }
    });

    btn
}

/// Build a per-message "Listen" button for one chat answer (chat mode).
///
/// Clicking speaks just this text (cancelling any previous speech);
/// clicking while chat audio plays stops it.
pub fn chat_speak_button(ctx: &AppContext, text: &str) -> gtk4::Button {
    let btn = gtk4::Button::from_icon_name("audio-speakers-symbolic");
    btn.add_css_class("flat");
    btn.set_tooltip_text(Some("Listen to response"));
    btn.set_valign(gtk4::Align::Center);

    if tts_cmds::speakable_text(text).is_empty() {
        btn.set_sensitive(false);
        btn.set_tooltip_text(Some("Nothing to read"));
        return btn;
    }

    let text = text.to_string();
    let ctx_click = ctx.clone();
    let btn_weak = glib::SendWeakRef::from(btn.downgrade());
    btn.connect_clicked(move |_| {
        let ctx = ctx_click.clone();
        if tts_cmds::is_speaking() {
            tts_cmds::stop_speaking(&ctx, TtsSource::Chat);
            return;
        }
        let text = text.clone();
        let btn_weak = btn_weak.clone();
        crate::runtime::spawn(async move {
            if let Err(e) = tts_cmds::speak_chat_message(&ctx, text).await {
                let message = e.to_string();
                glib::MainContext::default().invoke(move || {
                    if let Some(btn) = btn_weak.into_weak_ref().upgrade() {
                        btn.set_tooltip_text(Some(format!("Listen failed: {message}").as_str()));
                    }
                });
                ctx.report_error("chat_speak", e);
            }
        });
    });

    // Flip icon while chat audio plays.
    let btn_weak_bus = glib::SendWeakRef::from(btn.downgrade());
    let sub = ctx.bus.subscribe(move |event| {
        if let AppEvent::TtsStateChanged(state) = event {
            let for_chat = matches!(
                state,
                TtsState::Started { source, .. }
                    | TtsState::ChunkProgress { source, .. }
                    | TtsState::Paused { source }
                    | TtsState::Resumed { source, .. }
                    | TtsState::Stopped { source }
                    | TtsState::Error { source, .. }
                if source == TtsSource::Chat
            );
            if !for_chat {
                return;
            }
            let speaking = matches!(
                state,
                TtsState::Started { .. }
                    | TtsState::ChunkProgress { .. }
                    | TtsState::Resumed { .. }
            );
            let btn_weak_bus = btn_weak_bus.clone();
            glib::MainContext::default().invoke(move || {
                if let Some(btn) = btn_weak_bus.into_weak_ref().upgrade() {
                    btn.set_icon_name(if speaking {
                        "media-playback-stop-symbolic"
                    } else {
                        "audio-speakers-symbolic"
                    });
                }
            });
        }
    });
    let sub_holder = Arc::new(Mutex::new(Some(sub)));
    btn.connect_destroy(move |_| {
        if let Some(sub) = sub_holder.lock().unwrap_or_else(|e| e.into_inner()).take() {
            sub.unsubscribe();
        }
    });

    btn
}
///
/// `current_text` supplies the text to (re)start when idle; while an
/// utterance plays, the play button toggles pause/resume instead.
pub fn reader_mini_player(
    ctx: &AppContext,
    current_text: impl Fn() -> String + 'static,
) -> gtk4::Box {
    let bar = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    bar.set_hexpand(true);

    let play_btn = gtk4::Button::from_icon_name("media-playback-start-symbolic");
    play_btn.set_tooltip_text(Some("Read aloud"));
    play_btn.set_valign(gtk4::Align::Center);
    play_btn.add_css_class("flat");

    let stop_btn = gtk4::Button::from_icon_name("media-playback-stop-symbolic");
    stop_btn.set_tooltip_text(Some("Stop reading"));
    stop_btn.set_valign(gtk4::Align::Center);
    stop_btn.add_css_class("flat");
    stop_btn.set_sensitive(false);

    let progress = gtk4::Label::new(None);
    progress.set_halign(gtk4::Align::Start);
    progress.set_hexpand(true);
    progress.set_ellipsize(gtk4::pango::EllipsizeMode::End);

    bar.append(&play_btn);
    bar.append(&stop_btn);
    bar.append(&progress);

    let ctx_play = ctx.clone();
    let play_weak = glib::SendWeakRef::from(play_btn.downgrade());
    let stop_weak = glib::SendWeakRef::from(stop_btn.downgrade());
    play_btn.connect_clicked(move |_| {
        let ctx = ctx_play.clone();
        if tts_cmds::is_speaking() {
            if tts_cmds::is_paused() {
                tts_cmds::resume_speaking(&ctx, TtsSource::Reader);
            } else {
                tts_cmds::pause_speaking(&ctx, TtsSource::Reader);
            }
            return;
        }
        let text = current_text();
        if tts_cmds::speakable_text(&text).is_empty() {
            if let Some(btn) = play_weak.clone().into_weak_ref().upgrade() {
                btn.set_tooltip_text(Some("Nothing to read"));
            }
            return;
        }
        crate::runtime::spawn(async move {
            if let Err(e) = tts_cmds::speak_text(&ctx, text).await {
                ctx.report_error("read_aloud", e);
            }
        });
        let _ = &stop_weak;
    });

    let ctx_stop = ctx.clone();
    stop_btn.connect_clicked(move |_| {
        tts_cmds::stop_speaking(&ctx_stop, TtsSource::Reader);
    });

    // Live state binding.
    let play_weak_bus = glib::SendWeakRef::from(play_btn.downgrade());
    let stop_weak_bus = glib::SendWeakRef::from(stop_btn.downgrade());
    let progress_weak_bus = glib::SendWeakRef::from(progress.downgrade());
    let _sub = ctx.bus.subscribe(move |event| {
        if let AppEvent::TtsStateChanged(state) = event {
            let (speaking, paused, label) = match &state {
                TtsState::Started { total, .. } => (true, false, format!("Reading… 0/{total}")),
                TtsState::ChunkProgress { done, total, .. } => {
                    (true, false, format!("Reading… {done}/{total}"))
                }
                TtsState::Paused { .. } => (true, true, "Paused".to_string()),
                TtsState::Resumed { .. } => (true, false, "Reading…".to_string()),
                TtsState::Stopped { .. } => (false, false, String::new()),
                TtsState::Error { message, .. } => (false, false, format!("Error: {message}")),
            };
            // Reader-only: ignore chat utterances.
            let for_reader = match &state {
                TtsState::Started { source, .. }
                | TtsState::ChunkProgress { source, .. }
                | TtsState::Paused { source }
                | TtsState::Resumed { source }
                | TtsState::Stopped { source }
                | TtsState::Error { source, .. } => *source == TtsSource::Reader,
            };
            if !for_reader {
                return;
            }
            let play_weak_bus = play_weak_bus.clone();
            let stop_weak_bus = stop_weak_bus.clone();
            let progress_weak_bus = progress_weak_bus.clone();
            glib::MainContext::default().invoke(move || {
                if let Some(play) = play_weak_bus.into_weak_ref().upgrade() {
                    play.set_icon_name(if speaking && !paused {
                        "media-playback-pause-symbolic"
                    } else {
                        "media-playback-start-symbolic"
                    });
                    play.set_tooltip_text(Some(if !speaking {
                        "Read aloud"
                    } else if paused {
                        "Resume reading"
                    } else {
                        "Pause reading"
                    }));
                }
                if let Some(stop) = stop_weak_bus.into_weak_ref().upgrade() {
                    stop.set_sensitive(speaking);
                }
                if let Some(progress) = progress_weak_bus.into_weak_ref().upgrade() {
                    progress.set_text(&label);
                }
            });
        }
    });
    let sub_holder = Arc::new(Mutex::new(Some(_sub)));
    bar.connect_destroy(move |_| {
        if let Some(sub) = sub_holder.lock().unwrap_or_else(|e| e.into_inner()).take() {
            sub.unsubscribe();
        }
    });

    bar
}
