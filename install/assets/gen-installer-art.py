#!/usr/bin/env python3
"""Deterministic generator for the OccluView MSI WixUI artwork.

Produces two 24-bit BMP files consumed by WixUIExtension:

    WixUIBannerBmp.bmp   493 x 58    top banner (License/InstallDir/Progress)
    WixUIDialogBmp.bmp   493 x 312   full background (Welcome/Exit dialogs)

Design intent: strict, open-source, lightly stylish. Both images are built
from the REAL product logo the app itself ships -- the open occlusal ring with
a red occlusion point (crates/occluview-app/assets/windows/occluview.png, a.k.a.
assets/occluview-logo.png). We composite that PNG directly (LANCZOS downscale)
rather than re-drawing a mark, so the installer matches the shipped icon.

WiX overlays the standard dialog title/description text in BLACK, so the
regions where that text lands are kept light:
  * Banner  - the LEFT column stays clear for the WiX title; branding sits on
              the right edge; a 1px hairline runs along the bottom.
  * Dialog  - a light SIDEBAR band (left 164 px) carries the branding; the
              right panel (where WiX draws the Welcome/Exit text) is plain
              white, separated by a 1px hairline seam.

No network, no randomness: same inputs -> byte-identical output.

Run (from the repository root):
    python3 install/assets/gen-installer-art.py

Requires Pillow (PIL). Fonts: DejaVu Sans (available on the build box);
falls back to Liberation Sans (Arial-metric) if DejaVu is unavailable.
"""

from __future__ import annotations

import os
from PIL import Image, ImageDraw, ImageFont

HERE = os.path.dirname(os.path.abspath(__file__))
# Source logo: the exact PNG the desktop app ships (open ring + red point).
LOGO_SRC = os.path.join(
    HERE, "..", "..", "crates", "occluview-app", "assets", "windows", "occluview.png"
)

# Brand palette (strict / open-source, near-white surfaces) ------------------
# The near-white surfaces use the logo tile's exact interior colour (#FCFCFB)
# so the cropped ring mark composites with no visible tile seam.
BANNER_BG = (252, 252, 251)   # #FCFCFB  banner background (== logo tile)
SIDEBAR_BG = (252, 252, 251)  # #FCFCFB  dialog sidebar band (== logo tile)
PANEL_BG = (255, 255, 255)    # #FFFFFF  dialog right text panel
HAIRLINE = (226, 230, 234)    # #E2E6EA  thin separators
WORDMARK = (30, 40, 54)       # #1E2836  dark slate wordmark
TAGLINE = (139, 151, 167)     # #8B97A7  muted tagline

DIALOG_W, DIALOG_H = 493, 312
BANNER_W, BANNER_H = 493, 58
SIDEBAR_W = 164


def _font(names, size):
    roots = [
        "/usr/share/fonts/truetype/dejavu",
        "/usr/share/fonts/truetype/liberation",
    ]
    for name in names:
        for root in roots:
            path = os.path.join(root, name)
            if os.path.exists(path):
                return ImageFont.truetype(path, size)
    return ImageFont.load_default()


def font_bold(size):
    return _font(["DejaVuSans-Bold.ttf", "LiberationSans-Bold.ttf"], size)


def font_regular(size):
    return _font(["DejaVuSans.ttf", "LiberationSans-Regular.ttf"], size)


def _load_logo():
    return Image.open(LOGO_SRC).convert("RGBA")


def _content_bbox(logo):
    """Tight bbox of the ink (ring + dot) inside the rounded tile."""
    # Flatten onto the tile colour, then find pixels that deviate from it.
    tile = (252, 252, 251)
    bg = Image.new("RGBA", logo.size, tile + (255,))
    flat = Image.alpha_composite(bg, logo).convert("RGB")
    px = flat.load()
    w, h = flat.size
    minx, miny, maxx, maxy = w, h, 0, 0
    for y in range(h):
        for x in range(w):
            r, g, b = px[x, y]
            if abs(r - tile[0]) + abs(g - tile[1]) + abs(b - tile[2]) > 40:
                if x < minx:
                    minx = x
                if x > maxx:
                    maxx = x
                if y < miny:
                    miny = y
                if y > maxy:
                    maxy = y
    return (minx, miny, maxx + 1, maxy + 1)


