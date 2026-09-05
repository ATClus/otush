#!/usr/bin/env python3
"""Otush Icon Generator - GNOME 46+ Standard (Libadwaita).

Composition:
  1. Base: Adwaita squircle with lower 3D bevel and top edge highlight.
  2. Backdrop: Integrated translucent audio soundwaves spectrum.
  3. Contact Shadow: Soft ambient occlusion to elevate the studio mic from the background.
  4. Foreground: Sharp and solid studio microphone.
"""

import os
import math
from PIL import Image, ImageDraw, ImageFilter

SS = 4  # 4x Supersampling for high-precision anti-aliasing

# Detect output directory
SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
if os.path.basename(SCRIPT_DIR) == "scripts":
    OUT_DIR = os.path.join(SCRIPT_DIR, "..", "resources")
else:
    OUT_DIR = os.path.join(SCRIPT_DIR, "resources")

# ==============================================================================
# GNOME 46+ Palette (Libadwaita)
# ==============================================================================
# Base Theme (Adwaita Blue -> Violet)
BLUE_TOP    = (98, 160, 234)   # #62A0EA
BLUE_MID    = (53, 132, 228)   # #3584E4
BLUE_BOT    = (28, 113, 216)   # #1C71D8
VIOLET_DEEP = (98, 68, 197)    # #6244C5
BLUE_BEVEL  = (16, 68, 135)    # Base bevel shadow

# Active Recording Theme (Vibrant GNOME Red matching the recording pulse)
RED_TOP     = (255, 75, 85)    # #FF4B55 - bright scarlet red
RED_MID     = (240, 32, 50)    # #F02032 - pure recording red
RED_BOT     = (205, 18, 38)    # #CD1226 - deep crimson
RED_BEVEL   = (135, 14, 26)

# Element Colors
WHITE            = (255, 255, 255, 255)
WAVE_TINT_NORMAL = (255, 255, 255, 48)  # Subtle translucent waves (20% opacity)
WAVE_TINT_REC    = (255, 255, 255, 65)  # Crisp recording waves
DARK_PANEL_TRAY  = (255, 255, 255, 240) # Crisp white (94%) for dark GNOME Shell panel
LIGHT_PANEL_TRAY = (40, 40, 45, 240)    # Dark gray for light panels
ACCENT_AMBER     = (246, 211, 45, 255)  # Warning / Attention


def canvas(size):
    """Creates a transparent RGBA canvas with supersampling."""
    return Image.new("RGBA", (size * SS, size * SS), (0, 0, 0, 0))


def downsample(img, size):
    """Resizes with Lanczos filter ensuring smooth edges."""
    return img.resize((size, size), Image.LANCZOS)


# ==============================================================================
# Layer 1: Adwaita Base (Squircle + Bevel + Top Highlight Glow)
# ==============================================================================
def render_adwaita_surface(size, top_c, mid_c, bot_c, bevel_c, has_bevel=True):
    img = canvas(size)
    margin = size * 0.07 * SS
    box_w = (size * SS) - (2 * margin)
    box_h = box_w
    radius = box_w * 0.225
    bevel_h = (3.0 * SS) if has_bevel else 0.0

    if has_bevel:
        # 1. Lower Depth Bevel
        d = ImageDraw.Draw(img)
        d.rounded_rectangle(
            [margin, margin + bevel_h, margin + box_w, margin + box_h + bevel_h],
            radius=radius, fill=bevel_c + (255,)
        )

    # 2. Vertical Plate Gradient
    grad = Image.new("RGBA", (size * SS, size * SS))
    gd = ImageDraw.Draw(grad)
    for y in range(int(margin), int(margin + box_h)):
        t = (y - margin) / float(box_h)
        if t < 0.5:
            f = t / 0.5
            col = tuple(int(top_c[i] + (mid_c[i] - top_c[i]) * f) for i in range(3))
        else:
            f = (t - 0.5) / 0.5
            col = tuple(int(mid_c[i] + (bot_c[i] - mid_c[i]) * f) for i in range(3))
        gd.line([(margin, y), (margin + box_w, y)], fill=col + (255,))

    mask = Image.new("L", (size * SS, size * SS), 0)
    md = ImageDraw.Draw(mask)
    md.rounded_rectangle([margin, margin, margin + box_w, margin + box_h], radius=radius, fill=255)
    img.paste(grad, (0, 0), mask)

    # 3. Inner Top Glow (1.5px top highlight)
    glow = Image.new("RGBA", (size * SS, size * SS), (0, 0, 0, 0))
    g_draw = ImageDraw.Draw(glow)
    g_draw.rounded_rectangle(
        [margin + SS, margin + SS, margin + box_w - SS, margin + box_h - SS],
        radius=radius - SS, outline=(255, 255, 255, 55), width=max(1, round(1.2 * SS))
    )
    img.alpha_composite(glow)

    return img


