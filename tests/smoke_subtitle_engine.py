"""A separate subtitle engine (inbound.provider): config helpers, the engine
and key each pipeline gets, the runtime key check, the setup check, the
Full-view dashboard pickers and the Settings form. Nothing opens a device, a
session or the network, and the config file is never written."""
import copy
import os
import pathlib
import sys
import threading
import unittest
from unittest.mock import patch

os.environ["QT_QPA_PLATFORM"] = "offscreen"
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent.parent))
sys.path.insert(0, os.path.dirname(__file__))

from PySide6 import QtWidgets

from vrclt import config, pipeline, setup_check
from vrclt.app_controller import AppController
from vrclt.state import AppState

GEMINI_KEY = "AIza" + "x" * 35
SONIOX_KEY = "s" * 40


def cfg_with(provider="gemini", inbound="", gemini="", soniox=""):
    cfg = copy.deepcopy(config.DEFAULTS)
    cfg["provider"] = provider
    cfg["inbound"]["provider"] = inbound
    cfg["api_key"] = gemini
    cfg["soniox"]["api_key"] = soniox
    return cfg


class ConfigTests(unittest.TestCase):
    def test_subtitle_engine_follows_translation_by_default(self):
        cfg = cfg_with("qwen")
        self.assertEqual(config.inbound_provider(cfg), "qwen")
        self.assertEqual(config.providers_in_use(cfg), ("qwen",))
        cfg["inbound"]["provider"] = "nonsense"
        self.assertEqual(config.inbound_provider(cfg), "qwen")

    def test_separate_engines(self):
        cfg = cfg_with("gemini", "soniox")
        self.assertEqual(config.provider_for(cfg, "outbound"), "gemini")
        self.assertEqual(config.provider_for(cfg, "inbound"), "soniox")
        self.assertEqual(config.providers_in_use(cfg), ("gemini", "soniox"))
        self.assertTrue(config.has_speaker_ids(cfg, "inbound"))
        self.assertFalse(config.has_speaker_ids(cfg, "outbound"))
        self.assertTrue(config.uses_source_language(cfg, "inbound"))
        self.assertFalse(config.uses_source_language(cfg))
        cfg["inbound"]["enabled"] = False   # no subtitles: no second key needed
        self.assertEqual(config.providers_in_use(cfg), ("gemini",))

    def test_reset_keeps_the_subtitle_engine(self):
        self.assertIn(("inbound", "provider"), config.RESET_PRESERVE_PATHS)


class PipelineTests(unittest.TestCase):
    def test_each_direction_builds_its_own_engine_with_its_own_key(self):
        made = []

        def fake(kind):
            def build(**kwargs):
                made.append((kind, kwargs["name"], kwargs["api_key"]))
                return object()
            return build

        cfg = cfg_with("gemini", "soniox")
        segmenters = {}
        with patch.object(pipeline, "LiveTranslateSession", fake("gemini")), \
                patch.object(pipeline, "SonioxLiveTranslateSession", fake("soniox")):
            for name, key in (("outbound", "gemini-key"), ("inbound", "soniox-key")):
                p = object.__new__(pipeline._TranslationPipeline)
                pipeline._TranslationPipeline.__init__(
                    p, cfg, key, AppState(), source=None, name=name,
                    get_target_language=lambda: "en", enabled=lambda: True,
                    echo_target_language=False, turn_end_silence_sec=0.5,
                    finalize_silence_sec=1.0)
                segmenters[name] = type(p.segmenter).__name__
                p.detach()
        self.assertEqual(made, [("gemini", "outbound", "gemini-key"),
                                ("soniox", "inbound", "soniox-key")])
        # only the Soniox subtitles carry speaker labels
        self.assertEqual(segmenters, {"outbound": "Segmenter", "inbound": "SpeakerSegmenter"})


class ControllerKeyTests(unittest.TestCase):
    def controller(self, cfg):
        ctl = object.__new__(AppController)
        ctl.cfg = cfg
        ctl.statuses = []
        ctl._set_status = lambda status, detail="": ctl.statuses.append((status, detail))
        return ctl

    def test_both_keys_reach_the_runtime(self):
        ctl = self.controller(cfg_with("gemini", "soniox", GEMINI_KEY, SONIOX_KEY))
        self.assertEqual(ctl._engine_keys(), {"gemini": GEMINI_KEY, "soniox": SONIOX_KEY})

    def test_missing_subtitle_key_stops_the_start(self):
        ctl = self.controller(cfg_with("gemini", "soniox", GEMINI_KEY))
        self.assertIsNone(ctl._engine_keys())
        self.assertEqual(ctl.statuses, [("API key required", "Soniox API key is empty.")])

    def test_unused_subtitle_engine_needs_no_key(self):
        cfg = cfg_with("gemini", "soniox", GEMINI_KEY)
        cfg["inbound"]["enabled"] = False
        self.assertEqual(self.controller(cfg)._engine_keys(), {"gemini": GEMINI_KEY})