def logo_mark(logo, height, bg):
    """Cropped ring+dot mark, LANCZOS-scaled to `height`, flattened on `bg`.

    Returns a square RGB image. The rounded tile is cropped away so the mark
    reads as the ring floating on the installer surface; the residual tile
    interior colour (#fcfcfb) is visually identical to the near-white surface.
    """
    minx, miny, maxx, maxy = _content_bbox(logo)
    pad = 12  # keep a little air around the ring
    box = (
        max(0, minx - pad),
        max(0, miny - pad),
        min(logo.width, maxx + pad),
        min(logo.height, maxy + pad),
    )
    crop = logo.crop(box)
    # square it so the ring is not distorted
    side = max(crop.width, crop.height)
    sq = Image.new("RGBA", (side, side), (0, 0, 0, 0))
    sq.alpha_composite(crop, ((side - crop.width) // 2, (side - crop.height) // 2))
    surface = Image.new("RGBA", (side, side), bg + (255,))
    surface.alpha_composite(sq)
    return surface.convert("RGB").resize((height, height), Image.LANCZOS)


def logo_ink(logo):
    """RGBA of the ink only (ring + dot) on a transparent field.

    Alpha follows darkness so the near-white tile drops out; used faint as a
    watermark. Kept neutral-ish; the red dot survives at reduced alpha.
    """
    tile = (252, 252, 251)
    bg = Image.new("RGBA", logo.size, tile + (255,))
    flat = Image.alpha_composite(bg, logo).convert("RGB")
    px = flat.load()
    out = Image.new("RGBA", logo.size, (0, 0, 0, 0))
    op = out.load()
    w, h = flat.size
    for y in range(h):
        for x in range(w):
            r, g, b = px[x, y]
            lum = (0.299 * r + 0.587 * g + 0.114 * b)
            a = int(max(0, min(255, 255 - lum)))
            if a > 0:
                op[x, y] = (r, g, b, a)
    return out


def build_banner(logo):
    img = Image.new("RGB", (BANNER_W, BANNER_H), BANNER_BG)
    d = ImageDraw.Draw(img)

    # 1px hairline along the bottom edge.
    d.line([(0, BANNER_H - 1), (BANNER_W, BANNER_H - 1)], fill=HAIRLINE, width=1)

    # Right-aligned brand block: wordmark then the ring mark at the far edge.
    mark_h = 40
    mark = logo_mark(logo, mark_h, BANNER_BG)
    right_margin = 16
    mx = BANNER_W - right_margin - mark_h
    my = (BANNER_H - mark_h) // 2
    img.paste(mark, (mx, my))

    wm_font = font_bold(22)
    text = "OccluView"
    tb = d.textbbox((0, 0), text, font=wm_font)
    tw, th = tb[2] - tb[0], tb[3] - tb[1]
    gap = 12
    tx = mx - gap - tw - tb[0]
    ty = (BANNER_H - th) // 2 - tb[1]
    d.text((tx, ty), text, font=wm_font, fill=WORDMARK)
    return img


def build_dialog(logo):
    img = Image.new("RGB", (DIALOG_W, DIALOG_H), PANEL_BG)
    d = ImageDraw.Draw(img)

    # Light sidebar band + 1px hairline seam against the white text panel.
    d.rectangle([0, 0, SIDEBAR_W - 1, DIALOG_H], fill=SIDEBAR_BG)
    d.line([(SIDEBAR_W, 0), (SIDEBAR_W, DIALOG_H)], fill=HAIRLINE, width=1)

    # Faint ring watermark near the sidebar bottom (subtle, ~5% opacity).
    wm_size = 150
    ink = logo_ink(logo).resize((wm_size, wm_size), Image.LANCZOS)
    faded = Image.new("RGBA", ink.size, (0, 0, 0, 0))
    ia = ink.getchannel("A").point(lambda a: int(a * 0.05))
    faded.putdata(list(ink.getdata()))
    faded.putalpha(ia)
    base = img.convert("RGBA")
    wx = (SIDEBAR_W - wm_size) // 2
    wy = DIALOG_H - wm_size + 22  # partially tucked under the bottom edge
    base.alpha_composite(faded, (wx, wy))
    img = base.convert("RGB")
    d = ImageDraw.Draw(img)

    # Centred logo mark in the sidebar upper third.
    mark_h = 78
    mark = logo_mark(logo, mark_h, SIDEBAR_BG)
    mx = (SIDEBAR_W - mark_h) // 2
    img.paste(mark, (mx, 42))

    # Wordmark beneath the mark.
    wm_font = font_bold(23)
    text = "OccluView"
    tb = d.textbbox((0, 0), text, font=wm_font)
    tx = (SIDEBAR_W - (tb[2] - tb[0])) // 2 - tb[0]
    d.text((tx, 138), text, font=wm_font, fill=WORDMARK)

    # Small, weak tagline (wrap to two centred lines to fit the sidebar).
    tag_font = font_regular(11)
    lines = ["Fast 3D viewer", "for dental scans"]
    ty = 170
    for line in lines:
        lb = d.textbbox((0, 0), line, font=tag_font)
        lx = (SIDEBAR_W - (lb[2] - lb[0])) // 2 - lb[0]
        d.text((lx, ty), line, font=tag_font, fill=TAGLINE)
        ty += 16
    return img


def main():
    logo = _load_logo()
    banner = build_banner(logo)
    dialog = build_dialog(logo)
    assert banner.size == (BANNER_W, BANNER_H), banner.size
    assert dialog.size == (DIALOG_W, DIALOG_H), dialog.size
    assert banner.mode == "RGB" and dialog.mode == "RGB"
    bpath = os.path.join(HERE, "WixUIBannerBmp.bmp")
    dpath = os.path.join(HERE, "WixUIDialogBmp.bmp")
    banner.save(bpath, "BMP")
    dialog.save(dpath, "BMP")
    print("wrote", bpath, banner.size)
    print("wrote", dpath, dialog.size)


if __name__ == "__main__":
    main()
