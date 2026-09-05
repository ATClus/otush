//! Overlay state: statics, phases, window lifecycle, show/hide, level events. (split from `overlay.rs`; same behavior).

use super::paint::PillLayout;
use super::paint::{
    apply_surface_regions, draw_overlay, ensure_surface_transparent, get_monitor_geometry,
    overlay_css_provider, position_code, push_level, queue_pill_redraw,
};
use crate::context::{AppContext, AppEvent, EventBus};
use crate::settings::OverlayPosition;
use gtk4::prelude::*;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Whether the overlay is enabled at all (mirrors `OverlayStyle != None`).
pub(super) static OVERLAY_ENABLED: AtomicBool = AtomicBool::new(false);
/// True while the overlay is mapped (drives the pulsing ring animation).
pub(super) static OVERLAY_VISIBLE: AtomicBool = AtomicBool::new(false);
/// True while showing the streaming panel (live transcript visible).
pub(super) static OVERLAY_STREAMING: AtomicBool = AtomicBool::new(false);
/// Current anchor position (0 = bottom, 1 = top), mirroring `OverlayPosition`.
pub(super) static OVERLAY_POSITION: AtomicU8 = AtomicU8::new(0);
/// Current phase: 0 = Recording, 1 = Transcribing, 2 = Processing.
pub(super) static OVERLAY_PHASE: AtomicU8 = AtomicU8::new(0);
/// Whether the current state is recording (true) vs transcribing/processing (false).
pub(super) static OVERLAY_IS_RECORDING: AtomicBool = AtomicBool::new(true);

/// Stored state label text ("Recording… 0:00", "Transcribing…", "Processing…").
pub(super) static OVERLAY_STATE_TEXT: Mutex<String> = Mutex::new(String::new());
/// Stored live streaming transcript markup text.
pub(super) static OVERLAY_STREAM_MARKUP: Mutex<String> = Mutex::new(String::new());

/// Overlay display state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OverlayPhase {
    Recording = 0,
    Transcribing = 1,
    Processing = 2,
}

impl OverlayPhase {
    pub fn from_u8(code: u8) -> Self {
        match code {
            1 => OverlayPhase::Transcribing,
            2 => OverlayPhase::Processing,
            _ => OverlayPhase::Recording,
        }
    }

    pub fn to_u8(self) -> u8 {
        self as u8
    }
}

pub(super) static LAST_MIC_LEVEL_EMIT: AtomicU64 = AtomicU64::new(0);
pub(super) const EMIT_THROTTLE_MS: u64 = 33; // ~30 FPS

/// Recent mic levels (ring buffer) rendered as the live waveform.
pub(super) const LEVEL_HISTORY: usize = 20;
pub(super) static OVERLAY_LEVELS: OnceLock<Mutex<VecDeque<f32>>> = OnceLock::new();

/// Timestamp when recording started to display an elapsed timer.
pub(super) static RECORDING_START_INSTANT: Mutex<Option<std::time::Instant>> = Mutex::new(None);

/// Frame counter for the pulsing record ring (a ~40 ms main-loop timer).
pub(super) static PULSE_FRAME: AtomicU64 = AtomicU64::new(0);
pub(super) static PULSE_SOURCE: Mutex<Option<glib::SourceId>> = Mutex::new(None);

/// Pill geometry in screen pixels, recomputed on every draw/resize.
pub(super) static LAST_LAYOUT: Mutex<Option<PillLayout>> = Mutex::new(None);

/// The overlay window and its drawing area, reachable from any thread via
/// sendable weak refs (upgraded on the main thread).
pub(super) static OVERLAY_WINDOW: OnceLock<glib::SendWeakRef<gtk4::Window>> = OnceLock::new();
pub(super) static OVERLAY_DRAW_AREA: OnceLock<glib::SendWeakRef<gtk4::DrawingArea>> =
    OnceLock::new();

/// Pill dimensions (logical px).
pub(super) const PILL_W_COMPACT: i32 = 236;
pub(super) const PILL_W_STREAM: i32 = 384;
pub(super) const PILL_H_COMPACT: i32 = 64;
pub(super) const PILL_H_STREAM: i32 = 148;
/// Gap between the pill and the bottom screen edge (above the GNOME dash).
pub(super) const BOTTOM_MARGIN: i32 = 56;
pub(super) const TOP_MARGIN: i32 = 16;
pub(super) const BUTTON_SIZE: i32 = 44;
pub(super) const PAD: i32 = 12;
pub(super) const GAP: i32 = 10;

pub fn update_overlay_enabled_cache(enabled: bool) {
    OVERLAY_ENABLED.store(enabled, Ordering::Relaxed);
}