# ==============================================================================
# Layer 2: Background Soundwaves Spectrum
# ==============================================================================
def draw_backdrop_soundwaves(img, size, color=WAVE_TINT_NORMAL, ss=SS):
    """Renders stylized equalizer bars in the background."""
    wave_layer = Image.new("RGBA", img.size, (0, 0, 0, 0))
    wd = ImageDraw.Draw(wave_layer)

    cx = (size / 2.0) * ss
    cy = (size / 2.0) * ss
    box_w = ((size * ss) - (2 * (size * 0.07 * ss)))
    bar_w = 4.5 * ss
    gap = 4.5 * ss

    height_factors = [0.22, 0.40, 0.65, 0.88, 0.52, 0.95, 0.52, 0.88, 0.65, 0.40, 0.22]
    num_bars = len(height_factors)
    total_w = (num_bars * bar_w) + ((num_bars - 1) * gap)
    start_x = cx - (total_w / 2.0)

    for i, factor in enumerate(height_factors):
        x0 = start_x + (i * (bar_w + gap))
        x1 = x0 + bar_w
        bar_h = (box_w * 0.58) * factor
        y0 = cy - (bar_h / 2.0)
        y1 = cy + (bar_h / 2.0)
        wd.rounded_rectangle([x0, y0, x1, y1], radius=bar_w / 2.0, fill=color)

    margin = size * 0.07 * ss
    mask = Image.new("L", img.size, 0)
    md = ImageDraw.Draw(mask)
    md.rounded_rectangle([margin, margin, margin + box_w, margin + box_w], radius=box_w * 0.225, fill=255)

    img.paste(wave_layer, (0, 0), mask)


# ==============================================================================
# Layers 3 and 4: Studio Microphone with Soft Shadow
# ==============================================================================
def draw_studio_mic_geometry(d, cx, cy, h, color=WHITE, is_shadow=False):
    """Draws exact studio microphone geometry."""
    cap_w = h * 0.38
    cap_h = h * 0.58
    arc_w = h * 0.58
    arc_h = h * 0.46
    stroke = h * 0.088
    stem_h = h * 0.12
    base_w = h * 0.40

    total_h = cap_h + (arc_h * 0.4) + stem_h + stroke
    top_y = cy - (total_h * 0.46)

    # 1. Main Capsule
    cap_x0 = cx - (cap_w / 2.0)
    cap_x1 = cap_x0 + cap_w
    cap_y0 = top_y
    cap_y1 = cap_y0 + cap_h
    d.rounded_rectangle([cap_x0, cap_y0, cap_x1, cap_y1], radius=cap_w / 2.0, fill=color)

    if not is_shadow:
        # Subtle internal microphone grill line
        grill_y = cap_y0 + (cap_h * 0.42)
        d.line([cap_x0 + stroke * 0.3, grill_y, cap_x1 - stroke * 0.3, grill_y], fill=(0, 0, 0, 35), width=max(1, round(stroke * 0.25)))

    # 2. U-Arc Holder
    arc_top = cap_y1 - (arc_h * 0.65)
    arc_box = [cx - (arc_w / 2.0), arc_top, cx + (arc_w / 2.0), arc_top + arc_h]
    d.arc(arc_box, start=10, end=170, fill=color, width=round(stroke))

    # 3. Vertical Stem
    stem_top = arc_top + arc_h - (stroke * 0.4)
    stem_bot = stem_top + stem_h
    d.rounded_rectangle([cx - (stroke / 2.0), stem_top, cx + (stroke / 2.0), stem_bot], radius=stroke / 2.0, fill=color)

    # 4. Stabilizing Base
    base_y0 = stem_bot
    base_y1 = base_y0 + stroke
    d.rounded_rectangle([cx - (base_w / 2.0), base_y0, cx + (base_w / 2.0), base_y1], radius=stroke / 2.0, fill=color)


