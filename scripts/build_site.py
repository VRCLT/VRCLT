"""Build the five-language GitHub Pages site into build/site.

Run from any directory: .venv/Scripts/python scripts/build_site.py
Only allowlisted public assets are copied. No app settings, logs or executables
are included. Uses Pillow, already a dependency of the application.
"""
from __future__ import annotations

import html
import hashlib
import json
import re
import shutil
from pathlib import Path
from html.parser import HTMLParser
from urllib.parse import unquote, urlsplit

from PIL import Image, ImageDraw, ImageFont

ROOT = Path(__file__).resolve().parent.parent
SOURCE = ROOT / "site"
OUTPUT = ROOT / "build" / "site"
LANGUAGES = {"en": "English", "ko": "한국어", "ja": "日本語", "zh": "简体中文", "zh-hant": "繁體中文"}
SCREENSHOT_LOCALES = ("en", "ko", "ja", "zh")


def social_image(assets: Path) -> None:
    canvas = Image.new("RGB", (1200, 630), "#192C3D")
    logo = Image.open(ROOT / "docs" / "brand" / "logo-dark.png").convert("RGBA")
    logo.thumbnail((748, 220), Image.Resampling.LANCZOS)
    canvas.paste(logo, (80, 100), logo)
    draw = ImageDraw.Draw(canvas)
    font = ROOT / "vrclt" / "assets" / "fonts" / "PretendardJP-Regular.otf"
    draw.text((92, 370), "Real-time voice translation and subtitles",
              font=ImageFont.truetype(str(font), 45), fill="#F5F3ED")
    draw.text((94, 465), "VRChat  /  Discord  /  Windows",
              font=ImageFont.truetype(str(font), 23), fill="#8CC8F2")
    canvas.save(assets / "social.png", optimize=True)
    icon = Image.open(ROOT / "docs" / "brand" / "icon-1024.png").convert("RGBA")
    touch = Image.new("RGB", (180, 180), "#192C3D")
    icon = icon.resize((180, 180), Image.Resampling.LANCZOS)
    touch.paste(icon, (0, 0), icon)
    touch.save(assets / "apple-touch-icon.png", optimize=True)


class LinkCollector(HTMLParser):
    def __init__(self):
        super().__init__()
        self.links = []
        self.ids = set()

    def handle_starttag(self, tag, attrs):
        attrs = dict(attrs)
        if "id" in attrs:
            assert attrs["id"] not in self.ids, f"Duplicate id: {attrs['id']}"
            self.ids.add(attrs["id"])
        if tag in ("a", "link", "img", "script"):
            value = attrs.get("href", attrs.get("src"))
            if value:
                self.links.append(value)


def validate() -> None:
    for page in OUTPUT.rglob("*.html"):
        content = page.read_text(encoding="utf-8")
        assert "{{" not in content, f"Unresolved template: {page}"
        parser = LinkCollector()
        parser.feed(content)
        for value in parser.links:
            url = urlsplit(value)
            if url.scheme or url.netloc:
                continue
            if value.startswith("#"):
                assert url.fragment in parser.ids, (page, value)
                continue
            destination = (page.parent / unquote(url.path)).resolve()
            assert destination.is_relative_to(OUTPUT.resolve()), (page, value)
            if destination.is_dir():
                destination /= "index.html"
            assert destination.is_file(), (page, value)
    for asset in (OUTPUT / "assets").glob("*.png"):
        with Image.open(asset) as image:
            image.verify()


