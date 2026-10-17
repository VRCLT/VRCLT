"""Build app icons and README logos from the editable SVG mark.

Run: .venv/Scripts/python scripts/make_icon.py

Uses the project's existing PySide6 and Pillow dependencies. Wordmark outlines
come from the bundled, OFL-licensed Pretendard JP Bold; exported SVGs need no
installed fonts, web fonts, scripts, or external resources.
"""
from __future__ import annotations

import io
import os
from pathlib import Path
import xml.etree.ElementTree as ET

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

from PIL import Image, ImageDraw, ImageFont
from PySide6 import QtCore, QtGui, QtSvg

ROOT = Path(__file__).resolve().parent.parent
ASSETS = ROOT / "vrclt" / "assets"
BRAND = ROOT / "docs" / "brand"
FONT = ASSETS / "fonts" / "PretendardJP-Bold.otf"
ICO_SIZES = [(n, n) for n in (16, 20, 24, 32, 40, 48, 64, 128, 256)]
INK = "#192C3D"
PAPER = "#F5F3ED"


def render_svg(source: bytes, width: int, height: int) -> Image.Image:
    renderer = QtSvg.QSvgRenderer(source)
    if not renderer.isValid():
        raise ValueError("Invalid brand SVG")
    canvas = QtGui.QImage(width, height, QtGui.QImage.Format.Format_ARGB32)
    canvas.fill(QtCore.Qt.GlobalColor.transparent)
    painter = QtGui.QPainter(canvas)
    renderer.render(painter)
    painter.end()
    buf = QtCore.QBuffer()
    buf.open(QtCore.QIODevice.OpenModeFlag.WriteOnly)
    canvas.save(buf, "PNG")
    return Image.open(io.BytesIO(bytes(buf.data()))).convert("RGBA")


def wordmark_path() -> str:
    font_id = QtGui.QFontDatabase.addApplicationFont(str(FONT))
    if font_id < 0:
        raise RuntimeError(f"Cannot load bundled wordmark font: {FONT}")
    font = QtGui.QFont(QtGui.QFontDatabase.applicationFontFamilies(font_id)[0])
    font.setPixelSize(112)
    font.setWeight(QtGui.QFont.Weight.Bold)
    font.setLetterSpacing(QtGui.QFont.SpacingType.AbsoluteSpacing, -2)
    path = QtGui.QPainterPath()
    path.addText(0, 0, font, "VRCLT")
    bounds = path.boundingRect()
    # Optically center the actual capitals, independent of font ascenders.
    path.translate(192 - bounds.left(), 80 - bounds.center().y())
    commands = []
    i = 0
    while i < path.elementCount():
        element = path.elementAt(i)
        if element.isMoveTo():
            commands.append(f"M{element.x:.3f} {element.y:.3f}")
        elif element.isLineTo():
            commands.append(f"L{element.x:.3f} {element.y:.3f}")
        else:
            c2, end = path.elementAt(i + 1), path.elementAt(i + 2)
            commands.append(f"C{element.x:.3f} {element.y:.3f} "
                            f"{c2.x:.3f} {c2.y:.3f} {end.x:.3f} {end.y:.3f}")
            i += 2
        i += 1
    return " ".join(commands)


def logo_svg(mark: str, lettering: str, color: str) -> str:
    return (f'<svg xmlns="http://www.w3.org/2000/svg" width="544" height="160" '
            f'viewBox="0 0 544 160" fill="none">\n'
            f'  <title>VRCLT</title>\n'
            f'  <g transform="translate(4 4) scale(.59375)">{mark}</g>\n'
            f'  <path fill="{color}" fill-rule="evenodd" d="{lettering}"/>\n'
            f'</svg>\n')


def make_preview(icon: Image.Image, logos: dict[str, Image.Image]) -> None:
    """A reusable contact sheet showing real export sizes on both surfaces."""
    sheet = Image.new("RGB", (1200, 720), PAPER)
    draw = ImageDraw.Draw(sheet)
    draw.rectangle((600, 0, 1200, 720), fill="#121823")
    label = ImageFont.truetype(str(FONT), 18)
    small = ImageFont.truetype(str(FONT), 14)
    for left, variant, foreground in ((0, "light", INK), (600, "dark", PAPER)):
        draw.text((left + 48, 40), "VRCLT / 1.0", font=label, fill=foreground)
        logo = logos[variant].resize((476, 140), Image.Resampling.LANCZOS)
        sheet.paste(logo, (left + 48, 125), logo)
        draw.line((left + 48, 322, left + 552, 322), fill="#CED4D8" if left == 0 else "#344458")
        large = icon.resize((176, 176), Image.Resampling.LANCZOS)
        sheet.paste(large, (left + 48, 374), large)
        x = left + 280
        for size in (16, 24, 32, 48):
            frame = icon.resize((size, size), Image.Resampling.LANCZOS)
            sheet.paste(frame, (x, 452 - size), frame)
            draw.text((x, 478), str(size), font=small, fill=foreground)
            x += 64
        draw.text((left + 48, 626), "VOICE TRANSLATION + LIVE SUBTITLES", font=small, fill=foreground)
    sheet.save(BRAND / "preview.png", optimize=True)


def main() -> None:
    app = QtGui.QGuiApplication.instance() or QtGui.QGuiApplication([])
    BRAND.mkdir(parents=True, exist_ok=True)
    source = (ASSETS / "icon.svg").read_bytes()
    base = render_svg(source, 1024, 1024)
    base.save(BRAND / "icon-1024.png", optimize=True)
    base.resize((256, 256), Image.Resampling.LANCZOS).save(ASSETS / "icon.png", optimize=True)
    base.save(ASSETS / "icon.ico", format="ICO", sizes=ICO_SIZES)

    # Keep all mark geometry in icon.svg, shared by the icon and both logos.
    ET.register_namespace("", "http://www.w3.org/2000/svg")
    tree = ET.fromstring(source)
    mark = "".join(ET.tostring(child, encoding="unicode") for child in tree
                   if child.tag.rsplit("}", 1)[-1] not in ("title", "desc"))
    lettering = wordmark_path()
    logos = {}
    for variant, color in (("light", INK), ("dark", PAPER)):
        svg = logo_svg(mark, lettering, color)
        (BRAND / f"logo-{variant}.svg").write_text(svg, encoding="utf-8", newline="\n")
        logos[variant] = render_svg(svg.encode(), 1088, 320)
        logos[variant].save(BRAND / f"logo-{variant}.png", optimize=True)
    make_preview(base, logos)
    print("Generated app PNG/ICO, README SVG/PNG logos, 1024px icon and preview.")


if __name__ == "__main__":
    main()
