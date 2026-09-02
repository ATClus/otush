//! Recording overlay — a fullscreen, transparent, click-through surface with a
//! compact pill painted at the chosen screen edge (Whisper Flow style).
//!
//! GNOME/Mutter does not implement the wlr-layer-shell protocol, and Wayland
//! clients cannot position toplevel windows, so a regular undecorated window
//! would always appear centered by the compositor. Instead the overlay is a
//! fullscreen transparent window: the pill is drawn with Cairo at the exact
//! screen position we want (bottom-center by default, top-center when
//! configured), and an empty input region makes every pixel outside the pill
//! click-through. The window is only mapped while recording/transcribing.

use crate::context::{AppContext, AppEvent, EventBus};
use crate::settings::OverlayPosition;
use gtk4::cairo;
use gtk4::prelude::*;
use pangocairo::pango;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Whether the overlay is enabled at all (mirrors `OverlayStyle != None`).
static OVERLAY_ENABLED: AtomicBool = AtomicBool::new(false);
/// True while the overlay is mapped (drives the pulsing ring animation).
static OVERLAY_VISIBLE: AtomicBool = AtomicBool::new(false);
/// True while showing the streaming panel (live transcript visible).
static OVERLAY_STREAMING: AtomicBool = AtomicBool::new(false);
/// Current anchor position (0 = bottom, 1 = top), mirroring `OverlayPosition`.
static OVERLAY_POSITION: AtomicU8 = AtomicU8::new(0);
/// Current phase: 0 = Recording, 1 = Transcribing, 2 = Processing.
static OVERLAY_PHASE: AtomicU8 = AtomicU8::new(0);
/// Whether the current state is recording (true) vs transcribing/processing (false).
static OVERLAY_IS_RECORDING: AtomicBool = AtomicBool::new(true);

/// Stored state label text ("Recording… 0:00", "Transcribing…", "Processing…").
static OVERLAY_STATE_TEXT: Mutex<String> = Mutex::new(String::new());
/// Stored live streaming transcript markup text.
static OVERLAY_STREAM_MARKUP: Mutex<String> = Mutex::new(String::new());

/// Overlay display state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverlayPhase {
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

static LAST_MIC_LEVEL_EMIT: AtomicU64 = AtomicU64::new(0);
const EMIT_THROTTLE_MS: u64 = 33; // ~30 FPS

/// Recent mic levels (ring buffer) rendered as the live waveform.
const LEVEL_HISTORY: usize = 20;
static OVERLAY_LEVELS: OnceLock<Mutex<VecDeque<f32>>> = OnceLock::new();

/// Timestamp when recording started to display an elapsed timer.
static RECORDING_START_INSTANT: Mutex<Option<std::time::Instant>> = Mutex::new(None);

/// Frame counter for the pulsing record ring (a ~40 ms main-loop timer).
static PULSE_FRAME: AtomicU64 = AtomicU64::new(0);
static PULSE_SOURCE: Mutex<Option<glib::SourceId>> = Mutex::new(None);

/// Pill geometry in screen pixels, recomputed on every draw/resize.
static LAST_LAYOUT: Mutex<Option<PillLayout>> = Mutex::new(None);

/// The overlay window and its drawing area, reachable from any thread via
/// sendable weak refs (upgraded on the main thread).
static OVERLAY_WINDOW: OnceLock<glib::SendWeakRef<gtk4::Window>> = OnceLock::new();
static OVERLAY_DRAW_AREA: OnceLock<glib::SendWeakRef<gtk4::DrawingArea>> = OnceLock::new();

/// Pill dimensions (logical px).
const PILL_W_COMPACT: i32 = 236;
const PILL_W_STREAM: i32 = 384;
const PILL_H_COMPACT: i32 = 64;
const PILL_H_STREAM: i32 = 148;
/// Gap between the pill and the bottom screen edge (above the GNOME dash).
const BOTTOM_MARGIN: i32 = 56;
const TOP_MARGIN: i32 = 16;
const BUTTON_SIZE: i32 = 44;
const PAD: i32 = 12;
const GAP: i32 = 10;

/// Geometry of the floating pill (screen coordinates).
#[derive(Clone, Copy, Debug)]
struct PillLayout {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    btn_x: i32,
    btn_y: i32,
    state_x: i32,
    state_y: i32,
    wave_x: i32,
    wave_y: i32,
    wave_w: i32,
    wave_h: i32,
    text_x: i32,
    text_y: i32,
    text_w: i32,
}

