"""Subset the bundled OFL font for the static website (build-time only).

Requires fonttools[woff] / brotli. The generated WOFF2 files are checked in
under site/assets, so ordinary site builds need no extra dependencies.
Run this again when the translated copy gains new characters.
"""
from __future__ import annotations

import json
import shutil
from pathlib import Path

from fontTools import subset
from fontTools.ttLib import TTFont

ROOT = Path(__file__).resolve().parent.parent
ASSETS = ROOT / "site" / "assets"
FONTS = ROOT / "vrclt" / "assets" / "fonts"


def main() -> None:
    content = json.loads((ROOT / "site" / "content.json").read_text(encoding="utf-8"))
    text = "".join(chr(i) for i in range(32, 127)) + "↓↗→·—⌄"
    text += "".join(value for locale in content.values() for value in locale.values())
    ASSETS.mkdir(parents=True, exist_ok=True)
    for weight in ("Regular", "Bold"):
        font = TTFont(FONTS / f"NotoSansCJKkr-{weight}.otf")
        missing = set(map(ord, text)) - set(font.getBestCmap())
        # Arrows and decorative symbols can use the system font fallback.
        missing -= set(map(ord, "↓↗→·—⌄"))
        assert not missing, f"Missing characters: {missing}"
        options = subset.Options()
        options.flavor = "woff2"
        options.recalc_timestamp = False
        subsetter = subset.Subsetter(options=options)
        subsetter.populate(text=text)
        subsetter.subset(font)
        font.flavor = "woff2"
        output = ASSETS / f"vrclt-sans-{weight.lower()}.woff2"
        font.save(output)
        print(f"{output.name}: {output.stat().st_size:,} bytes")
    shutil.copyfile(FONTS / "LICENSE-NotoSansCJK.txt", ASSETS / "LICENSE-NotoSansCJK.txt")


if __name__ == "__main__":
    main()
