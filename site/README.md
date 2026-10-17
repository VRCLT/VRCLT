# VRCLT official site

Static, five-language site for `https://vrclt.github.io/`. English, Korean,
Japanese, Simplified Chinese and Traditional Chinese live at `/en/`, `/ko/`,
`/ja/`, `/zh/` and `/zh-hant/`.
No analytics, forms, external fonts or client-side API calls are included.

The root chooses a language from the browser's ordered language preferences,
or the user's last explicit menu choice stored in `vrclt.site.language`.
Explicit language URLs never redirect. Chinese Hant/TW/HK/MO selects Traditional
Chinese; Hans/CN/SG selects Simplified Chinese. Explicit script tags take priority
over region tags. Unsupported languages fall back to English. With JavaScript
disabled, the root remains an English page with normal language links.

All variants share one template and stylesheet. CSS, scripts and font URLs carry
content versions so a cached asset cannot hold a new page on an older design.
Automatic entry and menu links also version the HTML request, then remove only
that version parameter from the visible address, preserving other query data
and anchors. This prevents a cached older language page from reappearing.
The Traditional Chinese page currently uses the app's Simplified Chinese screenshots
and guide, explicitly labeled as such; it does not add a new app UI locale.

The editorial layout uses a cool gray paper surface, blue-gray product plates,
large headlines and actual app screenshots. Noto Sans CJK is self-hosted as
two small WOFF2 subsets under `assets/`, with its OFL license alongside them.
After adding new translated text, refresh those subsets with
`scripts/make_site_fonts.py` (build-time dependency: fonttools and brotli).

Content comes from `content.json`; `config.json` holds the canonical URL.
v1.0 is marked as released. Downloads link to the latest public
release at `https://github.com/VRCLT/VRCLT/releases/latest`.

From the application repository root:

```powershell
.\.venv\Scripts\python.exe scripts\build_site.py
.\.venv\Scripts\python.exe -m http.server 8765 --bind 127.0.0.1 --directory build/site
```

Build output is isolated in `build/site/`. The builder copies only the selected
public brand assets and screenshot images. Publish that output to the public
`vrclt/vrclt.github.io` repository and enable Pages from its `main` branch root.
Keep the application checkout's pending changes and private configuration out
of the website repository.

Before publishing, run `node tests/smoke_site_locale.cjs`, then check every language
on desktop and mobile, automatic entry and remembered choices, the four gallery
tabs (including arrow-key navigation), language links, FAQ disclosures, and
download/documentation targets. The builder validates local files and anchors.

Refresh the app screenshots with `scripts/make_screenshots.py` and brand exports
with `scripts/make_icon.py` when those sources change, then rebuild this site.
