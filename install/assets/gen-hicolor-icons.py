#!/usr/bin/env python3
"""Deterministic hicolor app-icon set for the Debian package.

Downscales the shipped 512x512 product logo (assets/occluview-logo.png)
to every freedesktop hicolor size the .deb installs, so launchers,
docks, and software centers find a size-appropriate icon instead of
only a 512px file. Same inputs -> byte-identical outputs.

Run (from the repository root):
    python3 install/assets/gen-hicolor-icons.py

Requires Pillow (PIL). Outputs are committed (like the MSI BMP art):
install/assets/icons/hicolor/<size>x<size>/apps/occluview.png
"""

from __future__ import annotations

import os

from PIL import Image

HERE = os.path.dirname(os.path.abspath(__file__))
LOGO_SRC = os.path.join(HERE, "..", "..", "assets", "occluview-logo.png")
OUT_ROOT = os.path.join(HERE, "icons", "hicolor")

# Every size build-deb.sh installs for the app icon.
SIZES = (16, 22, 24, 32, 48, 64, 128, 256, 512)


def main() -> None:
    master = Image.open(LOGO_SRC)
    assert master.size == (512, 512), f"unexpected logo size: {master.size}"
    for size in SIZES:
        out_dir = os.path.join(OUT_ROOT, f"{size}x{size}", "apps")
        os.makedirs(out_dir, exist_ok=True)
        out_path = os.path.join(out_dir, "occluview.png")
        resized = master.resize((size, size), Image.LANCZOS)
        resized.save(out_path)
        print(f"wrote {out_path}")


if __name__ == "__main__":
    main()