def make_sparkle_points(cx, cy, r, num_steps=16):
    """Calculates polygon vertices for a 4-pointed Libadwaita AI sparkle."""
    points = []
    quadrants = [
        ((cx, cy - r), (cx, cy), (cx + r, cy)),
        ((cx + r, cy), (cx, cy), (cx, cy + r)),
        ((cx, cy + r), (cx, cy), (cx - r, cy)),
        ((cx - r, cy), (cx, cy), (cx, cy - r)),
    ]
    for p0, pc, p1 in quadrants:
        for i in range(num_steps):
            t = i / float(num_steps)
            x = (1 - t) ** 2 * p0[0] + 2 * (1 - t) * t * pc[0] + t ** 2 * p1[0]
            y = (1 - t) ** 2 * p0[1] + 2 * (1 - t) * t * pc[1] + t ** 2 * p1[1]
            points.append((x, y))
    return points


def draw_ai_sparkle(d, cx, cy, r, color=WHITE):
    """Draws a smooth 4-pointed AI intelligence sparkle."""
    pts = make_sparkle_points(cx, cy, r)
    d.polygon(pts, fill=color)


# ==============================================================================
# Layers 3 and 4: Studio Microphone & AI Sparkles with Soft Shadow
# ==============================================================================
def draw_suite_elements(d, cx, cy, h, color=WHITE, is_shadow=False, ss=SS):
    """Draws studio microphone together with AI intelligence sparkles."""
    # 1. Microphone
    draw_studio_mic_geometry(d, cx, cy, h, color=color, is_shadow=is_shadow)

    # 2. Primary AI Sparkle (Upper Right)
    sparkle_cx = cx + (h * 0.48)
    sparkle_cy = cy - (h * 0.46)
    sparkle_r = h * 0.21
    draw_ai_sparkle(d, sparkle_cx, sparkle_cy, sparkle_r, color=color)

    # 3. Secondary Micro Sparkle (Upper Left)
    micro_cx = cx - (h * 0.47)
    micro_cy = cy - (h * 0.34)
    micro_r = h * 0.10
    draw_ai_sparkle(d, micro_cx, micro_cy, micro_r, color=color)


def draw_studio_microphone_with_shadow(img, size, color=WHITE, ss=SS):
    """Renders diffuse contact shadow and composites the solid microphone + AI sparkles."""
    cx = (size / 2.0) * ss
    cy = (size / 2.0) * ss
    h = size * 0.44 * ss

    # 1. Soft Shadow Layer (Ambient Occlusion)
    shadow_layer = Image.new("RGBA", img.size, (0, 0, 0, 0))
    s_draw = ImageDraw.Draw(shadow_layer)
    shadow_offset_y = 3.0 * ss
    draw_suite_elements(s_draw, cx, cy + shadow_offset_y, h, color=(0, 0, 0, 95), is_shadow=True, ss=ss)
    shadow_layer = shadow_layer.filter(ImageFilter.GaussianBlur(radius=3.5 * ss))
    img.alpha_composite(shadow_layer)

    # 2. Foreground Solid Layer
    fg_layer = Image.new("RGBA", img.size, (0, 0, 0, 0))
    fg_draw = ImageDraw.Draw(fg_layer)
    draw_suite_elements(fg_draw, cx, cy, h, color=color, is_shadow=False, ss=ss)
    img.alpha_composite(fg_layer)


