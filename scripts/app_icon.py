#!/usr/bin/env python3
"""Generate the QueryX desktop icons from the artwork PNG.

Run from the repo root: python3 scripts/app_icon.py <artwork.png>
The artwork is a rounded square on opaque black; the corners are cut to transparent here.
Writes assets/icons/app/queryx.png (Linux, 512), queryx-64.png (title bar), queryx.ico (Windows) and queryx.icns (macOS).
"""
import sys
from pathlib import Path

from PIL import Image, ImageDraw

OUT = Path("assets/icons/app")
# Measured on the artwork: the rounded square fills the canvas with a ~215 px corner radius.
RADIUS = 215
SS = 4

art = Image.open(sys.argv[1]).convert("RGBA")
w, h = art.size
mask = Image.new("L", (w * SS, h * SS), 0)
ImageDraw.Draw(mask).rounded_rectangle((0, 0, w * SS - 1, h * SS - 1), RADIUS * SS, fill=255)
art.putalpha(mask.resize((w, h), Image.LANCZOS))

side = max(w, h)
square = Image.new("RGBA", (side, side), (0, 0, 0, 0))
square.paste(art, ((side - w) // 2, (side - h) // 2))
master = square.resize((1024, 1024), Image.LANCZOS)

OUT.mkdir(parents=True, exist_ok=True)
master.resize((512, 512), Image.LANCZOS).save(OUT / "queryx.png", optimize=True)
# the title bar shows it at ~18 px; 64 keeps it sharp on 2x+ screens without embedding the 512
master.resize((64, 64), Image.LANCZOS).save(OUT / "queryx-64.png", optimize=True)
master.save(OUT / "queryx.ico", sizes=[(s, s) for s in (16, 24, 32, 48, 64, 128, 256)])
master.save(OUT / "queryx.icns")