/// Keep the cached overlay-enabled flag in sync so `emit_levels` stops (or
/// resumes) emitting on the next audio callback.
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
                    if let Some(area) = upgrade(&OVERLAY_DRAW_AREA) {
                        area.queue_draw();
                    }
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
                glib::MainContext::default().invoke(move || {
                    if let Some(area) = upgrade(&OVERLAY_DRAW_AREA) {
                        area.queue_draw();
                    }
                });
            }
            AppEvent::StreamPhase(_) => {
                OVERLAY_STREAMING.store(true, Ordering::Relaxed);
                glib::MainContext::default().invoke(move || {
                    if let Some(area) = upgrade(&OVERLAY_DRAW_AREA) {
                        area.queue_draw();
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

fn position_code(position: OverlayPosition) -> u8 {
    match position {
        OverlayPosition::Top => 1,
        OverlayPosition::Bottom => 0,
    }
}

fn get_monitor_geometry(window: &gtk4::Window) -> Option<gtk4::gdk::Rectangle> {
    let display = gtk4::gdk::Display::default()?;
    if let Some(surface) = window.surface() {
        if let Some(monitor) = display.monitor_at_surface(&surface) {
            return Some(monitor.geometry());
        }
    }
    let monitor = display
        .monitors()
        .item(0)
        .and_downcast::<gtk4::gdk::Monitor>()?;
    Some(monitor.geometry())
}

/// Clear the overlay surface's input and opaque regions so the fullscreen
/// window is click-through and rendered with the alpha channel (transparent
/// background). Re-applied from the map handler and after each show in case
/// the compositor/surface recreated the regions.
fn apply_surface_regions(window: &gtk4::Window) {
    if let Some(surface) = window.surface() {
        surface.set_input_region(Some(&cairo::Region::create()));
        surface.set_opaque_region(Some(&cairo::Region::create()));
        log::debug!(
            "overlay: cleared input+opaque regions on surface {}x{}",
            surface.width(),
            surface.height()
        );
    } else {
        log::debug!("overlay: no surface to clear regions on (not realized yet)");
    }
}

/// Whether the overlay surface regions have been cleared at least once.
static SURFACE_TRANSPARENT_DONE: AtomicBool = AtomicBool::new(false);

/// Enforce click-through + transparency as soon as the surface is available
/// (called from the draw path, which runs after the window is realized).
fn ensure_surface_transparent(window_weak: &glib::SendWeakRef<gtk4::Window>) {
    if SURFACE_TRANSPARENT_DONE.load(Ordering::Relaxed) {
        return;
    }
    if let Some(window) = window_weak.clone().into_weak_ref().upgrade() {
        if window.surface().is_some() {
            apply_surface_regions(&window);
            SURFACE_TRANSPARENT_DONE.store(true, Ordering::Relaxed);
        }
    }
}

fn position_from_code(code: u8) -> OverlayPosition {
    if code == 1 {
        OverlayPosition::Top
    } else {
        OverlayPosition::Bottom
    }
}

// --- Stylesheet -----------------------------------------------------------

/// The overlay stylesheet, applied app-wide (via the display) and scoped to
/// the overlay window (via its own style context) so the transparent
/// background can never be missed if the display is not yet available when the
/// window is created.
fn overlay_css_provider() -> gtk4::CssProvider {
    static CSS_LOADED: AtomicBool = AtomicBool::new(false);
    let provider = gtk4::CssProvider::new();
    provider.load_from_data(include_str!("overlay.css"));
    if !CSS_LOADED.swap(true, Ordering::Relaxed) {
        if let Some(display) = gtk4::gdk::Display::default() {
            gtk4::style_context_add_provider_for_display(
                &display,
                &provider,
                gtk4::STYLE_PROVIDER_PRIORITY_USER,
            );
        }
    }
    provider
}

// --- Layout ---------------------------------------------------------------

fn compute_layout(
    screen_w: i32,
    screen_h: i32,
    streaming: bool,
    position: OverlayPosition,
) -> PillLayout {
    let w = if streaming {
        PILL_W_STREAM
    } else {
        PILL_W_COMPACT
    };
    let h = if streaming {
        PILL_H_STREAM
    } else {
        PILL_H_COMPACT
    };
    let x = (screen_w - w).max(0) / 2;
    let y = match position {
        OverlayPosition::Top => TOP_MARGIN,
        OverlayPosition::Bottom => (screen_h - h - BOTTOM_MARGIN).max(TOP_MARGIN),
    };
    let btn_x = x + PAD;
    let btn_y = y + PAD;
    let state_x = btn_x + BUTTON_SIZE + GAP;
    let state_y = y + PAD + 4;
    let wave_x = state_x;
    let wave_y = state_y + 22;
    let wave_w = (x + w - PAD - wave_x).max(40);
    let wave_h = 12;
    let (text_x, text_y, text_w) = if streaming {
        (x + PAD, y + PAD + BUTTON_SIZE + 12, w - 2 * PAD)
    } else {
        (0, 0, 0)
    };
    PillLayout {
        x,
        y,
        w,
        h,
        btn_x,
        btn_y,
        state_x,
        state_y,
        wave_x,
        wave_y,
        wave_w,
        wave_h,
        text_x,
        text_y,
        text_w,
    }
}

fn draw_overlay(cr: &cairo::Context, w: f64, h: f64) {
    // Clear entire surface to fully transparent (alpha = 0) so the desktop
    // and background applications show through outside the pill.
    let _ = cr.save();
    cr.set_operator(cairo::Operator::Clear);
    let _ = cr.paint();
    let _ = cr.restore();
    cr.set_operator(cairo::Operator::Over);

    let streaming = OVERLAY_STREAMING.load(Ordering::Relaxed);
    let position = position_from_code(OVERLAY_POSITION.load(Ordering::Relaxed));
    let layout = compute_layout(w as i32, h as i32, streaming, position);
    if let Ok(mut guard) = LAST_LAYOUT.lock() {
        *guard = Some(layout);
    }

    let (x, y, pw, ph) = (
        layout.x as f64,
        layout.y as f64,
        layout.w as f64,
        layout.h as f64,
    );

    // Soft drop shadow behind the card.
    rounded_rect_path(cr, x - 5.0, y - 5.0, pw + 10.0, ph + 10.0, 20.0);
    cr.set_source_rgba(0.0, 0.0, 0.0, 0.28);
    let _ = cr.fill();

    let phase = OverlayPhase::from_u8(OVERLAY_PHASE.load(Ordering::Relaxed));

    // Outer subtle phase-tinted glow behind card
    rounded_rect_path(cr, x - 1.5, y - 1.5, pw + 3.0, ph + 3.0, 19.0);
    match phase {
        OverlayPhase::Recording => cr.set_source_rgba(1.0, 0.28, 0.35, 0.14),
        OverlayPhase::Transcribing => cr.set_source_rgba(0.23, 0.51, 0.96, 0.14),
        OverlayPhase::Processing => cr.set_source_rgba(0.68, 0.38, 1.0, 0.18),
    }
    let _ = cr.fill();

    // Card background.
    rounded_rect_path(cr, x, y, pw, ph, 18.0);
    cr.set_source_rgba(0.086, 0.086, 0.11, 0.90);
    let _ = cr.fill();

    // Phase-tinted hairline border.
    rounded_rect_path(cr, x + 0.5, y + 0.5, pw - 1.0, ph - 1.0, 17.5);
    match phase {
        OverlayPhase::Recording => cr.set_source_rgba(1.0, 0.32, 0.40, 0.24),
        OverlayPhase::Transcribing => cr.set_source_rgba(0.38, 0.63, 0.95, 0.24),
        OverlayPhase::Processing => cr.set_source_rgba(0.72, 0.42, 1.0, 0.28),
    }
    cr.set_line_width(1.0);
    let _ = cr.stroke();

    // Record / State button icon.
    let cx = layout.btn_x as f64 + BUTTON_SIZE as f64 / 2.0;
    let cy = layout.btn_y as f64 + BUTTON_SIZE as f64 / 2.0;
    draw_state_button(cr, cx, cy, BUTTON_SIZE as f64 / 2.0 - 4.0, phase);

    // Dynamic right-side meter: audio waveform for recording, AI neural pulse for processing.
    let pulse = pulse_phase();
    match phase {
        OverlayPhase::Recording | OverlayPhase::Transcribing => {
            let levels = snapshot_levels();
            draw_waveform(
                cr,
                layout.wave_x as f64,
                layout.wave_y as f64,
                layout.wave_w as f64,
                layout.wave_h as f64,
                &levels,
            );
        }
        OverlayPhase::Processing => {
            draw_ai_processing_meter(
                cr,
                layout.wave_x as f64,
                layout.wave_y as f64,
                layout.wave_w as f64,
                layout.wave_h as f64,
                pulse,
            );
        }
    }

    // State text ("Recording… 0:00", "Transcribing…", "Processing…")
    let state_text = {
        let label_prefix = OVERLAY_STATE_TEXT
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default();
        let label = if label_prefix.is_empty() {
            "Recording".to_string()
        } else {
            label_prefix.trim_end_matches('…').to_string()
        };

        if phase == OverlayPhase::Recording {
            if let Ok(guard) = RECORDING_START_INSTANT.lock() {
                if let Some(start) = *guard {
                    let secs = start.elapsed().as_secs();
                    let mins = secs / 60;
                    let rem_secs = secs % 60;
                    format!("{label} {:01}:{:02}", mins, rem_secs)
                } else {
                    format!("{label} 0:00")
                }
            } else {
                format!("{label} 0:00")
            }
        } else if let Ok(guard) = OVERLAY_STATE_TEXT.lock() {
            if guard.is_empty() {
                match phase {
                    OverlayPhase::Transcribing => "Transcribing…".to_string(),
                    OverlayPhase::Processing => "Processing…".to_string(),
                    OverlayPhase::Recording => "Recording 0:00".to_string(),
                }
            } else {
                guard.clone()
            }
        } else {
            match phase {
                OverlayPhase::Transcribing => "Transcribing…".to_string(),
                OverlayPhase::Processing => "Processing…".to_string(),
                OverlayPhase::Recording => "Recording 0:00".to_string(),
            }
        }
    };

    draw_pango_text(
        cr,
        layout.state_x as f64,
        layout.state_y as f64,
        &state_text,
        TextStyle {
            font: "Inter, Cantarell, Sans Semi-Bold 10.5",
            color: (1.0, 1.0, 1.0, 0.96),
            is_markup: false,
            max_width: None,
        },
    );

    // Live transcript streaming text (when streaming is enabled)
    if streaming {
        let markup = {
            if let Ok(guard) = OVERLAY_STREAM_MARKUP.lock() {
                guard.clone()
            } else {
                String::new()
            }
        };
        if !markup.is_empty() && layout.text_w > 0 {
            draw_pango_text(
                cr,
                layout.text_x as f64,
                layout.text_y as f64,
                &markup,
                TextStyle {
                    font: "Inter, Cantarell, Sans 11",
                    color: (1.0, 1.0, 1.0, 0.92),
                    is_markup: true,
                    max_width: Some(layout.text_w),
                },
            );
        }
    }
}

struct TextStyle<'a> {
    font: &'a str,
    color: (f64, f64, f64, f64),
    is_markup: bool,
    max_width: Option<i32>,
}

fn draw_pango_text(cr: &cairo::Context, x: f64, y: f64, text: &str, style: TextStyle<'_>) {
    let layout = pangocairo::functions::create_layout(cr);
    let font_desc = pango::FontDescription::from_string(style.font);
    layout.set_font_description(Some(&font_desc));

    if style.is_markup {
        layout.set_markup(text);
    } else {
        layout.set_text(text);
    }

    if let Some(w) = style.max_width {
        layout.set_width(w * pango::SCALE);
        layout.set_wrap(pango::WrapMode::WordChar);
        layout.set_ellipsize(pango::EllipsizeMode::Start);
        layout.set_alignment(pango::Alignment::Center);
    }

    let _ = cr.save();
    cr.move_to(x, y);
    cr.set_source_rgba(style.color.0, style.color.1, style.color.2, style.color.3);
    pangocairo::functions::show_layout(cr, &layout);
    let _ = cr.restore();
}

// --- Public UI entry points (called from any thread; marshaled to the main
// --- thread). All no-op when the overlay is disabled or not initialized.

pub fn emit_recording_ready(_ctx: &AppContext) {}

fn show_state(text: &str, phase: OverlayPhase, streaming: bool) {
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
        if let Some(area) = upgrade(&OVERLAY_DRAW_AREA) {
            area.queue_draw();
        }
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
        if let Some(area) = upgrade(&OVERLAY_DRAW_AREA) {
            area.queue_draw();
        }
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

fn start_pulse() {
    let mut guard = PULSE_SOURCE.lock().unwrap_or_else(|e| e.into_inner());
    if guard.is_some() {
        return;
    }
    *guard = Some(glib::timeout_add_local(Duration::from_millis(40), || {
        let _ = PULSE_FRAME.fetch_add(1, Ordering::Relaxed);
        if OVERLAY_VISIBLE.load(Ordering::Relaxed) {
            if let Some(area) = upgrade(&OVERLAY_DRAW_AREA) {
                area.queue_draw();
            }
        }
        glib::ControlFlow::Continue
    }));
}

fn stop_pulse() {
    let mut guard = PULSE_SOURCE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(source) = guard.take() {
        source.remove();
    }
    if let Ok(mut start_guard) = RECORDING_START_INSTANT.lock() {
        *start_guard = None;
    }
}

/// Pulse phase in `0.0..1.0`, cycling every ~1.2 s at the 40 ms timer rate.
fn pulse_phase() -> f64 {
    (PULSE_FRAME.load(Ordering::Relaxed) % 30) as f64 / 30.0
}

thread_local! {
    static RECORDING_SURFACE: RefCell<Option<cairo::ImageSurface>> = const { RefCell::new(None) };
    static TRANSCRIBING_SURFACE: RefCell<Option<cairo::ImageSurface>> = const { RefCell::new(None) };
}

fn with_icon_surface<R>(
    is_recording: bool,
    f: impl FnOnce(&cairo::ImageSurface) -> R,
) -> Option<R> {
    if is_recording {
        RECORDING_SURFACE.with(|cell| {
            let mut opt = cell.borrow_mut();
            if opt.is_none() {
                *opt = load_cairo_image(include_bytes!("../resources/recording.png"));
            }
            opt.as_ref().map(f)
        })
    } else {
        TRANSCRIBING_SURFACE.with(|cell| {
            let mut opt = cell.borrow_mut();
            if opt.is_none() {
                *opt = load_cairo_image(include_bytes!("../resources/transcribing.png"));
            }
            opt.as_ref().map(f)
        })
    }
}

fn load_cairo_image(bytes: &[u8]) -> Option<cairo::ImageSurface> {
    let img = image::load_from_memory(bytes).ok()?.into_rgba8();
    let (w, h) = img.dimensions();
    let mut surface =
        cairo::ImageSurface::create(cairo::Format::ARgb32, w as i32, h as i32).ok()?;
    {
        let mut data = surface.data().ok()?;
        let src = img.as_raw();
        // Convert RGBA to native premultiplied ARGB32 for Cairo
        for i in 0..(w * h) as usize {
            let r = src[i * 4] as u32;
            let g = src[i * 4 + 1] as u32;
            let b = src[i * 4 + 2] as u32;
            let a = src[i * 4 + 3] as u32;
            if a == 0 {
                data[i * 4] = 0;
                data[i * 4 + 1] = 0;
                data[i * 4 + 2] = 0;
                data[i * 4 + 3] = 0;
            } else {
                let pr = (r * a + 127) / 255;
                let pg = (g * a + 127) / 255;
                let pb = (b * a + 127) / 255;
                data[i * 4] = pb as u8;
                data[i * 4 + 1] = pg as u8;
                data[i * 4 + 2] = pr as u8;
                data[i * 4 + 3] = a as u8;
            }
        }
    }
    Some(surface)
}

fn paint_surface_at(cr: &cairo::Context, surface: &cairo::ImageSurface, cx: f64, cy: f64, r: f64) {
    let size = (r * 2.0).round();
    let x = (cx - size / 2.0).round();
    let y = (cy - size / 2.0).round();
    let sw = surface.width() as f64;
    let sh = surface.height() as f64;
    let _ = cr.save();
    cr.translate(x, y);
    cr.scale(size / sw, size / sh);
    let _ = cr.set_source_surface(surface, 0.0, 0.0);
    let _ = cr.paint();
    let _ = cr.restore();
}

fn draw_state_button(cr: &cairo::Context, cx: f64, cy: f64, r: f64, phase: OverlayPhase) {
    // Expanding, fading pulse rings while the overlay is visible.
    let pulse = pulse_phase();
    for (offset, alpha) in [(0.0, 0.5), (0.4, 0.3)] {
        let p = (pulse + offset).fract();
        cr.set_line_width(2.0);
        match phase {
            OverlayPhase::Recording => {
                cr.set_source_rgba(1.0, 0.28, 0.35, (1.0 - p) * alpha);
            }
            OverlayPhase::Transcribing => {
                cr.set_source_rgba(0.38, 0.63, 0.92, (1.0 - p) * alpha);
            }
            OverlayPhase::Processing => {
                cr.set_source_rgba(0.72, 0.38, 1.0, (1.0 - p) * alpha);
            }
        }
        cr.arc(cx, cy, r + 3.0 + p * 5.0, 0.0, 2.0 * std::f64::consts::PI);
        let _ = cr.stroke();
    }

    match phase {
        OverlayPhase::Recording => {
            let painted = with_icon_surface(true, |surface| {
                paint_surface_at(cr, surface, cx, cy, r);
            });
            if painted.is_none() {
                draw_fallback_disc(cr, cx, cy, r, phase);
            }
        }
        OverlayPhase::Transcribing => {
            let painted = with_icon_surface(false, |surface| {
                paint_surface_at(cr, surface, cx, cy, r);
            });
            if painted.is_none() {
                draw_fallback_disc(cr, cx, cy, r, phase);
            }
        }
        OverlayPhase::Processing => {
            draw_fallback_disc(cr, cx, cy, r, phase);
        }
    }
}

#[allow(dead_code)]
fn draw_record_button(cr: &cairo::Context, cx: f64, cy: f64, r: f64, is_recording: bool) {
    draw_state_button(
        cr,
        cx,
        cy,
        r,
        if is_recording {
            OverlayPhase::Recording
        } else {
            OverlayPhase::Transcribing
        },
    );
}

fn draw_fallback_disc(cr: &cairo::Context, cx: f64, cy: f64, r: f64, phase: OverlayPhase) {
    let gradient = cairo::RadialGradient::new(cx, cy - r * 0.4, r * 0.15, cx, cy, r);
    match phase {
        OverlayPhase::Recording => {
            gradient.add_color_stop_rgba(0.0, 1.0, 0.36, 0.42, 1.0);
            gradient.add_color_stop_rgba(0.55, 0.96, 0.20, 0.30, 1.0);
            gradient.add_color_stop_rgba(1.0, 0.86, 0.10, 0.23, 1.0);
        }
        OverlayPhase::Transcribing => {
            gradient.add_color_stop_rgba(0.0, 0.38, 0.63, 0.92, 1.0);
            gradient.add_color_stop_rgba(0.55, 0.21, 0.52, 0.89, 1.0);
            gradient.add_color_stop_rgba(1.0, 0.11, 0.44, 0.85, 1.0);
        }
        OverlayPhase::Processing => {
            // Modern vibrant AI violet / fuchsia gradient
            gradient.add_color_stop_rgba(0.0, 0.78, 0.48, 1.0, 1.0);
            gradient.add_color_stop_rgba(0.55, 0.58, 0.22, 0.95, 1.0);
            gradient.add_color_stop_rgba(1.0, 0.36, 0.10, 0.88, 1.0);
        }
    }
    let _ = cr.set_source(&gradient);
    cr.arc(cx, cy, r, 0.0, 2.0 * std::f64::consts::PI);
    let _ = cr.fill();

    cr.set_line_width(1.5);
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.25);
    cr.arc(cx, cy, r - 1.0, 0.0, 2.0 * std::f64::consts::PI);
    let _ = cr.stroke();

    match phase {
        OverlayPhase::Recording | OverlayPhase::Transcribing => {
            draw_mic_glyph(cr, cx, cy - 13.0, 10.0);
        }
        OverlayPhase::Processing => {
            draw_ai_sparkles_glyph(cr, cx, cy, r * 0.62);
        }
    }
}

fn draw_sparkle_star(cr: &cairo::Context, cx: f64, cy: f64, r: f64) {
    let inner_r = r * 0.26;
    cr.new_path();
    cr.move_to(cx, cy - r);
    cr.curve_to(
        cx + inner_r,
        cy - inner_r,
        cx + inner_r,
        cy - inner_r,
        cx + r,
        cy,
    );
    cr.curve_to(
        cx + inner_r,
        cy + inner_r,
        cx + inner_r,
        cy + inner_r,
        cx,
        cy + r,
    );
    cr.curve_to(
        cx - inner_r,
        cy + inner_r,
        cx - inner_r,
        cy + inner_r,
        cx - r,
        cy,
    );
    cr.curve_to(
        cx - inner_r,
        cy - inner_r,
        cx - inner_r,
        cy - inner_r,
        cx,
        cy - r,
    );
    cr.close_path();
    let _ = cr.fill();
}

fn draw_ai_sparkles_glyph(cr: &cairo::Context, cx: f64, cy: f64, r: f64) {
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.98);
    // Center diamond star
    draw_sparkle_star(cr, cx - r * 0.08, cy + r * 0.08, r * 0.72);
    // Secondary star (top right)
    draw_sparkle_star(cr, cx + r * 0.45, cy - r * 0.42, r * 0.38);
    // Mini accent star (bottom left)
    draw_sparkle_star(cr, cx - r * 0.52, cy + r * 0.52, r * 0.22);
}

fn draw_ai_processing_meter(cr: &cairo::Context, x: f64, y: f64, w: f64, h: f64, phase: f64) {
    const BARS: usize = 18;
    let gap = 3.0;
    let bw = (w - gap * (BARS as f64 - 1.0)) / BARS as f64;
    let max_h = h - 2.0;

    for i in 0..BARS {
        let t = i as f64 / (BARS - 1) as f64;
        let wave = (t * 4.0 * std::f64::consts::PI - phase * 2.0 * std::f64::consts::PI).sin();
        let normalized = (wave * 0.5 + 0.5).powf(1.4);
        let bh = 3.0 + normalized * (max_h - 3.0);
        let bx = x + i as f64 * (bw + gap);
        let by = y + (h - bh) / 2.0;

        rounded_rect_path(cr, bx, by, bw, bh, bw / 2.0);

        let r = 0.70 + 0.28 * (t - 0.5).abs() * 2.0;
        let g = 0.45 + 0.30 * normalized;
        let b = 0.98;
        let alpha = 0.50 + 0.45 * normalized;

        cr.set_source_rgba(r.min(1.0), g.min(1.0), b, alpha);
        let _ = cr.fill();
    }
}

fn draw_mic_glyph(cr: &cairo::Context, cx: f64, top: f64, w: f64) {
    const PI: f64 = std::f64::consts::PI;
    let h = w * 1.35;
    let lw = w * 0.28;
    let r = w / 2.0;
    cr.set_source_rgba(1.0, 1.0, 1.0, 1.0);
    cr.set_line_width(lw);
    cr.set_line_cap(cairo::LineCap::Round);
    cr.set_line_join(cairo::LineJoin::Round);

    // Capsule (fully rounded ends).
    rounded_rect_path(cr, cx - r, top, w, h, r);
    let _ = cr.fill();

    // U-arc hanging below the capsule.
    let arc_r = w * 0.6;
    let arc_center_y = top + h + arc_r;
    cr.arc(cx, arc_center_y, arc_r, PI, 2.0 * PI);
    let _ = cr.stroke();

    // Stem + base.
    cr.move_to(cx, arc_center_y);
    cr.line_to(cx, arc_center_y + lw * 1.5);
    let _ = cr.stroke();
    cr.move_to(cx - lw * 1.6, arc_center_y + lw * 2.1);
    cr.line_to(cx + lw * 1.6, arc_center_y + lw * 2.1);
    let _ = cr.stroke();
}

// --- Live waveform --------------------------------------------------------

fn push_level(level: f32) {
    let mut levels = OVERLAY_LEVELS
        .get_or_init(|| Mutex::new(VecDeque::from(vec![0.0; LEVEL_HISTORY])))
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    // Fast-attack / smooth-decay damping for studio-grade audio visualization
    let prev = levels.back().copied().unwrap_or(0.0);
    let smoothed = if level > prev {
        level
    } else {
        prev * 0.75 + level * 0.25
    };

    levels.pop_front();
    levels.push_back(smoothed);
}

fn snapshot_levels() -> Vec<f32> {
    OVERLAY_LEVELS
        .get()
        .map(|queue| {
            queue
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .iter()
                .copied()
                .collect()
        })
        .unwrap_or_else(|| vec![0.0; LEVEL_HISTORY])
}

fn draw_waveform(cr: &cairo::Context, x: f64, y: f64, w: f64, h: f64, levels: &[f32]) {
    const BARS: usize = 18;
    let gap = 3.0;
    let bw = (w - gap * (BARS as f64 - 1.0)) / BARS as f64;
    let max_h = h - 3.0;
    let n = levels.len().min(BARS);
    for (i, level) in levels.iter().take(n).enumerate() {
        let val = (level.clamp(0.0, 1.0) as f64).sqrt();
        let bh = 3.0 + val * (max_h - 3.0);
        let bx = x + i as f64 * (bw + gap);
        let by = y + h - bh;
        rounded_rect_path(cr, bx, by, bw, bh, bw / 2.0);

        if val > 0.30 {
            // Voice energy active highlight
            cr.set_source_rgba(1.0, 0.96, 0.96, 0.95);
        } else {
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.72);
        }
        let _ = cr.fill();
    }
}

// --- Cairo helpers --------------------------------------------------------

fn rounded_rect_path(cr: &cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    const PI: f64 = std::f64::consts::PI;
    let r = r.min(w / 2.0).min(h / 2.0);
    cr.new_path();
    cr.move_to(x + r, y);
    cr.line_to(x + w - r, y);
    cr.arc(x + w - r, y + r, r, -PI / 2.0, 0.0);
    cr.line_to(x + w, y + h - r);
    cr.arc(x + w - r, y + h - r, r, 0.0, PI / 2.0);
    cr.line_to(x + r, y + h);
    cr.arc(x + r, y + h - r, r, PI / 2.0, PI);
    cr.line_to(x, y + r);
    cr.arc(x + r, y + r, r, PI, -PI / 2.0);
    cr.close_path();
}

#[cfg(test)]
mod tests {
    use super::*;
    use gtk4::cairo::{Context, Format, ImageSurface};

    /// Render `f` into an ARGB32 image surface (no display required).
    fn render(width: i32, height: i32, f: impl Fn(&cairo::Context)) -> ImageSurface {
        let surface = ImageSurface::create(Format::ARgb32, width, height).unwrap();
        let ctx = Context::new(&surface).unwrap();
        f(&ctx);
        surface
    }

    /// Read a pixel; cairo ARGB32 is stored premultiplied, native endian
    /// (little-endian byte order: B, G, R, A).
    fn pixel(surface: &mut ImageSurface, x: i32, y: i32) -> (u8, u8, u8, u8) {
        let stride = surface.stride() as usize;
        let data = surface.data().unwrap();
        let i = (y as usize) * stride + (x as usize) * 4;
        (data[i + 2], data[i + 1], data[i], data[i + 3]) // R, G, B, A
    }

    #[test]
    fn overlay_css_parses_without_errors() {
        if gtk4::init().is_err() {
            return;
        }
        let provider = gtk4::CssProvider::new();
        let errors = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let err_clone = errors.clone();
        provider.connect_parsing_error(move |_provider, section, error| {
            err_clone
                .lock()
                .unwrap()
                .push(format!("{error} at {section:?}"));
        });
        provider.load_from_data(include_str!("overlay.css"));
        let errs = errors.lock().unwrap();
        assert!(
            errs.is_empty(),
            "overlay.css had CSS parsing errors: {:?}",
            *errs
        );
    }

    #[test]
    fn record_button_renders_red_disc_with_white_mic() {
        // 44 px button: center (22, 22), disc radius 18.
        let mut surface = render(44, 44, |cr| draw_record_button(cr, 22.0, 22.0, 18.0, true));

        // Center of the icon (mic capsule): white / bright.
        let (r, g, b, a) = pixel(&mut surface, 22, 22);
        assert!(a > 200, "icon must be opaque, got a={a}");
        assert!(
            r > 150 && g > 150 && b > 150,
            "mic center should be white, got rgb({r},{g},{b})"
        );

        // Pulse ring (phase 0 => inner ring at r+3=21 from center cy=22 => y=1):
        let (r, g, _, a) = pixel(&mut surface, 22, 1);
        assert!(a > 0, "pulse ring should be drawn");
        assert!(r > g, "pulse ring should be red-tinted");
    }

    #[test]
    fn processing_button_renders_purple_disc_with_sparkles() {
        let mut surface = render(44, 44, |cr| {
            draw_state_button(cr, 22.0, 22.0, 18.0, OverlayPhase::Processing)
        });

        // Center of the AI sparkles icon: white / bright.
        let (r, g, b, a) = pixel(&mut surface, 22, 22);
        assert!(a > 200, "AI icon center must be opaque, got a={a}");
        assert!(
            r > 150 && g > 150 && b > 150,
            "sparkles center should be white, got rgb({r},{g},{b})"
        );

        // Pulse ring for Processing: purple-tinted (b > g).
        let (_, g, b, a) = pixel(&mut surface, 22, 1);
        assert!(a > 0, "pulse ring should be drawn");
        assert!(b > g, "pulse ring should be purple-tinted");
    }

    #[test]
    fn waveform_renders_bars() {
        let mut surface = render(120, 16, |cr| {
            let levels = vec![0.5f32; 20];
            draw_waveform(cr, 0.0, 0.0, 120.0, 16.0, &levels);
        });

        // With a constant mid level, the bottom band of the first bar is filled.
        let (r, g, b, a) = pixel(&mut surface, 1, 14);
        assert!(
            a > 100 && r > 200 && g > 200 && b > 200,
            "bars should be white-ish"
        );

        // Corners stay transparent (bars are bottom-aligned with a gap above).
        let (_, _, _, a) = pixel(&mut surface, 1, 2);
        assert!(a < 40, "top of the meter should be empty");
    }

    #[test]
    fn ai_processing_meter_renders_bars() {
        let mut surface = render(120, 16, |cr| {
            draw_ai_processing_meter(cr, 0.0, 0.0, 120.0, 16.0, 0.5);
        });

        let (_, g, b, a) = pixel(&mut surface, 1, 8);
        assert!(a > 80, "AI meter bar should be rendered");
        assert!(b > g, "AI meter bar should have purple/blue tint");
    }

    #[test]
    fn compact_pill_layouts_at_bottom_center() {
        let layout = compute_layout(1920, 1080, false, OverlayPosition::Bottom);
        assert_eq!(layout.w, PILL_W_COMPACT);
        assert_eq!(layout.h, PILL_H_COMPACT);
        // Horizontally centered…
        assert_eq!(layout.x, (1920 - PILL_W_COMPACT) / 2);
        // …and sitting above the bottom margin.
        assert_eq!(layout.y + layout.h + BOTTOM_MARGIN, 1080);
        // The button fits inside the pill.
        assert!(layout.btn_x >= layout.x && layout.btn_y >= layout.y);
        assert!(layout.btn_x + BUTTON_SIZE <= layout.x + layout.w);
        assert!(layout.btn_y + BUTTON_SIZE <= layout.y + layout.h);
    }

    #[test]
    fn top_position_pins_pill_to_top() {
        let layout = compute_layout(1920, 1080, false, OverlayPosition::Top);
        assert_eq!(layout.y, TOP_MARGIN);
    }

    #[test]
    fn overlay_canvas_paints_the_card() {
        let mut surface = render(1920, 1080, |cr| draw_overlay(cr, 1920.0, 1080.0));
        let layout = compute_layout(1920, 1080, false, OverlayPosition::Bottom);
        // Center of the card: translucent dark, not transparent.
        let (_, _, _, a) = pixel(&mut surface, layout.x + layout.w / 2, layout.y + 10);
        assert!(a > 180, "card should be painted");
        // Far corner of the screen stays transparent (click-through canvas).
        let (_, _, _, a) = pixel(&mut surface, 100, 100);
        assert!(a < 10, "screen area outside the pill must stay transparent");
    }

    /// One-off preview dump (OTUSH_DUMP_OVERLAY=1) used to inspect the pill
    /// composition outside a live session. Renders the 1920x1080 overlay
    /// canvas and saves a PNG into target/ (cairo's own PNG backend is a non-default
    /// feature, so the surface is re-encoded via the `image` crate).
    #[test]
    fn dump_overlay_preview() {
        if std::env::var("OTUSH_DUMP_OVERLAY").is_err() {
            return;
        }
        let mut surface = render(1920, 1080, |cr| draw_overlay(cr, 1920.0, 1080.0));
        let layout = compute_layout(1920, 1080, false, OverlayPosition::Bottom);
        let (r, g, b, _) = pixel(&mut surface, layout.btn_x + 22, layout.btn_y + 22);
        println!("button center rgb({r},{g},{b})");
        let (_, _, _, a) = pixel(&mut surface, layout.x + layout.w - 10, layout.y + 30);
        println!("card right edge alpha={a}");

        let stride = surface.stride() as usize;
        let data = surface.data().unwrap();
        let mut img = image::RgbaImage::new(1920, 1080);
        for y in 0..1080 {
            for x in 0..1920 {
                let i = y * stride + x * 4;
                let (b, g, r, a) = (data[i], data[i + 1], data[i + 2], data[i + 3]);
                let (r, g, b) = if a == 0 {
                    (0, 0, 0)
                } else {
                    let f = 255.0 / a as f32;
                    (
                        (r as f32 * f).round() as u8,
                        (g as f32 * f).round() as u8,
                        (b as f32 * f).round() as u8,
                    )
                };
                img.put_pixel(
                    x.try_into().unwrap(),
                    y.try_into().unwrap(),
                    image::Rgba([r, g, b, a]),
                );
            }
        }
        img.save(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/target/overlay_preview.png"
        ))
        .unwrap();
        println!("preview written to target/overlay_preview.png");
    }
}