class SetupCheckTests(unittest.TestCase):
    def collect(self, cfg):
        with patch("vrclt.vrchat_settings.read", return_value=None), \
                patch("vrclt.audio.devices.default_input_name", return_value=""), \
                patch("vrclt.audio.devices.default_output_name", return_value=""), \
                patch("vrclt.cable_installer.driver_installed", return_value=True):
            return setup_check.collect(cfg, inputs=[""], outputs=[""])

    def test_names_the_engine_without_a_key(self):
        env = self.collect(cfg_with("gemini", "soniox", GEMINI_KEY))
        self.assertFalse(env.api_key_present)
        self.assertEqual(env.missing_key_provider, "soniox")
        env = self.collect(cfg_with("gemini", "soniox", GEMINI_KEY, SONIOX_KEY))
        self.assertTrue(env.api_key_present)
        self.assertEqual(env.missing_key_provider, "")


class QtTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.app = QtWidgets.QApplication.instance() or QtWidgets.QApplication([])

    def window(self, easy=False, **keys):
        from smoke_qt import StubController
        from vrclt.qt_ui import MainWindow
        environment = setup_check.Environment(api_key_present=True)
        ctl = StubController()
        ctl.cfg["ui"]["easy_mode"] = easy
        ctl.cfg["hotkeys"]["enabled"] = False
        ctl.cfg["api_key"] = keys.get("gemini", GEMINI_KEY)
        ctl.cfg["soniox"]["api_key"] = keys.get("soniox", "")
        ctl.restarts = []

        def restart(cfg):
            ctl.restarts.append(cfg)
            ctl.cfg = ctl.raw_cfg = cfg
            return True

        ctl.restart = restart
        with patch.object(MainWindow, "_collect_setup", lambda self: environment):
            win = MainWindow(ctl, pathlib.Path(__file__).parent / "nonexistent.log")
        win._timer.stop()
        win._desktop_overlay._timer.stop()
        win._collect_setup = lambda: environment
        win.show()
        win._refresh()
        self.app.processEvents()
        return ctl, win

    def close(self, win):
        win._quitting = True
        win.close()
        win.deleteLater()
        self.app.processEvents()

    def pick(self, combo, value):
        index = combo.findData(value)
        combo.setCurrentIndex(index)
        combo.activated.emit(index)

    def finish(self, win):
        thread = win._mode_thread
        if isinstance(thread, threading.Thread):
            thread.join(5)
        self.app.processEvents()
        win._refresh()
        self.app.processEvents()

    def test_pickers_only_in_the_full_view(self):
        ctl, win = self.window(easy=True)
        try:
            self.assertTrue(win._in_engine.isHidden() and win._out_engine.isHidden())
            win._view_buttons[False].click()
            self.assertFalse(win._in_engine.isHidden() or win._out_engine.isHidden())
            self.assertEqual(win._in_engine.currentData(), "")      # same as translation
            self.assertEqual(win._out_engine.currentData(), "gemini")
        finally:
            self.close(win)

    def test_engine_without_a_key_is_refused(self):
        ctl, win = self.window()
        try:
            with patch.object(config, "save") as save:
                self.pick(win._in_engine, "soniox")
            save.assert_not_called()
            self.assertEqual(ctl.restarts, [])
            self.assertIn("Soniox", win._dashboard_note.text())
            self.assertEqual(win._in_engine.currentData(), "")     # rolled back
        finally:
            self.close(win)

    def test_separate_subtitle_engine_applies(self):
        ctl, win = self.window(soniox=SONIOX_KEY)
        try:
            with patch.object(config, "save") as save:
                self.pick(win._in_engine, "soniox")
                self.finish(win)
            saved = save.call_args.args[0]
            self.assertEqual((saved["provider"], saved["inbound"]["provider"]),
                             ("gemini", "soniox"))
            self.assertEqual(len(ctl.restarts), 1)
            self.assertEqual(win._dashboard_note.text(), win._tr("msg_engine_applied"))
            self.assertEqual(win._engine_button.text(), "Gemini Live / Soniox")
            self.assertFalse(win._keep_speaker_context.isHidden())   # Soniox options
            self.assertFalse(win._speaker_badge.isHidden())          # Soniox labels speakers
            self.assertEqual(win._in_engine.currentData(), "soniox")
        finally:
            self.close(win)

    def test_settings_form_shows_both_engines(self):
        ctl, win = self.window(soniox=SONIOX_KEY)
        try:
            form = win._settings_form
            row_visible = lambda path: not form._fields[path][0].isHidden()
            win._tabs.setCurrentIndex(win._tab_settings_idx)
            self.app.processEvents()
            self.assertTrue(row_visible("api_key"))
            self.assertFalse(row_visible("soniox.api_key"))
            combo = form._fields["inbound.provider"][0]
            combo.setCurrentIndex(combo.findData("soniox"))
            self.app.processEvents()
            self.assertTrue(row_visible("api_key") and row_visible("soniox.api_key"))
            self.assertIn("Gemini + Soniox", form._section_hint.text())
            self.assertEqual(form.config_from_fields()["inbound"]["provider"], "soniox")
            combo.setCurrentIndex(0)
            self.app.processEvents()
            self.assertFalse(row_visible("soniox.api_key"))
            self.assertEqual(form.config_from_fields()["inbound"]["provider"], "")
        finally:
            self.close(win)


if __name__ == "__main__":
    unittest.main(verbosity=1)
