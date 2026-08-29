//! Recording overlay — a Wayland layer-shell surface (GTK4 + gtk4-layer-shell).
//!
//! A small always-on-top surface anchored to the top/bottom screen edge that
//! shows the recording state, a live mic-level bar and (in streaming mode) the
//! live transcript. The window lives on the GTK main thread; every public
//! entry point marshals onto it via `glib::MainContext::invoke`.

use crate::context::{AppContext, AppEvent, EventBus};
use crate::settings::{OverlayPosition, OverlayStyle};
use gtk4::prelude::*;
use gtk4_layer_shell::LayerShell;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

/// Whether the overlay is enabled at all (mirrors `OverlayStyle != None`).
static OVERLAY_ENABLED: AtomicBool = AtomicBool::new(false);
/// True when the overlay is a real layer-shell surface; false when it falls
/// back to a plain always-on-top window (no layer-shell support).
static OVERLAY_IS_LAYER_SHELL: AtomicBool = AtomicBool::new(true);

static LAST_MIC_LEVEL_EMIT: AtomicU64 = AtomicU64::new(0);
const EMIT_THROTTLE_MS: u64 = 33; // ~30 FPS

/// Overlay window size (logical px) per state: compact pill vs streaming panel.
const COMPACT_WIDTH: i32 = 256;
const COMPACT_HEIGHT: i32 = 46;
const STREAM_WIDTH: i32 = 400;
const STREAM_HEIGHT: i32 = 120;

/// The layer-shell window and its child widgets, reachable from any thread via
/// sendable weak refs (upgraded on the main thread).
static OVERLAY_WINDOW: OnceLock<glib::SendWeakRef<gtk4::Window>> = OnceLock::new();
static OVERLAY_STATE_LABEL: OnceLock<glib::SendWeakRef<gtk4::Label>> = OnceLock::new();
static OVERLAY_LEVEL_BAR: OnceLock<glib::SendWeakRef<gtk4::ProgressBar>> = OnceLock::new();
static OVERLAY_TEXT_LABEL: OnceLock<glib::SendWeakRef<gtk4::Label>> = OnceLock::new();

/// Keep the cached overlay-enabled flag in sync so `emit_levels` stops (or
/// resumes) emitting on the next audio callback.
pub fn update_overlay_enabled_cache(enabled: bool) {
    OVERLAY_ENABLED.store(enabled, Ordering::Relaxed);
}

/// Create the overlay window (layer-shell surface, or a plain undecorated
/// window when the compositor lacks layer-shell) and subscribe to bus events.
/// Called once from the GTK shell after the main window exists.
pub fn init_overlay(ctx: &AppContext, position: OverlayPosition) {
    let bus = ctx.bus.clone();
    glib::MainContext::default().invoke(move || {
        let window = gtk4::Window::new();
        window.set_title(Some("Otush Overlay"));
        window.set_decorated(false);
        window.set_resizable(false);
        window.set_visible(false);
        // Never steal keyboard focus (a focused overlay would swallow the
        // paste chord). Layer-shell windows also use KeyboardMode::None.
        window.set_focusable(false);

        if gtk4_layer_shell::is_supported() {
            window.init_layer_shell();
            window.set_layer(gtk4_layer_shell::Layer::Overlay);
            window.set_keyboard_mode(gtk4_layer_shell::KeyboardMode::None);
            window.set_exclusive_zone(0);
            OVERLAY_IS_LAYER_SHELL.store(true, Ordering::Relaxed);
            log::info!("overlay: initialized as a layer-shell surface");
        } else {
            // Fallback: a plain undecorated window (layer-shell unavailable).
            OVERLAY_IS_LAYER_SHELL.store(false, Ordering::Relaxed);
            log::warn!(
                "gtk-layer-shell not supported; using a plain undecorated window for the overlay"
            );
        }

        let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
        vbox.set_margin_top(6);
        vbox.set_margin_bottom(6);
        vbox.set_margin_start(12);
        vbox.set_margin_end(12);

        let state_label = gtk4::Label::new(Some(""));
        state_label.add_css_class("heading");
        vbox.append(&state_label);

        let level_bar = gtk4::ProgressBar::new();
        level_bar.set_fraction(0.0);
        level_bar.set_show_text(false);
        vbox.append(&level_bar);

        let text_label = gtk4::Label::new(Some(""));
        text_label.set_wrap(true);
        text_label.set_justify(gtk4::Justification::Center);
        text_label.set_visible(false);
        vbox.append(&text_label);

        window.set_child(Some(&vbox));
        apply_position(&window, position);
        window.set_size_request(COMPACT_WIDTH, COMPACT_HEIGHT);

        let _ = OVERLAY_WINDOW.set(glib::SendWeakRef::from(window.downgrade()));
        let _ = OVERLAY_STATE_LABEL.set(glib::SendWeakRef::from(state_label.downgrade()));
        let _ = OVERLAY_LEVEL_BAR.set(glib::SendWeakRef::from(level_bar.downgrade()));
        let _ = OVERLAY_TEXT_LABEL.set(glib::SendWeakRef::from(text_label.downgrade()));

        // Live updates: mic level + streaming text.
        bus.subscribe(move |event| match event {
            AppEvent::MicLevel(level) => {
                glib::MainContext::default().invoke(move || {
                    if let Some(bar) = upgrade(&OVERLAY_LEVEL_BAR) {
                        bar.set_fraction(level as f64);
                    }
                });
            }
            AppEvent::StreamText(text) => {
                glib::MainContext::default().invoke(move || {
                    if let Some(label) = upgrade(&OVERLAY_TEXT_LABEL) {
                        label.set_text(&format!("{}{}", text.committed, text.tentative));
                    }
                });
            }
            AppEvent::StreamPhase(_) => {
                glib::MainContext::default().invoke(move || {
                    if let Some(window) = upgrade(&OVERLAY_WINDOW) {
                        window.set_size_request(STREAM_WIDTH, STREAM_HEIGHT);
                    }
                });
            }
            _ => {}
        });
    });
}

