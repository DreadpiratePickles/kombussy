#!/usr/bin/env python3
"""Build the test corpus for kombussy.

The fonts here are synthesised from scratch rather than copied from the system,
so the fixtures carry no third-party licence and stay small enough to commit.
fontTools writes the WOFF and WOFF2 variants, which makes them independent
ground truth: if our Rust codec agrees with these bytes it agrees with a mature
implementation, not merely with itself.

Run with:  uv run --with "fonttools[woff]" --with brotli python3 fixtures/generate.py
"""
from __future__ import annotations

import sys
from pathlib import Path

from fontTools.fontBuilder import FontBuilder
from fontTools.pens.ttGlyphPen import TTGlyphPen
from fontTools.pens.t2CharStringPen import T2CharStringPen
from fontTools.ttLib import woff2

OUT = Path(__file__).parent
UNITS_PER_EM = 1000
# A square, a triangle and a bar: enough contour data that a corrupted glyf
# table changes the bytes, while keeping the fixture tiny.
SHAPES: dict[str, list[list[tuple[int, int]]]] = {
    "A": [[(50, 0), (450, 0), (450, 700), (50, 700)]],
    "B": [[(50, 0), (450, 0), (250, 700)]],
    "C": [[(50, 200), (450, 200), (450, 500), (50, 500)]],
}
# "D" is a composite referencing "A", and "E" references two components with an
# offset. Composite glyphs take a completely separate branch through the WOFF2
# glyf transform, so the corpus has to contain them.
COMPOSITES: dict[str, list[tuple[str, int, int]]] = {
    "D": [("A", 0, 0)],
    "E": [("A", 0, 0), ("B", 120, 60)],
}
GLYPH_ORDER = [".notdef", *SHAPES, *COMPOSITES]
CHARACTER_MAP = {ord(name): name for name in list(SHAPES) + list(COMPOSITES)}


def _advance_widths() -> dict[str, int]:
    return {name: 500 for name in GLYPH_ORDER}


def _common_metadata(fb: FontBuilder, family: str) -> None:
    fb.setupHorizontalHeader(ascent=800, descent=-200, lineGap=0)
    fb.setupNameTable(
        {
            "familyName": family,
            "styleName": "Regular",
            "uniqueFontIdentifier": f"kombussy.fixtures.{family}",
            "fullName": f"{family} Regular",
            "psName": family.replace(" ", ""),
            "version": "Version 1.000",
        }
    )
    fb.setupOS2(sTypoAscender=800, sTypoDescender=-200, usWinAscent=800, usWinDescent=200)
    fb.setupPost()


def build_truetype() -> FontBuilder:
    """A glyf-outline font: the common case, and the one WOFF2 transforms."""
    fb = FontBuilder(UNITS_PER_EM, isTTF=True)
    fb.setupGlyphOrder(GLYPH_ORDER)
    fb.setupCharacterMap(CHARACTER_MAP)

    glyphs = {".notdef": TTGlyphPen(None).glyph()}
    for name, contours in SHAPES.items():
        pen = TTGlyphPen(None)
        for contour in contours:
            pen.moveTo(contour[0])
            for point in contour[1:]:
                pen.lineTo(point)
            pen.closePath()
        glyphs[name] = pen.glyph()

    for name, components in COMPOSITES.items():
        pen = TTGlyphPen(glyphs)
        for base, dx, dy in components:
            pen.addComponent(base, (1, 0, 0, 1, dx, dy))
        glyphs[name] = pen.glyph()

    fb.setupGlyf(glyphs)
    fb.setupHorizontalMetrics({n: (w, 50) for n, w in _advance_widths().items()})
    _common_metadata(fb, "Kombussy Fixture TT")
    return fb


def build_cff() -> FontBuilder:  # noqa: D401
    """A CFF-outline font, so the 'OTTO' flavor path is covered too."""
    order = [".notdef", *SHAPES]
    fb = FontBuilder(UNITS_PER_EM, isTTF=False)
    fb.setupGlyphOrder(order)
    fb.setupCharacterMap({ord(n): n for n in SHAPES})

    charstrings = {}
    for name in order:
        pen = T2CharStringPen(500, None)
        for contour in SHAPES.get(name, []):
            pen.moveTo(contour[0])
            for point in contour[1:]:
                pen.lineTo(point)
            pen.closePath()
        charstrings[name] = pen.getCharString()

    fb.setupCFF("KombussyFixtureCFF", {"FullName": "Kombussy Fixture CFF"}, charstrings, {})
    fb.setupHorizontalMetrics({n: (500, 50) for n in order})
    _common_metadata(fb, "Kombussy Fixture CFF")
    return fb


def save(fb: FontBuilder, path: Path, flavor: str | None, transformed: bool = True) -> None:
    font = fb.font
    font.flavor = flavor
    # WOFF2 makes the glyf/loca transform optional. Emitting both variants lets
    # the Rust tests cover the null-transform path we implement and assert a
    # clean typed error on the transformed one we do not.
    original = woff2.woff2TransformedTableTags
    if not transformed:
        woff2.woff2TransformedTableTags = ()
    try:
        font.save(path)
    finally:
        woff2.woff2TransformedTableTags = original
    print(f"  {path.name:<34} {path.stat().st_size:>7,} bytes")


def main() -> int:
    print("truetype (glyf outlines)")
    save(build_truetype(), OUT / "synthetic.ttf", None)
    save(build_truetype(), OUT / "synthetic.woff", "woff")
    save(build_truetype(), OUT / "synthetic_null.woff2", "woff2", transformed=False)
    save(build_truetype(), OUT / "synthetic_transformed.woff2", "woff2", transformed=True)

    print("cff (postscript outlines)")
    save(build_cff(), OUT / "synthetic.otf", None)
    save(build_cff(), OUT / "synthetic_cff.woff", "woff")
    save(build_cff(), OUT / "synthetic_cff.woff2", "woff2", transformed=False)
    return 0


if __name__ == "__main__":
    sys.exit(main())