# ==============================================================================
# GNOME 46+ Tray Icons (Full Optical Proportion 16px/24px)
# ==============================================================================
def draw_warning_badge(d, cx, cy, size, color=ACCENT_AMBER, ss=SS):
    cx = cx * ss
    cy = cy * ss
    s = size * ss
    half = s / 2.0
    pts = [(cx, cy - half), (cx - half * 1.15, cy + half), (cx + half * 1.15, cy + half)]
    d.polygon(pts, fill=color)
    bar_w = s * 0.14
    d.line([(cx, cy - half * 0.2), (cx, cy + half * 0.3)], fill=(0, 0, 0, 220), width=round(bar_w))
    d.ellipse([cx - bar_w / 2.0, cy + half * 0.55, cx + bar_w / 2.0, cy + half * 0.55 + bar_w], fill=(0, 0, 0, 220))


def make_app_icon(size, is_recording=False, has_bevel=True):
    """Generates the full primary application icon with all layers integrated."""
    if is_recording:
        base = render_adwaita_surface(size, RED_TOP, RED_MID, RED_BOT, RED_BEVEL, has_bevel=has_bevel)
        draw_backdrop_soundwaves(base, size, color=WAVE_TINT_REC, ss=SS)
    else:
        base = render_adwaita_surface(size, BLUE_TOP, BLUE_MID, VIOLET_DEEP, BLUE_BEVEL, has_bevel=has_bevel)
        draw_backdrop_soundwaves(base, size, color=WAVE_TINT_NORMAL, ss=SS)

    draw_studio_microphone_with_shadow(base, size, color=WHITE, ss=SS)
    return downsample(base, size)


def make_tray_symbolic(color, transcribing=False, recording=False, warning=False):
    """Generates high-definition symbolic icon filling the standard GNOME 46+ icon height."""
    img = canvas(64)
    d = ImageDraw.Draw(img)
    ss = SS
    cx = 32.0 * ss

    # GNOME 46+ microphone geometry (~84% optical height fill)
    cap_w = 20.0 * ss
    cap_h = 32.0 * ss
    cap_y0 = 6.0 * ss
    cap_y1 = cap_y0 + cap_h

    arc_w = 38.0 * ss
    arc_h = 28.0 * ss
    arc_top = 18.0 * ss
    stroke = 6.5 * ss

    stem_top = arc_top + arc_h - (stroke * 0.3)
    stem_h = 8.5 * ss
    stem_bot = stem_top + stem_h

    base_w = 30.0 * ss
    base_h = 6.0 * ss
    base_y0 = stem_bot
    base_y1 = base_y0 + base_h

    if transcribing:
        # Animated lateral equalizer waves
        wave_heights = [12.0 * ss, 22.0 * ss, 14.0 * ss]
        bar_w = 4.0 * ss
        for i, h in enumerate(wave_heights):
            x_l = (8.0 + i * 5.0) * ss
            x_r = (56.0 - (i + 1) * 5.0) * ss
            y0 = (24.0 * ss) - (h / 2.0)
            y1 = y0 + h
            d.rounded_rectangle([x_l, y0, x_l + bar_w, y1], radius=bar_w / 2.0, fill=color[:3] + (160,))
            d.rounded_rectangle([x_r, y0, x_r + bar_w, y1], radius=bar_w / 2.0, fill=color[:3] + (160,))

    # 1. Central microphone capsule
    d.rounded_rectangle([cx - cap_w / 2.0, cap_y0, cx + cap_w / 2.0, cap_y1], radius=cap_w / 2.0, fill=color)

    # 2. U-Arc support
    arc_box = [cx - arc_w / 2.0, arc_top, cx + arc_w / 2.0, arc_top + arc_h]
    d.arc(arc_box, start=0, end=180, fill=color, width=round(stroke))

    # 3. Vertical stem
    d.rounded_rectangle([cx - stroke / 2.0, stem_top, cx + stroke / 2.0, stem_bot], radius=stroke / 2.0, fill=color)

    # 4. Lower base
    d.rounded_rectangle([cx - base_w / 2.0, base_y0, cx + base_w / 2.0, base_y1], radius=base_h / 2.0, fill=color)

    # 5. AI Suite Sparkle on idle/transcribing
    if not recording and not warning:
        sparkle_cx = 48.0 * ss
        sparkle_cy = 13.0 * ss
        sparkle_r = 7.5 * ss
        draw_ai_sparkle(d, sparkle_cx, sparkle_cy, sparkle_r, color=color)

    if recording:
        # Pulsing bright red recording dot on top-right
        dot_r = 8.5 * ss
        dot_cx = 49.0 * ss
        dot_cy = 13.0 * ss
        d.ellipse([dot_cx - dot_r, dot_cy - dot_r, dot_cx + dot_r, dot_cy + dot_r], fill=RED_TOP + (255,))
        # Subtle contrast outline around the red dot
        d.ellipse([dot_cx - dot_r, dot_cy - dot_r, dot_cx + dot_r, dot_cy + dot_r], outline=(255, 255, 255, 200), width=round(1.5 * ss))

    if warning:
        draw_warning_badge(d, 48, 16, 22, color=ACCENT_AMBER, ss=SS)

    return downsample(img, 64)


