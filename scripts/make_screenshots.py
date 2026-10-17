"""Generate the README and wiki screenshots in docs/images/.

Renders the real UI with sample data: the desktop Dashboard (Simple view), the
setup check and Settings with each engine selected (the wiki's API key guide)
for every README language, plus the VR panels (subtitle overlay, wrist menu,
SteamVR Dashboard). Nothing appears on screen, no tray
icon is created, and no audio device, network, hotkey or VRChat setting is
touched.

Run: .venv/Scripts/python scripts/make_screenshots.py
"""
import copy
import os
import sys
import tempfile
from pathlib import Path
from unittest.mock import patch

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "docs" / "images"
# a throwaway config path before vrclt.config is imported: never the user's
os.environ["VRCLT_CONFIG"] = str(Path(tempfile.mkdtemp(prefix="vrclt-shots-")) / "config.yaml")
sys.path.insert(0, str(ROOT))

from PySide6 import QtCore, QtWidgets  # noqa: E402

from vrclt import config as config_mod, setup_check  # noqa: E402
from vrclt.state import AppState  # noqa: E402
from vrclt.subtitles import SubtitleLine, SubtitleStore  # noqa: E402
from vrclt.vrchat_settings import VRChatSettings  # noqa: E402

REALTEK = "Microphone (Realtek(R) Audio)"
HEADSET = "Headset Microphone (Oculus Virtual Audio Device)"
CABLE_OUT = "CABLE Output (VB-Audio Virtual Cable)"
CABLE_IN = "CABLE Input (VB-Audio Virtual Cable)"
SPEAKERS = "Speakers (Realtek(R) Audio)"
INPUTS = ["", REALTEK, HEADSET, CABLE_OUT]

# per README language: (my output language, my subtitle language, subtitles)
SAMPLES = {
    "en": ("ja", "en", [
        ("こんにちは！どこから来たの？", "Hi! Where are you from?", "1"),
        ("Je viens de France !", "I'm from France!", "2"),
        ("このワールド、すごくきれいだね。", "This world is really beautiful.", "1"),
    ], ("みんなで写真撮ろうよ", "Let's all take a photo", "2")),
    "ko": ("ja", "ko", [
        ("こんにちは！どこから来たの？", "안녕하세요! 어디에서 오셨어요?", "1"),
        ("Je viens de France !", "저는 프랑스에서 왔어요!", "2"),
        ("このワールド、すごくきれいだね。", "이 월드 정말 예쁘다.", "1"),
    ], ("みんなで写真撮ろうよ", "다 같이 사진 찍자", "2")),
    "ja": ("en", "ja", [
        ("Hi! Where are you from?", "こんにちは！どこから来たの？", "1"),
        ("Je viens de France !", "フランスから来たよ！", "2"),
        ("This world is really beautiful.", "このワールド、すごくきれいだね。", "1"),
    ], ("Let's all take a photo", "みんなで写真撮ろう", "2")),
    "zh": ("ja", "zh-Hans", [
        ("こんにちは！どこから来たの？", "你好！你是从哪里来的？", "1"),
        ("Je viens de France !", "我来自法国！", "2"),
        ("このワールド、すごくきれいだね。", "这个世界真漂亮。", "1"),
    ], ("みんなで写真撮ろうよ", "大家一起拍张照吧", "2")),
}


class Controller:
    """Just enough of AppController for MainWindow, with sample state."""

    def __init__(self, lang: str):
        out_lang, sub_lang, finals, partial = SAMPLES[lang]
        self.cfg = copy.deepcopy(config_mod.DEFAULTS)
        self.cfg["provider"] = "soniox"
        self.cfg["soniox"]["api_key"] = "sample"
        self.cfg["hotkeys"]["enabled"] = False
        self.cfg["ui"]["easy_mode"] = True
        self.cfg["control"]["languages"] = list(dict.fromkeys([out_lang, "en", "ko", "ja"]))
        self.cfg["inbound"]["languages"] = list(dict.fromkeys([sub_lang, "en", "ko", "ja"]))
        self.raw_cfg = self.cfg
        self.config_revision = 0
        self.state = AppState()
        self.state.ui_lang = lang
        self.state.target_language = out_lang
        self.state.inbound_language = sub_lang
        self._lines = ([SubtitleLine(s, d, "auto", sp) for s, d, sp in finals],
                       SubtitleLine(partial[0], partial[1], "auto", partial[2]))

    def subscribe(self, fn): pass
    def subscribe_errors(self, fn): pass
    def get_status_info(self): return (True, "status_running", "")
    def get_provider(self): return config_mod.provider(self.cfg)
    def subtitles_snapshot_with_speakers(self): return self._lines
    def subtitles_snapshot(self): return ([], ("", ""))
    def tts_gain(self): return 1.0
    def mic_level(self): return (900.0, 90.0)
    def close_action(self): return "tray"
    def last_config_version(self):
        from vrclt import __version__
        return __version__
    def mark_config_version_seen(self): pass
    def get_steamvr_auto_launch(self): return None
    def set_steamvr_auto_launch(self, enabled): pass
    def set_ui_lang(self, lang, persist=True): self.state.ui_lang = lang
    def set_hold_mute(self, held): pass
    def set_easy_mode(self, enabled): self.cfg["ui"]["easy_mode"] = enabled
    def osc_status(self): return (True, 1.0)
    def shutdown(self): pass