def main() -> None:
    config = json.loads((SOURCE / "config.json").read_text(encoding="utf-8"))
    site_url = config["url"].rstrip("/") + "/"
    content = json.loads((SOURCE / "content.json").read_text(encoding="utf-8"))
    template = (SOURCE / "index.html").read_text(encoding="utf-8")
    site_version = hashlib.sha256(b"".join(
        (SOURCE / name).read_bytes()
        for name in ("index.html", "content.json", "styles.css", "app.js", "locale.js")
    )).hexdigest()[:12]
    assets = OUTPUT / "assets"
    assets.mkdir(parents=True, exist_ok=True)
    for name in ("styles.css", "app.js", "locale.js"):
        shutil.copyfile(SOURCE / name, OUTPUT / name)
    shutil.copyfile(ROOT / "LICENSE", OUTPUT / "LICENSE")
    (OUTPUT / "README.md").write_text(
        "# VRCLT official website\n\n"
        "[vrclt.github.io](https://vrclt.github.io/)\n\n"
        "Static pages in English, Korean, Japanese, Simplified Chinese and Traditional Chinese.\n"
        "The root detects the browser language; explicit language choices are remembered locally.\n"
        "v1.0 is released; downloads link to the latest published release.\n\n"
        "[Application source and downloads](https://github.com/shgeum/VRCLT)\n\n"
        "The website templates live in the application checkout's `site/` directory,\n"
        "built with `scripts/build_site.py`. This repository contains the public\n"
        "build output only. GitHub Pages serves the root of the `main` branch.\n",
        encoding="utf-8", newline="\n")
    for name in ("logo-light.svg", "logo-dark.svg"):
        shutil.copyfile(ROOT / "docs" / "brand" / name, assets / name)
    for name in ("icon.svg", "icon.ico"):
        shutil.copyfile(ROOT / "vrclt" / "assets" / name, assets / name)
    for name in ("vrclt-sans-regular.woff2", "vrclt-sans-bold.woff2", "LICENSE-NotoSansCJK.txt"):
        shutil.copyfile(SOURCE / "assets" / name, assets / name)
    css = (SOURCE / "styles.css").read_text(encoding="utf-8")
    font_versions = {}
    for weight in ("regular", "bold"):
        name = f"vrclt-sans-{weight}.woff2"
        font_versions[weight] = hashlib.sha256((assets / name).read_bytes()).hexdigest()[:12]
        css = css.replace(name, f"{name}?v={font_versions[weight]}")
    (OUTPUT / "styles.css").write_text(css, encoding="utf-8", newline="\n")
    for locale in SCREENSHOT_LOCALES:
        for kind in ("dashboard", "vr-subtitles", "vr-wrist", "vr-dashboard"):
            name = f"{kind}-{locale}.png"
            shutil.copyfile(ROOT / "docs" / "images" / name, assets / name)
    social_image(assets)

    urls = []
    pages = [(locale, f"{locale}/") for locale in LANGUAGES] + [("en", "")]
    for locale, route in pages:
        base = "../" if route else "./"
        urls.append(site_url + route)
        values = {key: html.escape(value, quote=True) for key, value in content[locale].items()}
        image_locale = "zh" if locale == "zh-hant" else locale
        values.update(base=base, home="./", imageLocale=image_locale,
                      siteVersion=site_version,
                      autoLanguage="" if route else " data-auto-language",
                      fontVersion=font_versions["bold"],
                      cssVersion=hashlib.sha256((OUTPUT / "styles.css").read_bytes()).hexdigest()[:12],
                      jsVersion=hashlib.sha256((OUTPUT / "app.js").read_bytes()).hexdigest()[:12],
                      localeVersion=hashlib.sha256((OUTPUT / "locale.js").read_bytes()).hexdigest()[:12],
                      siteUrl=html.escape(site_url, quote=True), canonical=html.escape(site_url + f"{locale}/", quote=True),
                      docsUrl="https://github.com/shgeum/VRCLT/blob/main/" +
                      ("README.md" if locale == "en" else f"README.{image_locale}.md"))
        values["languageLinks"] = "".join(
            f'<a href="{base}{code}/?v={site_version}" data-language="{code}" lang="{content[code]["lang"]}"'
            f' hreflang="{content[code]["lang"]}"' + (' aria-current="page"' if code == locale else '') +
            f'>{label}</a>' for code, label in LANGUAGES.items())
        result = re.sub(r"\{\{(\w+)\}\}", lambda match: values[match[1]], template)
        destination = OUTPUT / route
        destination.mkdir(parents=True, exist_ok=True)
        (destination / "index.html").write_text(result, encoding="utf-8", newline="\n")
    (OUTPUT / ".nojekyll").touch()
    (OUTPUT / "robots.txt").write_text(f"User-agent: *\nAllow: /\nSitemap: {site_url}sitemap.xml\n", encoding="utf-8")
    (OUTPUT / "sitemap.xml").write_text(
        '<?xml version="1.0" encoding="UTF-8"?>\n<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">\n' +
        "".join(f"  <url><loc>{html.escape(url)}</loc></url>\n" for url in urls) + "</urlset>\n", encoding="utf-8")
    validate()
    print(f"Built and validated {len(LANGUAGES)} languages and an automatic language entry at {OUTPUT}")
    print(f"Canonical URL: {site_url}")


if __name__ == "__main__":
    main()