def write_scalable_svg(path):
    """Generates layered SVG file with backdrop soundwaves, studio microphone, and AI sparkles."""
    svg = """<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" width="512" height="512" viewBox="0 0 512 512">
  <defs>
    <!-- Adwaita Surface Gradient -->
    <linearGradient id="adwaita_bg" x1="0%" y1="0%" x2="0%" y2="100%">
      <stop offset="0%" stop-color="#62a0ea"/>
      <stop offset="45%" stop-color="#3584e4"/>
      <stop offset="100%" stop-color="#6244c5"/>
    </linearGradient>

    <!-- Lower 3D Bevel -->
    <linearGradient id="bevel_bg" x1="0%" y1="0%" x2="0%" y2="100%">
      <stop offset="0%" stop-color="#1c71d8"/>
      <stop offset="100%" stop-color="#104487"/>
    </linearGradient>

    <!-- Microphone Soft Shadow -->
    <filter id="mic_shadow" x="-20%" y="-20%" width="140%" height="140%">
      <feDropShadow dx="0" dy="6" stdDeviation="8" flood-color="#000000" flood-opacity="0.30"/>
    </filter>

    <!-- Base Shadow -->
    <filter id="plate_shadow" x="-10%" y="-10%" width="120%" height="120%">
      <feDropShadow dx="0" dy="12" stdDeviation="14" flood-color="#050a14" flood-opacity="0.30"/>
    </filter>
  </defs>

  <!-- 1. Adwaita Squircle Base -->
  <rect x="36" y="48" width="440" height="440" rx="99" fill="url(#bevel_bg)" filter="url(#plate_shadow)"/>
  <rect x="36" y="36" width="440" height="440" rx="99" fill="url(#adwaita_bg)"/>
  <rect x="38" y="38" width="436" height="436" rx="97" fill="none" stroke="#ffffff" stroke-width="2" opacity="0.25"/>

  <!-- 2. Background Soundwaves Spectrum -->
  <g fill="#ffffff" opacity="0.20">
    <rect x="74"  y="227" width="18" height="58"  rx="9"/>
    <rect x="110" y="203" width="18" height="106" rx="9"/>
    <rect x="146" y="170" width="18" height="172" rx="9"/>
    <rect x="182" y="139" width="18" height="234" rx="9"/>
    <rect x="218" y="187" width="18" height="138" rx="9"/>
    <rect x="254" y="130" width="18" height="252" rx="9"/>
    <rect x="290" y="187" width="18" height="138" rx="9"/>
    <rect x="326" y="139" width="18" height="234" rx="9"/>
    <rect x="362" y="170" width="18" height="172" rx="9"/>
    <rect x="398" y="203" width="18" height="106" rx="9"/>
    <rect x="434" y="227" width="18" height="58"  rx="9"/>
  </g>

  <!-- 3. Foreground Studio Microphone & AI Sparkles (with Shadow) -->
  <g filter="url(#mic_shadow)" fill="#ffffff" stroke="#ffffff" stroke-linecap="round" stroke-linejoin="round">
    <!-- Central Capsule -->
    <rect x="212" y="148" width="88" height="130" rx="44" stroke-width="0"/>
    <!-- Grill Detail -->
    <line x1="220" y1="202" x2="292" y2="202" stroke="#000000" stroke-width="2.5" opacity="0.2"/>

    <!-- U-Arc Holder -->
    <path d="M 190 236 A 66 66 0 0 0 322 236" fill="none" stroke-width="20"/>

    <!-- Stem and Base -->
    <line x1="256" y1="300" x2="256" y2="328" stroke-width="20"/>
    <line x1="210" y1="338" x2="302" y2="338" stroke-width="20"/>

    <!-- Primary AI Sparkle (Intelligence Symbol) -->
    <path d="M 366 102 Q 366 150 414 150 Q 366 150 366 198 Q 366 150 318 150 Q 366 150 366 102 Z" stroke-width="0"/>

    <!-- Secondary Micro Sparkle -->
    <path d="M 148 156 Q 148 178 170 178 Q 148 178 148 200 Q 148 178 126 178 Q 148 178 148 156 Z" stroke-width="0" opacity="0.90"/>
  </g>
</svg>
"""
    with open(path, "w", encoding="utf-8") as f:
        f.write(svg)