fn upgrade<T: glib::object::ObjectType>(slot: &OnceLock<glib::SendWeakRef<T>>) -> Option<T> {
    let send_weak = slot.get()?.clone();
    send_weak.into_weak_ref().upgrade()
}

fn apply_position(window: &gtk4::Window, position: OverlayPosition) {
    if OVERLAY_IS_LAYER_SHELL.load(Ordering::Relaxed) {
        match position {
            OverlayPosition::Top => {
                window.set_anchor(gtk4_layer_shell::Edge::Top, true);
                window.set_anchor(gtk4_layer_shell::Edge::Bottom, false);
                window.set_margin(gtk4_layer_shell::Edge::Top, 4);
            }
            OverlayPosition::Bottom => {
                window.set_anchor(gtk4_layer_shell::Edge::Bottom, true);
                window.set_anchor(gtk4_layer_shell::Edge::Top, false);
                window.set_margin(gtk4_layer_shell::Edge::Bottom, 40);
            }
        }
    } else {
        // Plain-window fallback: no programmatic positioning in GTK4
        // (window.move was removed); the window appears at the compositor's
        // default placement.
        let _ = position;
    }
}

// --- Public UI entry points (called from any thread; marshaled to the main
// --- thread). All no-op when the overlay is disabled or not initialized.

pub fn emit_recording_ready(_ctx: &AppContext) {}

fn show_state(text: &str, streaming: bool) {
    if !OVERLAY_ENABLED.load(Ordering::Relaxed) {
        log::debug!("overlay: show_state({text}) skipped (overlay disabled)");
        return;
    }
    let text = text.to_string();
    glib::MainContext::default().invoke(move || {
        let Some(window) = upgrade(&OVERLAY_WINDOW) else {
            log::warn!("overlay: show_state({text}) skipped (window not initialized)");
            return;
        };
        if let Some(label) = upgrade(&OVERLAY_STATE_LABEL) {
            label.set_text(&text);
        }
        if let Some(text_label) = upgrade(&OVERLAY_TEXT_LABEL) {
            text_label.set_visible(streaming);
        }
        if streaming {
            window.set_size_request(STREAM_WIDTH, STREAM_HEIGHT);
        } else {
            window.set_size_request(COMPACT_WIDTH, COMPACT_HEIGHT);
        }
        // NOTE: gtk_window_present() is a no-op for layer-shell surfaces; only
        // show() maps them (gtk-layer-shell documented limitation).
        window.show();
        log::info!("overlay: showing '{text}' (streaming={streaming})");
    });
}

pub fn show_recording_overlay(_ctx: &AppContext) {
    show_state("Recording…", false);
}

pub fn show_streaming_overlay(_ctx: &AppContext) {
    show_state("Recording…", true);
}

pub fn show_transcribing_overlay(_ctx: &AppContext) {
    show_state("Transcribing…", false);
}

pub fn show_processing_overlay(_ctx: &AppContext) {
    show_state("Processing…", false);
}

pub fn update_overlay_position(ctx: &AppContext) {
    let position = ctx.settings().overlay_position;
    glib::MainContext::default().invoke(move || {
        if let Some(window) = upgrade(&OVERLAY_WINDOW) {
            apply_position(&window, position);
        }
    });
}

pub fn hide_recording_overlay(_ctx: &AppContext) {
    glib::MainContext::default().invoke(move || {
        if let Some(window) = upgrade(&OVERLAY_WINDOW) {
            window.hide();
        }
    });
}

/// Emit the mic level for the overlay level meter, throttled to ~30 FPS and
/// gated on the overlay being enabled. Called from the audio callback thread.
pub fn emit_levels(bus: &EventBus, levels: &[f32]) {
    if !OVERLAY_ENABLED.load(Ordering::Relaxed) {
        return;
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    if now.saturating_sub(LAST_MIC_LEVEL_EMIT.load(Ordering::Relaxed)) < EMIT_THROTTLE_MS {
        return;
    }
    LAST_MIC_LEVEL_EMIT.store(now, Ordering::Relaxed);

    let level = levels.iter().copied().fold(0.0f32, f32::max);
    bus.send(AppEvent::MicLevel(level));
}

// Keep the OverlayStyle reference for callers that pass it (API stability).
#[allow(dead_code)]
fn _style_used(style: OverlayStyle) -> bool {
    style != OverlayStyle::None
}
