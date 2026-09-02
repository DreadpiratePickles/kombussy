#!/usr/bin/env python3
"""Prove that a mature implementation can read what kombussy writes.

The Rust test suite checks that we read fontTools' output. This checks the other
direction, which is the half that actually matters for a converter: a file only
counts as a WOFF2 if something other than its own encoder accepts it.

Run with:
  uv run --with "fonttools[woff]" --with brotli python3 fixtures/verify_interop.py
"""
from __future__ import annotations

import subprocess
import sys
import tempfile
from pathlib import Path

from fontTools.pens.recordingPen import DecomposingRecordingPen
from fontTools.ttLib import TTFont

ROOT = Path(__file__).resolve().parent.parent
FIXTURES = ROOT / "fixtures"
CLI = ROOT / "target" / "release" / "kombussy"
# Derived from the whole file, or set by WOFF2 producers; see corpus.rs.
VOLATILE_HEAD_FIELDS = ("checkSumAdjustment", "flags")
COMPARED_TABLES = ("glyf", "cmap", "hmtx", "hhea", "maxp", "name", "OS/2", "post", "loca", "CFF ")

failures: list[str] = []


def check(condition: bool, message: str) -> None:
    status = "ok  " if condition else "FAIL"
    print(f"  [{status}] {message}")
    if not condition:
        failures.append(message)


def convert(source: Path, target: str, destination: Path) -> None:
    subprocess.run(
        [str(CLI), "--to", target, str(source), "--output", str(destination)],
        check=True,
        capture_output=True,
    )


def table_signature(font: TTFont, tag: str) -> bytes | None:
    """Compiled bytes for one table, with volatile head fields neutralised."""
    if tag not in font:
        return None
    if tag == "head":
        head = font["head"]
        saved = {f: getattr(head, f) for f in VOLATILE_HEAD_FIELDS}
        for f in VOLATILE_HEAD_FIELDS:
            setattr(head, f, 0)
        try:
            return head.compile(font)
        finally:
            for f, v in saved.items():
                setattr(head, f, v)
    return font[tag].compile(font)


def compare(produced: Path, reference: Path, label: str) -> None:
    ours, theirs = TTFont(produced), TTFont(reference)
    check(ours.sfntVersion == theirs.sfntVersion, f"{label}: outline flavor preserved")
    check(
        sorted(ours.keys()) == sorted(theirs.keys()),
        f"{label}: table inventory preserved ({len(ours.keys())} tables)",
    )
    check(
        ours.getGlyphOrder() == theirs.getGlyphOrder(),
        f"{label}: glyph order preserved ({len(ours.getGlyphOrder())} glyphs)",
    )
    for tag in COMPARED_TABLES:
        if tag in theirs and tag in ours:
            check(table_signature(ours, tag) == table_signature(theirs, tag), f"{label}: '{tag}' byte-identical")
    check(table_signature(ours, "head") == table_signature(theirs, "head"), f"{label}: 'head' identical modulo derived fields")


def outlines(path: Path) -> dict[str, list]:
    """Fully decomposed drawing operations per glyph.

    The WOFF2 glyf transform preserves outlines, not bytes: the encoder discards
    the original's flag repetition and coordinate widths. Comparing recorded pen
    operations tests what the format actually promises. Decomposing also expands
    composite glyphs, so a mis-reconstructed component shows up as a coordinate
    difference rather than passing unnoticed.
    """
    font = TTFont(path)
    glyph_set = font.getGlyphSet()
    recorded = {}
    for name in font.getGlyphOrder():
        pen = DecomposingRecordingPen(glyph_set)
        glyph_set[name].draw(pen)
        recorded[name] = pen.value
    return recorded


def compare_outlines(produced: Path, reference: Path, label: str) -> None:
    ours, theirs = outlines(produced), outlines(reference)
    check(sorted(ours) == sorted(theirs), f"{label}: glyph set preserved ({len(theirs)} glyphs)")
    mismatched = [n for n in theirs if ours.get(n) != theirs[n]]
    check(not mismatched, f"{label}: all glyph outlines identical" + (f" (differs: {mismatched})" if mismatched else ""))
    # DecomposingRecordingPen expands components, so composite structure has to
    # be compared separately or a composite rebuilt as a simple glyph would pass.
    ours_font, theirs_font = TTFont(produced), TTFont(reference)
    if "glyf" in ours_font and "glyf" in theirs_font:
        ours_glyf, theirs_glyf = ours_font["glyf"], theirs_font["glyf"]
        expected = {n for n in theirs_font.getGlyphOrder() if theirs_glyf[n].isComposite()}
        actual = {n for n in ours_font.getGlyphOrder() if ours_glyf[n].isComposite()}
        check(expected == actual, f"{label}: composite glyphs stayed composite ({sorted(expected)})")
        for name in sorted(expected & actual):
            ref = [(c.glyphName, c.x, c.y) for c in theirs_glyf[name].components]
            got = [(c.glyphName, c.x, c.y) for c in ours_glyf[name].components]
            check(ref == got, f"{label}: composite '{name}' components identical")


def main() -> int:
    if not CLI.exists():
        print(f"error: {CLI} not built. Run: cargo build --release -p kombussy-cli", file=sys.stderr)
        return 2

    source = FIXTURES / "synthetic.ttf"
    cff_source = FIXTURES / "synthetic.otf"

    with tempfile.TemporaryDirectory() as tmp:
        out = Path(tmp)

        print("fontTools reads kombussy output (truetype outlines)")
        for target, name in (("woff", "kombussy.woff"), ("woff2", "kombussy.woff2"), ("ttf", "kombussy.ttf")):
            produced = out / name
            convert(source, target, produced)
            compare(produced, source, f"ttf -> {target}")

        print("fontTools reads kombussy output (cff outlines)")
        for target, name in (("woff", "cff.woff"), ("woff2", "cff.woff2")):
            produced = out / name
            convert(cff_source, target, produced)
            compare(produced, cff_source, f"otf -> {target}")

        print("glyf transform reconstruction (fontTools default output)")
        rebuilt = out / "from_transformed.ttf"
        convert(FIXTURES / "synthetic_transformed.woff2", "ttf", rebuilt)
        compare_outlines(rebuilt, source, "transformed woff2 -> ttf")

        print("kombussy reads fontTools output")
        for fixture, label in (("synthetic.woff", "woff"), ("synthetic_null.woff2", "woff2")):
            produced = out / f"from_{label}.ttf"
            convert(FIXTURES / fixture, "ttf", produced)
            compare(produced, source, f"fontTools {label} -> ttf")

        print("size behaviour")
        convert(source, "woff", out / "s.woff")
        convert(source, "woff2", out / "s.woff2")
        w1, w2 = (out / "s.woff").stat().st_size, (out / "s.woff2").stat().st_size
        check(w2 < w1, f"woff2 ({w2} B) smaller than woff ({w1} B)")

    print()
    if failures:
        print(f"{len(failures)} interop check(s) FAILED")
        return 1
    print("all interop checks passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