/// Create the overlay window (fullscreen transparent surface) and subscribe to
/// bus events. Called once from the GTK shell after the main window exists.
pub fn init_overlay(ctx: &AppContext, position: OverlayPosition) {
    let bus = ctx.bus.clone();
    glib::MainContext::default().invoke(move || {
        let provider = overlay_css_provider();

        let window = gtk4::Window::new();
        window.set_title(Some("Otush Overlay"));
        window.set_decorated(false);
        window.set_resizable(false);
        window.set_visible(false);
        // Never steal keyboard focus (a focused overlay would swallow the
        // paste chord).
        window.set_focusable(false);
        window.add_css_class("otush-overlay-window");
        window.remove_css_class("background");
        // Scope the transparent-background CSS to this window so the overlay
        // is never painted with an opaque backdrop, independent of the
        // display-wide provider.
        window
            .style_context()
            .add_provider(&provider, gtk4::STYLE_PROVIDER_PRIORITY_USER);
        OVERLAY_POSITION.store(position_code(position), Ordering::Relaxed);

        // Size to the monitor so the pill layout and drawing area cover the screen
        // without calling window.fullscreen() (which causes GNOME Mutter on Wayland
        // to paint an opaque black backdrop).
        if let Some(geo) = get_monitor_geometry(&window) {
            window.set_default_size(geo.width(), geo.height());
            log::info!(
                "overlay: backend={:?} monitor={}x{} window default size set",
                std::env::var("GDK_BACKEND").ok(),
                geo.width(),
                geo.height()
            );
        }

        // Full-window canvas: paints the pill, buttons, dynamic meters, and text.
        let draw_area = gtk4::DrawingArea::new();
        draw_area.set_hexpand(true);
        draw_area.set_vexpand(true);
        let window_weak = glib::SendWeakRef::from(window.downgrade());
        draw_area.set_draw_func(move |_, cr, w, h| {
            // The surface is guaranteed to exist while drawing, so this is a
            // reliable place to enforce click-through + alpha transparency.
            ensure_surface_transparent(&window_weak);
            draw_overlay(cr, w as f64, h as f64);
        });

        window.set_child(Some(&draw_area));

        // Every time the surface maps or realizes, make it click-through and truly
        // transparent: an empty input region means the window never steals pointer
        // events from the app below (the pill is display-only), and an empty opaque
        // region forces the compositor to use the alpha channel (otherwise the
        // "transparent" background composites as black).
        window.connect_map(|window| {
            apply_surface_regions(window);
        });
        window.connect_realize(|window| {
            apply_surface_regions(window);
        });

        let _ = OVERLAY_WINDOW.set(glib::SendWeakRef::from(window.downgrade()));
        let _ = OVERLAY_DRAW_AREA.set(glib::SendWeakRef::from(draw_area.downgrade()));

        // Live updates: mic level + streaming text.
        bus.subscribe(move |event| match event {
            AppEvent::MicLevel(level) => {
                glib::MainContext::default().invoke(move || {
                    push_level(level);
                    queue_pill_redraw();
                });
            }
            AppEvent::StreamText(text) => {
                let committed_escaped = glib::markup_escape_text(&text.committed);
                let tentative_escaped = glib::markup_escape_text(&text.tentative);
                let markup = if text.tentative.is_empty() {
                    format!("<span>{}</span>", committed_escaped)
                } else if text.committed.is_empty() {
                    format!(
                        "<span alpha='65%' style='italic'>{}</span><span alpha='75%' foreground='#38BDF8'> ▮</span>",
                        tentative_escaped
                    )
                } else {
                    format!(
                        "<span>{}</span><span alpha='65%' style='italic'> {}</span><span alpha='75%' foreground='#38BDF8'> ▮</span>",
                        committed_escaped, tentative_escaped
                    )
                };
                if let Ok(mut guard) = OVERLAY_STREAM_MARKUP.lock() {
                    *guard = markup;
                }
                glib::MainContext::default().invoke(queue_pill_redraw);
            }
            AppEvent::StreamPhase(_) => {
                OVERLAY_STREAMING.store(true, Ordering::Relaxed);
                glib::MainContext::default().invoke(queue_pill_redraw);
            }
            _ => {}
        });
    });
}

pub(super) fn upgrade<T: glib::object::ObjectType>(
    slot: &OnceLock<glib::SendWeakRef<T>>,
) -> Option<T> {
    let send_weak = slot.get()?.clone();
    send_weak.into_weak_ref().upgrade()
}

pub fn emit_recording_ready(_ctx: &AppContext) {}

