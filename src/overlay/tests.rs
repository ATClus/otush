//! Overlay Cairo pixel tests (split from `overlay.rs`; same assertions).

use super::paint::{
    compute_layout, draw_ai_processing_meter, draw_overlay, draw_state_button, draw_waveform,
};
use super::state::OverlayPhase;
use super::state::{BOTTOM_MARGIN, BUTTON_SIZE, PILL_H_COMPACT, PILL_W_COMPACT, TOP_MARGIN};
use crate::settings::OverlayPosition;
use gtk4::cairo;

#[cfg(test)]
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

mod overlay_tests {
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
                .unwrap_or_else(|e| e.into_inner())
                .push(format!("{error} at {section:?}"));
        });
        provider.load_from_data(include_str!("../overlay.css"));
        let errs = errors.lock().unwrap_or_else(|e| e.into_inner());
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
