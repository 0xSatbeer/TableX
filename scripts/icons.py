#!/usr/bin/env python3
"""Generate QueryX object icons: one item glyph per kind, plus a "stack" variant for its group.

Run from the repo root: python3 scripts/icons.py
Writes assets/icons/queryx/<kind>.svg and <kind>s.svg. GPUI renders SVGs as single-color
masks, so only shape and alpha matter; color is applied at render time.
"""
from pathlib import Path

OUT = Path("assets/icons/queryx")

# A box with a solid header bar: the "this holds rows" family (table, view).
HEADER = '<path d="M3 9V7a4 4 0 0 1 4-4h10a4 4 0 0 1 4 4v2Z" fill="#000"/>'
BODY = '<path d="M3 9v10a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2V9"/>'
BOX = '<rect x="3" y="3" width="18" height="18" rx="3"/>'

GLYPHS = {
    "table": HEADER + BODY + '<path d="M3 15h18M12 9v12"/>',
    "view": HEADER + BODY
    + '<path d="M6.5 15c1.5-2.3 3.4-3.4 5.5-3.4s4 1.1 5.5 3.4c-1.5 2.3-3.4 3.4-5.5 3.4s-4-1.1-5.5-3.4Z" stroke-width="1.6"/>'
    + '<circle cx="12" cy="15" r="1.3" fill="#000" stroke="none"/>',
    # A view whose result is stored: the eye is filled in.
    "matview": HEADER + BODY
    + '<path d="M6.5 15c1.5-2.3 3.4-3.4 5.5-3.4s4 1.1 5.5 3.4c-1.5 2.3-3.4 3.4-5.5 3.4s-4-1.1-5.5-3.4Z" stroke-width="1.6" fill="#000" fill-opacity="0.45"/>'
    + '<circle cx="12" cy="15" r="1.3" fill="#000" stroke="none"/>',
    # Rising bars: values that count up.
    "sequence": BOX + '<path d="M8 17v-3M12 17v-6M16 17V8"/>',
    # Lucide square-function (ISC).
    "function": BOX + '<path d="M9 17c2 0 2.8-1 2.8-2.8V10c0-2 1-3.3 3.2-3"/><path d="M9 11.2h5.7"/>',
    # Play: a routine you call to run.
    "procedure": BOX + '<path d="M10 8.5v7l5.5-3.5Z"/>',
    # A list of named values.
    "type": BOX + '<path d="M8 8h.01M8 12h.01M8 16h.01M11.5 8H16M11.5 12H16M11.5 16H16"/>',
    # Sorted keys: a down arrow beside shrinking lines.
    "index": BOX + '<path d="M8 7v10M6 15l2 2 2-2M12.5 8H17M12.5 12h3.5M12.5 16H15"/>',
    # Lightning: fires on an event.
    "trigger": BOX + '<path d="M13 6.5 9 12.5h4l-1 5 4-6h-4Z"/>',
}

# The edge of a second box peeking out behind the front one.
BACK = '<path d="M7.5 5.5V5a2 2 0 0 1 2-2h9.5a2 2 0 0 1 2 2v9.5a2 2 0 0 1-2 2h-.5"/>'
# Shrink the front glyph to 75%, keeping its strokes at 2px on screen.
FRONT = '<g transform="translate(0.5 5.5) scale(0.75)" stroke-width="2.667">{}</g>'


def svg(body: str) -> str:
    return (
        '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#000" '
        f'stroke-width="2" stroke-linecap="round" stroke-linejoin="round">{body}</svg>\n'
    )


OUT.mkdir(parents=True, exist_ok=True)
for kind, glyph in GLYPHS.items():
    (OUT / f"{kind}.svg").write_text(svg(glyph))
    (OUT / f"{kind}s.svg").write_text(svg(BACK + FRONT.format(glyph)))
    print(f"{kind}.svg {kind}s.svg")
