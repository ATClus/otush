#!/usr/bin/env python3
"""Generate the Otush icon set (app icon + tray icons).

Brand colors (GNOME-style blue -> violet):
  top    #6FA8FF
  bottom #8B5CF6

Outputs (written to resources/):
  otush.png                512x512 app icon
  otush-128.png            128x128 app icon (hicolor install)
  otush-icon.svg           scalable app icon (hicolor)
  tray_idle.png            mic outline, light gray  (dark panels)
  tray_idle_dark.png       mic outline, dark gray   (light panels)
  tray_recording.png       filled mic + red dot     (dark panels)
  tray_recording_dark.png  filled mic + red dot     (light panels)
  tray_transcribing.png    mic + waveform bars      (dark panels)
  tray_transcribing_dark.png mic + waveform bars    (light panels)
  tray_idle_warning.png    mic outline + warning    (dark panels)
  tray_idle_warning_dark.png mic outline + warning  (light panels)
  recording.png            colored filled mic + dot (brand)
  transcribing.png         colored mic + bars       (brand)

Run from the repository root: python3 scripts/gen_icons.py
"""

import os
from PIL import Image, ImageDraw, ImageFilter

SS = 4  # supersampling factor for smooth edges
OUT = os.path.join(os.path.dirname(__file__), "..", "resources")

TOP = (111, 168, 255)     # #6FA8FF
BOTTOM = (139, 92, 246)   # #8B5CF6
WHITE = (255, 255, 255, 255)
LIGHT_GRAY = (230, 230, 230, 255)   # for dark panels
DARK_GRAY = (74, 74, 74, 255)       # for light panels
RED = (255, 62, 84, 255)            # recording dot
AMBER = (255, 176, 32, 255)         # warning


def canvas(size):
    """Supersampled transparent canvas."""
    return Image.new("RGBA", (size * SS, size * SS), (0, 0, 0, 0))


def downscale(img, size):
    return img.resize((size, size), Image.LANCZOS)


def draw_mic(d, cx, top, w, h, radius, color, filled, width=2.4, ss=SS):
    """Draw a microphone glyph in a 64-unit logical space (supersampled)."""
    x0, y0 = (cx - w / 2) * ss, top * ss
    x1, y1 = (cx + w / 2) * ss, (top + h) * ss
    r = radius * ss
    if filled:
        d.rounded_rectangle([x0, y0, x1, y1], radius=r, fill=color)
    else:
        d.rounded_rectangle([x0, y0, x1, y1], radius=r, outline=color, width=round(width * ss))
    # Stand: semicircle below the body
    arc_cx = cx * ss
    arc_top = (top + h) * ss
    arc_r = (w / 2 + 1.5) * ss
    d.arc(
        [arc_cx - arc_r, arc_top - arc_r, arc_cx + arc_r, arc_top + arc_r],
        start=180, end=360, fill=color, width=round(width * ss),
    )
    # Stem + base
    stem_top = arc_top + arc_r
    d.line(
        [arc_cx, stem_top, arc_cx, stem_top + 5 * ss],
        fill=color, width=round(width * ss),
    )
    d.line(
        [arc_cx - 5.5 * ss, stem_top + 7 * ss, arc_cx + 5.5 * ss, stem_top + 7 * ss],
        fill=color, width=round(width * ss),
    )


def draw_bars(d, cx, bottom, heights, color, width=3.0, gap=4.5, ss=SS):
    """Draw waveform bars to the right of the mic."""
    x = cx
    for hgt in heights:
        top = bottom - hgt
        d.rounded_rectangle(
            [x * ss, top * ss, (x + width) * ss, bottom * ss],
            radius=1.5 * ss, fill=color,
        )
        x += width + gap


def draw_warning_triangle(d, cx, top, size, color, ss=SS):
    """Small exclamation triangle (warning badge)."""
    half = size / 2
    pts = [
        (cx * ss, top * ss),
        ((cx - half) * ss, (top + size) * ss),
        ((cx + half) * ss, (top + size) * ss),
    ]
    d.polygon(pts, fill=color)
    # Exclamation dot
    ex_cx = cx * ss
    ex_top = (top + size * 0.52) * ss
    d.ellipse([ex_cx - 1.1 * ss, ex_top, ex_cx + 1.1 * ss, ex_top + 2.2 * ss], fill=(255, 255, 255, 255))
    d.ellipse(
        [ex_cx - 1.1 * ss, (top + size * 0.78) * ss, ex_cx + 1.1 * ss, (top + size * 0.78 + 2.2) * ss],
        fill=(255, 255, 255, 255),
    )


def tray_icon(mic_color, filled=False, bars=None, warning=False, dot=False):
    img = canvas(64)
    d = ImageDraw.Draw(img)
    draw_mic(d, 30, 14, 17, 26, 8.5, mic_color, filled)
    if bars:
        draw_bars(d, 45, 44, bars, mic_color)
    if warning:
        draw_warning_triangle(d, 49, 10, 12, AMBER)
    if dot:
        d.ellipse([46 * SS, 46 * SS, 56 * SS, 56 * SS], fill=RED)
    return downscale(img, 64)