# ==============================================================================
# Execution
# ==============================================================================
def main():
    os.makedirs(OUT_DIR, exist_ok=True)

    # 1. Main App Icons
    make_app_icon(512).save(os.path.join(OUT_DIR, "otush.png"))
    make_app_icon(128).save(os.path.join(OUT_DIR, "otush-128.png"))
    write_scalable_svg(os.path.join(OUT_DIR, "otush-icon.svg"))

    # 2. Symbolic Tray Icons (Dark Panel)
    make_tray_symbolic(DARK_PANEL_TRAY).save(os.path.join(OUT_DIR, "tray_idle.png"))
    make_tray_symbolic(DARK_PANEL_TRAY, recording=True).save(os.path.join(OUT_DIR, "tray_recording.png"))
    make_tray_symbolic(DARK_PANEL_TRAY, transcribing=True).save(os.path.join(OUT_DIR, "tray_transcribing.png"))
    make_tray_symbolic(DARK_PANEL_TRAY, warning=True).save(os.path.join(OUT_DIR, "tray_idle_warning.png"))

    # 3. Symbolic Tray Icons (Light Panel)
    make_tray_symbolic(LIGHT_PANEL_TRAY).save(os.path.join(OUT_DIR, "tray_idle_dark.png"))
    make_tray_symbolic(LIGHT_PANEL_TRAY, recording=True).save(os.path.join(OUT_DIR, "tray_recording_dark.png"))
    make_tray_symbolic(LIGHT_PANEL_TRAY, transcribing=True).save(os.path.join(OUT_DIR, "tray_transcribing_dark.png"))
    make_tray_symbolic(LIGHT_PANEL_TRAY, warning=True).save(os.path.join(OUT_DIR, "tray_idle_warning_dark.png"))

    # 4. Colored Status Badges (Overlay / UI) - without lower bevel for symmetric shape
    make_app_icon(64, is_recording=True, has_bevel=False).save(os.path.join(OUT_DIR, "recording.png"))
    make_app_icon(64, is_recording=False, has_bevel=False).save(os.path.join(OUT_DIR, "transcribing.png"))

    print(f"✓ Otush resources generated successfully in: {os.path.abspath(OUT_DIR)}")


if __name__ == "__main__":
    main()