pub(super) fn show_state(text: &str, phase: OverlayPhase, streaming: bool) {
    if !OVERLAY_ENABLED.load(Ordering::Relaxed) {
        log::debug!("overlay: show_state({text}) skipped (overlay disabled)");
        return;
    }
    OVERLAY_PHASE.store(phase.to_u8(), Ordering::Relaxed);
    OVERLAY_IS_RECORDING.store(phase == OverlayPhase::Recording, Ordering::Relaxed);
    OVERLAY_STREAMING.store(streaming, Ordering::Relaxed);

    if phase == OverlayPhase::Recording {
        if let Ok(mut guard) = RECORDING_START_INSTANT.lock() {
            if guard.is_none() {
                *guard = Some(std::time::Instant::now());
            }
        }
    } else if let Ok(mut guard) = RECORDING_START_INSTANT.lock() {
        *guard = None;
    }

    if let Ok(mut guard) = OVERLAY_STATE_TEXT.lock() {
        *guard = text.to_string();
    }

    glib::MainContext::default().invoke(move || {
        let Some(window) = upgrade(&OVERLAY_WINDOW) else {
            log::warn!("overlay: show_state skipped (window not initialized)");
            return;
        };
        // Size to the monitor so the pill layout and drawing area cover the screen.
        if let Some(geo) = get_monitor_geometry(&window) {
            window.set_default_size(geo.width(), geo.height());
        }
        window.present();
        apply_surface_regions(&window);
        queue_pill_redraw();
        OVERLAY_VISIBLE.store(true, Ordering::Relaxed);
        start_pulse();
        log::info!("overlay: showing state (phase={phase:?}, streaming={streaming})");
    });
}

pub fn show_recording_overlay(_ctx: &AppContext) {
    show_state("Recording…", OverlayPhase::Recording, false);
}

pub fn show_meeting_recording_overlay(_ctx: &AppContext) {
    show_state("Meeting Mode…", OverlayPhase::Recording, false);
}

pub fn show_streaming_overlay(_ctx: &AppContext) {
    show_state("Recording…", OverlayPhase::Recording, true);
}

pub fn show_meeting_streaming_overlay(_ctx: &AppContext) {
    show_state("Meeting Mode…", OverlayPhase::Recording, true);
}

pub fn show_transcribing_overlay(_ctx: &AppContext) {
    show_state("Transcribing…", OverlayPhase::Transcribing, false);
}

pub fn show_meeting_transcribing_overlay(_ctx: &AppContext) {
    show_state("Transcribing Meeting…", OverlayPhase::Transcribing, false);
}

pub fn show_processing_overlay(_ctx: &AppContext) {
    show_state("Processing…", OverlayPhase::Processing, false);
}

pub fn show_meeting_processing_overlay(_ctx: &AppContext) {
    show_state(
        "Formatting Meeting Minutes…",
        OverlayPhase::Processing,
        false,
    );
}

pub fn update_overlay_position(ctx: &AppContext) {
    let position = ctx.settings().overlay_position;
    glib::MainContext::default().invoke(move || {
        OVERLAY_POSITION.store(position_code(position), Ordering::Relaxed);
        queue_pill_redraw();
    });
}

pub fn hide_recording_overlay(_ctx: &AppContext) {
    glib::MainContext::default().invoke(move || {
        OVERLAY_VISIBLE.store(false, Ordering::Relaxed);
        OVERLAY_STREAMING.store(false, Ordering::Relaxed);
        if let Ok(mut guard) = OVERLAY_STREAM_MARKUP.lock() {
            guard.clear();
        }
        stop_pulse();
        if let Some(window) = upgrade(&OVERLAY_WINDOW) {
            window.hide();
        }
    });
}

/// Emit the mic level for the overlay waveform and audio settings VU meter,
/// throttled to ~30 FPS. Called from the audio callback thread.
pub fn emit_levels(bus: &EventBus, levels: &[f32]) {
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

// --- Pulsing record button ------------------------------------------------

pub(super) fn start_pulse() {
    let mut guard = PULSE_SOURCE.lock().unwrap_or_else(|e| e.into_inner());
    if guard.is_some() {
        return;
    }
    // 10 FPS: enough for the AI meter / waveform / 1 s timer text, and 2.5x
    // less fullscreen Cairo + compositor work than the previous 25 FPS.
    // Level-driven redraws (mic events) already arrive between ticks.
    *guard = Some(glib::timeout_add_local(Duration::from_millis(100), || {
        let _ = PULSE_FRAME.fetch_add(1, Ordering::Relaxed);
        if OVERLAY_VISIBLE.load(Ordering::Relaxed) {
            queue_pill_redraw();
        }
        glib::ControlFlow::Continue
    }));
}

pub(super) fn stop_pulse() {
    let mut guard = PULSE_SOURCE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(source) = guard.take() {
        source.remove();
    }
    if let Ok(mut start_guard) = RECORDING_START_INSTANT.lock() {
        *start_guard = None;
    }
}

/// Pulse phase in `0.0..1.0`, cycling every ~1.2 s at the 100 ms timer rate.
pub(super) fn pulse_phase() -> f64 {
    (PULSE_FRAME.load(Ordering::Relaxed) % 12) as f64 / 12.0
}