def gradient_rect(size, radius, top=TOP, bottom=BOTTOM, margin=None):
    """Rounded-rect with a vertical blue->violet gradient (supersampled)."""
    if margin is None:
        margin = size * 0.055  # transparent padding around the shape
    img = canvas(size)
    grad = Image.new("RGBA", (size * SS, size * SS))
    gd = ImageDraw.Draw(grad)
    for y in range(size * SS):
        t = y / (size * SS - 1)
        c = tuple(int(top[i] + (bottom[i] - top[i]) * t) for i in range(3)) + (255,)
        gd.line([(0, y), (size * SS, y)], fill=c)
    m = margin * SS
    mask = Image.new("L", (size * SS, size * SS), 0)
    md = ImageDraw.Draw(mask)
    md.rounded_rectangle([m, m, size * SS - 1 - m, size * SS - 1 - m], radius=radius * SS, fill=255)
    img.paste(grad, (0, 0), mask)
    # Soft top highlight
    hl = Image.new("RGBA", (size * SS, size * SS), (0, 0, 0, 0))
    hd = ImageDraw.Draw(hl)
    for y in range(int(size * SS * 0.45)):
        a = int(30 * (1 - y / (size * SS * 0.45)))
        hd.line([(0, y), (size * SS, y)], fill=(255, 255, 255, a))
    hl = hl.filter(ImageFilter.GaussianBlur(radius=size * SS * 0.03))
    img.alpha_composite(hl)
    # Clip back to the rounded rect
    final = Image.new("RGBA", (size * SS, size * SS), (0, 0, 0, 0))
    final.paste(img, (0, 0), mask)
    return downscale(final, size)


def gradient_mic(size=64, bars=None, dot=False):
    """Colored tray icon: mic filled with the brand gradient."""
    img = canvas(64)
    grad = Image.new("RGBA", (64 * SS, 64 * SS))
    gd = ImageDraw.Draw(grad)
    for y in range(64 * SS):
        t = y / (64 * SS - 1)
        c = tuple(int(TOP[i] + (BOTTOM[i] - TOP[i]) * t) for i in range(3)) + (255,)
        gd.line([(0, y), (64 * SS, y)], fill=c)
    mask = Image.new("L", (64 * SS, 64 * SS), 0)
    md = ImageDraw.Draw(mask)
    draw_mic(md, 30, 14, 17, 26, 8.5, 255, filled=True)
    if bars:
        draw_bars(md, 45, 44, bars, 255)
    img.paste(grad, (0, 0), mask)
    if dot:
        d = ImageDraw.Draw(img)
        d.ellipse([46 * SS, 46 * SS, 56 * SS, 56 * SS], fill=RED)
    return downscale(img, 64)


def app_icon(size):
    img = gradient_rect(size, radius=size * 0.225)
    ss = SS * (size / 512.0) if size != 512 else SS
    # Redraw mic at full res directly onto the icon canvas for crispness
    big = Image.new("RGBA", (size * SS, size * SS), (0, 0, 0, 0))
    d = ImageDraw.Draw(big)
    scale = size / 512.0
    # Mic geometry in 512-space, scaled
    cx = 256 * scale * SS
    top = 118 * scale * SS
    w = 124 * scale * SS
    h = 206 * scale * SS
    r = 62 * scale * SS
    lw = 26 * scale * SS
    d.rounded_rectangle([cx - w / 2, top, cx + w / 2, top + h], radius=r, fill=WHITE)
    arc_r = (w / 2 + 14) * scale * SS
    arc_top = (top + h) * SS
    d.arc(
        [cx - arc_r, arc_top - arc_r, cx + arc_r, arc_top + arc_r],
        start=180, end=360, fill=WHITE, width=round(lw),
    )
    stem_top = arc_top + arc_r
    d.line([cx, stem_top, cx, stem_top + 30 * scale * SS], fill=WHITE, width=round(lw))
    d.line(
        [cx - 42 * scale * SS, stem_top + 40 * scale * SS, cx + 42 * scale * SS, stem_top + 40 * scale * SS],
        fill=WHITE, width=round(lw),
    )
    img = img.resize((size * SS, size * SS), Image.LANCZOS)
    img.alpha_composite(big)
    return downscale(img, size)


def main():
    os.makedirs(OUT, exist_ok=True)

    app_icon(512).save(os.path.join(OUT, "otush.png"))
    app_icon(128).save(os.path.join(OUT, "otush-128.png"))

    # Tray: dark-panel variants (light glyphs)
    tray_icon(LIGHT_GRAY, filled=False).save(os.path.join(OUT, "tray_idle.png"))
    tray_icon(LIGHT_GRAY, filled=True, dot=True).save(os.path.join(OUT, "tray_recording.png"))
    tray_icon(LIGHT_GRAY, filled=False, bars=[9, 15, 11]).save(os.path.join(OUT, "tray_transcribing.png"))
    tray_icon(LIGHT_GRAY, filled=False, warning=True).save(os.path.join(OUT, "tray_idle_warning.png"))

    # Tray: light-panel variants (dark glyphs)
    tray_icon(DARK_GRAY, filled=False).save(os.path.join(OUT, "tray_idle_dark.png"))
    tray_icon(DARK_GRAY, filled=True, dot=True).save(os.path.join(OUT, "tray_recording_dark.png"))
    tray_icon(DARK_GRAY, filled=False, bars=[9, 15, 11]).save(os.path.join(OUT, "tray_transcribing_dark.png"))
    tray_icon(DARK_GRAY, filled=False, warning=True).save(os.path.join(OUT, "tray_idle_warning_dark.png"))

    # Tray: colored (Linux) variants
    gradient_mic(bars=None, dot=True).save(os.path.join(OUT, "recording.png"))
    gradient_mic(bars=[9, 15, 11]).save(os.path.join(OUT, "transcribing.png"))

    print("icons written to", os.path.abspath(OUT))


if __name__ == "__main__":
    main()
