//! Overlay painting: layout, Cairo pill, text, icons. (split from `overlay.rs`; same behavior).

use super::state::{
    pulse_phase, upgrade, OverlayPhase, BOTTOM_MARGIN, BUTTON_SIZE, GAP, LAST_LAYOUT,
    LEVEL_HISTORY, OVERLAY_DRAW_AREA, OVERLAY_LEVELS, OVERLAY_PHASE, OVERLAY_POSITION,
    OVERLAY_STATE_TEXT, OVERLAY_STREAMING, OVERLAY_STREAM_MARKUP, PAD, PILL_H_COMPACT,
    PILL_H_STREAM, PILL_W_COMPACT, PILL_W_STREAM, RECORDING_START_INSTANT, TOP_MARGIN,
};
use crate::settings::OverlayPosition;
use gtk4::cairo;
use gtk4::prelude::*;
use pangocairo::pango;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// Geometry of the floating pill (screen coordinates).
#[derive(Clone, Copy, Debug)]
pub(super) struct PillLayout {
    pub(super) x: i32,
    pub(super) y: i32,
    pub(super) w: i32,
    pub(super) h: i32,
    pub(super) btn_x: i32,
    pub(super) btn_y: i32,
    pub(super) state_x: i32,
    pub(super) state_y: i32,
    pub(super) wave_x: i32,
    pub(super) wave_y: i32,
    pub(super) wave_w: i32,
    pub(super) wave_h: i32,
    pub(super) text_x: i32,
    pub(super) text_y: i32,
    pub(super) text_w: i32,
}

/// Repaint the overlay drawing area.
///
/// NOTE: GTK4 removed partial invalidation (`gtk_widget_queue_draw_area`
/// is gone), so a full `queue_draw()` is the only option. The window is a
/// fullscreen click-through surface by design (see `apply_surface_regions`),
/// and every phase animates (waveform / AI meter / recording timer), so the
/// pulse timer legitimately repaints while visible — kept at 10 FPS, the
/// slowest rate that keeps the AI meter smooth. The timer is removed on hide
/// (`stop_pulse`), so nothing repaints while the overlay is invisible. When
/// no pill layout is cached yet the draw itself is a no-op clear.
pub(super) fn queue_pill_redraw() {
    let Some(area) = upgrade(&OVERLAY_DRAW_AREA) else {
        return;
    };
    area.queue_draw();
}

pub(super) fn position_code(position: OverlayPosition) -> u8 {
    match position {
        OverlayPosition::Top => 1,
        OverlayPosition::Bottom => 0,
    }
}

pub(super) fn get_monitor_geometry(window: &gtk4::Window) -> Option<gtk4::gdk::Rectangle> {
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
pub(super) fn apply_surface_regions(window: &gtk4::Window) {
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
pub(super) static SURFACE_TRANSPARENT_DONE: AtomicBool = AtomicBool::new(false);

/// Enforce click-through + transparency as soon as the surface is available
/// (called from the draw path, which runs after the window is realized).
pub(super) fn ensure_surface_transparent(window_weak: &glib::SendWeakRef<gtk4::Window>) {
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

pub(super) fn position_from_code(code: u8) -> OverlayPosition {
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
pub(super) fn overlay_css_provider() -> gtk4::CssProvider {
    static CSS_LOADED: AtomicBool = AtomicBool::new(false);
    let provider = gtk4::CssProvider::new();
    provider.load_from_data(include_str!("../overlay.css"));
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

pub(super) fn compute_layout(
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

pub(super) fn draw_overlay(cr: &cairo::Context, w: f64, h: f64) {
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

thread_local! {
    static RECORDING_SURFACE: RefCell<Option<cairo::ImageSurface>> = const { RefCell::new(None) };
    static TRANSCRIBING_SURFACE: RefCell<Option<cairo::ImageSurface>> = const { RefCell::new(None) };
}

pub(super) fn with_icon_surface<R>(
    is_recording: bool,
    f: impl FnOnce(&cairo::ImageSurface) -> R,
) -> Option<R> {
    if is_recording {
        RECORDING_SURFACE.with(|cell| {
            let mut opt = cell.borrow_mut();
            if opt.is_none() {
                *opt = load_cairo_image(include_bytes!("../../resources/recording.png"));
            }
            opt.as_ref().map(f)
        })
    } else {
        TRANSCRIBING_SURFACE.with(|cell| {
            let mut opt = cell.borrow_mut();
            if opt.is_none() {
                *opt = load_cairo_image(include_bytes!("../../resources/transcribing.png"));
            }
            opt.as_ref().map(f)
        })
    }
}

pub(super) fn load_cairo_image(bytes: &[u8]) -> Option<cairo::ImageSurface> {
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

pub(super) fn paint_surface_at(
    cr: &cairo::Context,
    surface: &cairo::ImageSurface,
    cx: f64,
    cy: f64,
    r: f64,
) {
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

pub(super) fn draw_state_button(
    cr: &cairo::Context,
    cx: f64,
    cy: f64,
    r: f64,
    phase: OverlayPhase,
) {
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

pub(super) fn draw_sparkle_star(cr: &cairo::Context, cx: f64, cy: f64, r: f64) {
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

pub(super) fn draw_ai_sparkles_glyph(cr: &cairo::Context, cx: f64, cy: f64, r: f64) {
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.98);
    // Center diamond star
    draw_sparkle_star(cr, cx - r * 0.08, cy + r * 0.08, r * 0.72);
    // Secondary star (top right)
    draw_sparkle_star(cr, cx + r * 0.45, cy - r * 0.42, r * 0.38);
    // Mini accent star (bottom left)
    draw_sparkle_star(cr, cx - r * 0.52, cy + r * 0.52, r * 0.22);
}

pub(super) fn draw_ai_processing_meter(
    cr: &cairo::Context,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    phase: f64,
) {
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

pub(super) fn draw_mic_glyph(cr: &cairo::Context, cx: f64, top: f64, w: f64) {
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

pub(super) fn push_level(level: f32) {
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

pub(super) fn snapshot_levels() -> Vec<f32> {
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

pub(super) fn draw_waveform(cr: &cairo::Context, x: f64, y: f64, w: f64, h: f64, levels: &[f32]) {
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

pub(super) fn rounded_rect_path(cr: &cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
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