def ready_env():
    return setup_check.Environment(
        api_key_present=True, inputs=INPUTS, outputs=["", SPEAKERS, CABLE_IN],
        default_input=REALTEK, default_output=SPEAKERS,
        vrchat=VRChatSettings(CABLE_OUT, CABLE_OUT, True), osc_bound=True, osc_last_rx=1.0)


def fresh_env():
    return setup_check.Environment(
        api_key_present=True, inputs=INPUTS, outputs=["", SPEAKERS, CABLE_IN],
        default_input=CABLE_OUT, default_output=SPEAKERS,
        vrchat=VRChatSettings(HEADSET, "", False), osc_bound=True)


def settle(app, win):
    for _ in range(4):
        win._setup_checked_at = 0.0
        win._refresh()
        win._tick_mic_meter()
        app.processEvents()


def desktop_shots(app, lang: str) -> None:
    from vrclt.qt_ui import MainWindow, _install_app_font
    from vrclt.ui.update_banner import UpdateBanner
    _install_app_font(app, lang)
    ctl = Controller(lang)
    with patch.object(UpdateBanner, "start_check", return_value=False):
        win = MainWindow(ctl, Path(tempfile.gettempdir()) / "vrclt-shots.log")
    win._timer.stop()
    win._desktop_overlay._timer.stop()
    win.setAttribute(QtCore.Qt.WidgetAttribute.WA_DontShowOnScreen, True)
    try:
        win._collect_setup = ready_env
        win.resize(1000, 780)   # include the mic picker and the full subtitle sample
        win.show()
        settle(app, win)
        win.grab().save(str(OUT / f"dashboard-{lang}.png"))
        ctl.cfg["outbound"]["tts_device"] = ""
        win._collect_setup = fresh_env
        win.resize(1000, 1100)   # room for the expanded checklist
        settle(app, win)
        win._setup_panel.grab().save(str(OUT / f"setup-check-{lang}.png"))
    finally:
        win._quitting = True
        win.close()


# fake values: the wiki shows where each engine's key goes, never a real key
KEY_SAMPLES = {
    "gemini": {"api_key": "AIza-sample"},
    "qwen": {"qwen.api_key": "sk-sample", "qwen.workspace_id": "ws-xxxxxxxx"},
    "openai": {"openai.api_key": "sk-sample"},
    "soniox": {"soniox.api_key": "sample"},
}


def settings_shots(app, lang: str) -> None:
    """Settings -> Translation with each engine selected, for the wiki's API
    key guide."""
    from vrclt.qt_ui import MainWindow, _install_app_font
    from vrclt.ui.update_banner import UpdateBanner
    _install_app_font(app, lang)
    ctl = Controller(lang)
    with patch.object(UpdateBanner, "start_check", return_value=False):
        win = MainWindow(ctl, Path(tempfile.gettempdir()) / "vrclt-shots.log")
    win._timer.stop()
    win._desktop_overlay._timer.stop()
    win.setAttribute(QtCore.Qt.WidgetAttribute.WA_DontShowOnScreen, True)
    try:
        win._collect_setup = ready_env
        win.resize(1000, 1200)   # the longest engine section fits unscrolled
        win._tabs.setCurrentIndex(win._tab_settings_idx)
        win.show()
        form = win._settings_form
        group = next(g for g, paths in form._groups
                     if paths and form._group_keys[paths[0]] == "grp_api")
        for provider, values in KEY_SAMPLES.items():
            form._fields["provider"][0].setCurrentText(provider)
            for path, value in values.items():
                form._fields[path][0].setText(value)
            settle(app, win)
            bottom = group.mapTo(win, QtCore.QPoint(0, group.height())).y()
            win.grab(QtCore.QRect(0, 0, win.width(), bottom + 16)).save(
                str(OUT / f"settings-{provider}-{lang}.png"))
    finally:
        win._quitting = True
        win.close()


def vr_shots(lang: str) -> None:
    from vrclt.vr.dashboard_panel import DashboardPanel
    from vrclt.vr.subtitle_overlay import SubtitlePanel
    from vrclt.vr.wrist_ui import WristPanel
    out_lang, sub_lang, finals, partial = SAMPLES[lang]
    state = AppState()
    state.ui_lang = lang
    state.target_language = out_lang
    state.inbound_language = sub_lang
    running = (True, "status_running", "")
    store = SubtitleStore()
    panel = SubtitlePanel(store, state, height_m=0.3)
    lines = [SubtitleLine(s, d, "auto", sp) for s, d, sp in finals]
    image = panel._render(lines, SubtitleLine(partial[0], partial[1], "auto", partial[2]))
    image.crop(image.getbbox()).save(OUT / f"vr-subtitles-{lang}.png")
    languages = list(dict.fromkeys([out_lang, "en", "ko", "ja"]))
    wrist = WristPanel(state, languages, inbound_languages=[sub_lang, "en"],
                       get_provider=lambda: "soniox")
    wrist._render(running, False).save(OUT / f"vr-wrist-{lang}.png")
    dash = DashboardPanel(state, languages, inbound_languages=[sub_lang, "en"],
                          get_provider=lambda: "soniox")
    dash._render(running).save(OUT / f"vr-dashboard-{lang}.png")


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    app = QtWidgets.QApplication([])
    QtWidgets.QSystemTrayIcon.show = lambda self: None   # no tray icon flash
    for lang in SAMPLES:
        desktop_shots(app, lang)
        settings_shots(app, lang)
        vr_shots(lang)
        print("rendered", lang)
    print("saved to", OUT)


if __name__ == "__main__":
    main()
