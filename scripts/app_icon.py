#!/usr/bin/env python3
"""Generate the TableX desktop icons from the artwork PNG.

Run from the repo root: python3 scripts/app_icon.py <artwork.png>
The artwork is either a rounded square on opaque black (its corners are cut to transparent here)
or already transparent (its empty margin is trimmed).
Writes assets/icons/app/tablex.png (Linux, 512), tablex-64.png (title bar), tablex.ico (Windows) and tablex.icns (macOS).
"""
import sys
from pathlib import Path

from PIL import Image, ImageDraw

OUT = Path("assets/icons/app")
# Measured on the artwork: the rounded square fills the canvas with a ~215 px corner radius.
RADIUS = 215
SS = 4

art = Image.open(sys.argv[1]).convert("RGBA")
if art.getpixel((0, 0))[3] == 255:
    w, h = art.size
    mask = Image.new("L", (w * SS, h * SS), 0)
    ImageDraw.Draw(mask).rounded_rectangle((0, 0, w * SS - 1, h * SS - 1), RADIUS * SS, fill=255)
    art.putalpha(mask.resize((w, h), Image.LANCZOS))
else:
    # Already cut out: trim the empty margin so the shape fills the icon.
    art = art.crop(art.getchannel("A").point(lambda a: 255 if a > 8 else 0).getbbox())
w, h = art.size

side = max(w, h)
square = Image.new("RGBA", (side, side), (0, 0, 0, 0))
square.paste(art, ((side - w) // 2, (side - h) // 2))
master = square.resize((1024, 1024), Image.LANCZOS)

OUT.mkdir(parents=True, exist_ok=True)
master.resize((512, 512), Image.LANCZOS).save(OUT / "tablex.png", optimize=True)
# the title bar shows it at ~18 px; 64 keeps it sharp on 2x+ screens without embedding the 512
master.resize((64, 64), Image.LANCZOS).save(OUT / "tablex-64.png", optimize=True)
master.save(OUT / "tablex.ico", sizes=[(s, s) for s in (16, 24, 32, 48, 64, 128, 256)])
master.save(OUT / "tablex.icns")
