"""Setup checklist, auto setup, mic finder and easy/full dashboard regressions.

No audio devices, VRChat install, registry writes or network are needed.
Run with .venv/Scripts/python tests/smoke_setup_check.py.
"""
import copy
import os
import pathlib
import socket
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

os.environ["QT_QPA_PLATFORM"] = "offscreen"
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent.parent))
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

import io
import zipfile
from unittest import mock

import yaml
from PySide6 import QtWidgets

from vrclt import cable_installer, config, i18n, setup_check, vrchat_settings
from vrclt.audio import mic_probe
from vrclt.audio.mic_probe import MicProbe, pick_speaking_device
from vrclt.control.osc_listener import OscControl
from vrclt.state import AppState
from vrclt.vrchat_settings import VRChatSettings

REALTEK = "Microphone (Realtek(R) Audio)"
QUEST = "Headset Microphone (Oculus Virtual Audio Device)"
CABLE_OUT = "CABLE Output (VB-Audio Virtual Cable)"
CABLE_IN = "CABLE Input(VB-Audio Virtual Cable)"   # PortAudio drops the space
SPEAKERS = "Speakers (Realtek(R) Audio)"
INPUTS = ["", REALTEK, QUEST, CABLE_OUT]


def env(**overrides):
    values = dict(api_key_present=True, inputs=INPUTS, outputs=["", SPEAKERS, CABLE_IN],
                  default_input=REALTEK, default_output=SPEAKERS,
                  vrchat=VRChatSettings(CABLE_OUT, CABLE_OUT, True), osc_bound=True)
    values.update(overrides)
    return setup_check.Environment(**values)


def cfg(**outbound):
    out = copy.deepcopy(config.DEFAULTS)
    out["outbound"].update(outbound)
    return out


def by_key(items):
    return {item.key: item for item in items}


class VRChatSettingsTests(unittest.TestCase):
    def test_parse_playerprefs_values(self):
        parsed = vrchat_settings.parse({
            "VRC_INPUT_MIC_DEVICE_NAME_VR_h2729556311": CABLE_OUT.encode() + b"\0",
            "VRC_INPUT_MIC_DEVICE_NAME_Desktop_h2596021377": b"\0",
            "VRC_INPUT_OSC_h1104161515": 1,
            "VRC_INPUT_OSC_SOMETHING_ELSE_h1": 0,
        })
        self.assertEqual(parsed, VRChatSettings(CABLE_OUT, "", True))
        self.assertEqual(vrchat_settings.parse({}), VRChatSettings(None, None, None))
        self.assertIs(vrchat_settings.parse({"VRC_INPUT_OSC_h1": 0}).osc_enabled, False)

    def test_value_names_and_device_names(self):
        self.assertEqual(vrchat_settings.unity_hash("VRC_INPUT_OSC"), 1104161515)
        self.assertEqual(vrchat_settings.unity_hash("VRC_INPUT_MIC_DEVICE_NAME_VR"), 2729556311)
        self.assertEqual(vrchat_settings.unity_hash("VRC_INPUT_MIC_DEVICE_NAME_Desktop"),
                         2596021377)
        self.assertEqual(vrchat_settings.vrchat_mic_name("CABLE Output(VB-Audio Virtual Cable)"),
                         CABLE_OUT)
        self.assertEqual(vrchat_settings.vrchat_mic_name(CABLE_OUT), CABLE_OUT)

    def test_write_reuses_existing_names_and_creates_missing_ones(self):
        reg = FakeWinreg({"VRC_INPUT_MIC_DEVICE_NAME_VR_h2729556311": QUEST.encode() + b"\0",
                          "VRC_INPUT_OSC_h1104161515": 0})
        previous = vrchat_settings.write(mic=CABLE_OUT, osc=True, _winreg=reg)
        self.assertEqual(reg.values["VRC_INPUT_MIC_DEVICE_NAME_VR_h2729556311"],
                         CABLE_OUT.encode() + b"\0")
        self.assertEqual(reg.values["VRC_INPUT_MIC_DEVICE_NAME_Desktop_h2596021377"],
                         CABLE_OUT.encode() + b"\0")
        self.assertEqual(reg.values["VRC_INPUT_OSC_h1104161515"], 1)
        self.assertEqual(previous["VRC_INPUT_MIC_DEVICE_NAME_VR_h2729556311"], QUEST)
        self.assertEqual(vrchat_settings.parse(reg.values), VRChatSettings(CABLE_OUT, CABLE_OUT, True))
        vrchat_settings.restore(previous, _winreg=reg)
        self.assertEqual(vrchat_settings.parse(reg.values), VRChatSettings(QUEST, None, False))
        self.assertNotIn("VRC_INPUT_MIC_DEVICE_NAME_Desktop_h2596021377", reg.values)


class FakeWinreg:
    HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE, REG_BINARY, REG_DWORD = 1, 2, 4, 3, 4

    def __init__(self, values):
        self.values = dict(values)

    def OpenKey(self, *_args):
        return self

    def __enter__(self):
        return self

    def __exit__(self, *_exc):
        return False

    def EnumValue(self, _key, index):
        items = list(self.values.items())
        if index >= len(items):
            raise OSError("no more values")
        name, data = items[index]
        return name, data, 0

    def SetValueEx(self, _key, name, _reserved, _kind, data):
        self.values[name] = data

    def DeleteValue(self, _key, name):
        del self.values[name]


class SetupRuleTests(unittest.TestCase):
    def test_device_names(self):
        self.assertTrue(setup_check.is_virtual_device(CABLE_OUT))
        self.assertTrue(setup_check.is_virtual_device("VoiceMeeter Output (VB-Audio VoiceMeeter VAIO)"))
        self.assertFalse(setup_check.is_virtual_device(QUEST))   # the Quest's real mic
        self.assertFalse(setup_check.is_virtual_device("마이크(Virtual Desktop Audio)"))
        self.assertEqual(setup_check.app_mic_name(CABLE_IN), "CABLE Output")
        self.assertEqual(setup_check.app_mic_name("CABLE-A Input (VB-Audio Cable A)"),
                         "CABLE-A Output")
        self.assertEqual(setup_check.app_mic_name("BlackHole 2ch"), "BlackHole 2ch")
        self.assertEqual(setup_check.find_device(INPUTS, "realtek"), REALTEK)

    def test_mic_pickers_hide_the_cables_recording_side(self):
        meeter = "VoiceMeeter Output (VB-Audio VoiceMeeter VAIO)"
        names = ["", REALTEK, CABLE_OUT, "CABLE Output(VB-Audio Virtual Cable)", meeter]
        self.assertEqual(setup_check.mic_choices(names, cfg()), ["", REALTEK, meeter])
        # a real speaker as voice output must not hide inputs that share a word
        self.assertEqual(setup_check.mic_choices(["", "Speakers Mic Array"],
                                                 cfg(tts_device="Speakers")),
                         ["", "Speakers Mic Array"])
        a_cable = cfg(tts_device="CABLE-A Input (VB-Audio Cable A)")
        self.assertEqual(setup_check.mic_choices(["", REALTEK, "CABLE-A Output (VB-Audio Cable A)"],
                                                 a_cable), ["", REALTEK])

    def test_everything_ready(self):
        items = setup_check.run_checks(cfg(), env())
        self.assertEqual([i.key for i in items],
                         ["api_key", "cable", "voice_out", "mic", "vrchat_mic", "vrchat_osc"])
        self.assertEqual(setup_check.worst(items), setup_check.OK)
        self.assertEqual(by_key(items)["mic"].state, "default")
        new, changed = setup_check.auto_fix(cfg(), env())
        self.assertEqual(changed, [])

    def test_fresh_install_problems(self):
        fresh = env(api_key_present=False, outputs=["", SPEAKERS], default_input=CABLE_OUT,
                    vrchat=VRChatSettings(QUEST, "", False))
        items = by_key(setup_check.run_checks(cfg(), fresh))
        self.assertEqual(items["api_key"].status, setup_check.ERROR)
        self.assertEqual(items["cable"].state, "missing")
        self.assertEqual(items["voice_out"].state, "needs_cable")
        self.assertFalse(items["voice_out"].fixable)
        self.assertEqual(items["mic"].state, "virtual")      # default mic loops back
        self.assertTrue(items["mic"].fixable)
        self.assertEqual(items["vrchat_mic"].state, "wrong")
        # desktop uses VRChat's "Default", which is the cable here: only VR is wrong
        self.assertEqual([m[0] for m in items["vrchat_mic"].args["modes"]], ["vr"])
        self.assertEqual(items["vrchat_osc"].state, "off")
        new, changed = setup_check.auto_fix(cfg(), fresh)
        self.assertEqual(changed, ["outbound.mic_device"])
        self.assertEqual(new["outbound"]["mic_device"], QUEST)   # what VRChat used

    def test_auto_fix_routes_voice_to_the_cable(self):
        speakers = cfg(tts_device="")
        items = by_key(setup_check.run_checks(speakers, env()))
        self.assertEqual(items["voice_out"].state, "not_cable")
        self.assertTrue(items["voice_out"].fixable)
        new, changed = setup_check.auto_fix(speakers, env())
        self.assertEqual(changed, ["outbound.tts_device"])
        self.assertEqual(new["outbound"]["tts_device"], "CABLE Input")
        self.assertEqual(speakers["outbound"]["tts_device"], "")   # input untouched
        gone = cfg(tts_device="Old Headset")
        self.assertEqual(by_key(setup_check.run_checks(gone, env()))["voice_out"].state,
                         "missing")

    def test_mic_suggestion_order(self):
        self.assertEqual(setup_check.suggest_mic(env(vrchat=VRChatSettings(QUEST, None, True))),
                         (QUEST, "vrchat"))
        self.assertEqual(setup_check.suggest_mic(env()), (REALTEK, "default"))
        self.assertEqual(setup_check.suggest_mic(env(default_input=CABLE_OUT, vrchat=None)),
                         (REALTEK, "first"))
        self.assertEqual(setup_check.suggest_mic(env(inputs=["", CABLE_OUT], vrchat=None,
                                                     default_input=CABLE_OUT)), ("", ""))
        # the default works but is not the mic VRChat used: pin the VRChat one
        hinted = env(vrchat=VRChatSettings(QUEST, None, True))
        self.assertEqual(setup_check.auto_fix(cfg(), hinted)[0]["outbound"]["mic_device"], QUEST)
        # an explicit choice is never overridden while it works
        self.assertEqual(setup_check.auto_fix(cfg(mic_device=REALTEK), hinted)[1], [])
        missing = by_key(setup_check.run_checks(cfg(mic_device="Unplugged"), env()))["mic"]
        self.assertEqual((missing.state, missing.fixable), ("missing", True))

    def test_mode_specific_items(self):
        discord = cfg()
        discord["app"]["mode"] = "discord"
        keys = [i.key for i in setup_check.run_checks(discord, env(app_mode="discord"))]
        self.assertNotIn("vrchat_mic", keys)
        self.assertNotIn("vrchat_osc", keys)
        off = cfg(enabled=False)
        keys = [i.key for i in setup_check.run_checks(off, env())]
        self.assertNotIn("mic", keys)
        self.assertNotIn("voice_out", keys)
        osc = by_key(setup_check.run_checks(cfg(), env(osc_bound=False)))["vrchat_osc"]
        self.assertEqual(osc.state, "port_busy")
        osc = by_key(setup_check.run_checks(cfg(), env(osc_last_rx=time.time())))["vrchat_osc"]
        self.assertEqual(osc.state, "receiving")
        unknown = by_key(setup_check.run_checks(cfg(), env(vrchat=None)))
        self.assertEqual(unknown["vrchat_mic"].state, "unknown")
        self.assertEqual(unknown["vrchat_osc"].state, "unknown")
        default = env(vrchat=VRChatSettings("", "", True), default_input=CABLE_OUT)
        self.assertEqual(by_key(setup_check.run_checks(cfg(mic_device=REALTEK), default))
                         ["vrchat_mic"].state, "ok")   # VRChat "Default" = the cable

    def test_vrchat_settings_are_fixable_while_installed(self):
        wrong = env(vrchat=VRChatSettings(QUEST, "", False))
        items = by_key(setup_check.run_checks(cfg(), wrong))
        self.assertTrue(items["vrchat_mic"].fixable)
        self.assertTrue(items["vrchat_osc"].fixable)
        self.assertEqual(setup_check.vrchat_changes(cfg(), wrong),
                         {"mic": CABLE_OUT, "osc": True})
        self.assertEqual(setup_check.vrchat_changes(cfg(), env()), {})
        # never ran VRChat: nothing to write into
        self.assertEqual(setup_check.vrchat_changes(cfg(), env(vrchat=None)), {})
        # no cable yet: the mic cannot point at it
        no_cable = env(outputs=["", SPEAKERS], vrchat=VRChatSettings(QUEST, "", True))
        self.assertEqual(setup_check.vrchat_changes(cfg(), no_cable), {})

    def test_cable_install_can_steal_the_default_output(self):
        stolen = by_key(setup_check.run_checks(cfg(), env(default_output=CABLE_IN)))
        self.assertEqual(stolen["system_sound"].status, setup_check.WARN)
        self.assertNotIn("system_sound", by_key(setup_check.run_checks(cfg(), env())))

    def test_every_item_has_text(self):
        keys = set()
        for item in (setup_check.run_checks(cfg(), env())
                     + setup_check.run_checks(cfg(tts_device=""), env(
                         api_key_present=False, outputs=["", SPEAKERS],
                         default_input=CABLE_OUT, vrchat=VRChatSettings(QUEST, "", False)))):
            keys.update({f"setup_{item.key}", f"setup_{item.key}_{item.state}"})
        for state in ("missing", "not_cable", "other_virtual"):
            keys.add(f"setup_voice_out_{state}")
        for state in ("ok", "missing", "none"):
            keys.add(f"setup_mic_{state}")
        for state in ("port_busy", "receiving", "enabled", "unknown"):
            keys.add(f"setup_vrchat_osc_{state}")
        keys.add("setup_vrchat_mic_unknown")
        keys.update({"setup_system_sound", "setup_system_sound_output"})
        for key in keys:
            self.assertTrue(i18n.has(key), key)


class MicProbeTests(unittest.TestCase):
    def test_speech_must_rise_above_each_devices_floor(self):
        peaks = {"headset": 2400.0, "webcam": 500.0, "hiss": 700.0}
        floors = {"headset": 40.0, "webcam": 30.0, "hiss": 650.0}
        self.assertEqual(pick_speaking_device(peaks, floors), "headset")
        self.assertEqual(pick_speaking_device({"hiss": 700.0}, {"hiss": 650.0}), "")
        self.assertEqual(pick_speaking_device({"quiet": 120.0}, {"quiet": 0.0}), "")

    def test_open_click_and_single_spikes_are_not_speech(self):
        import numpy as np
        probe = MicProbe(["mic"])
        clock = [100.0]
        block = lambda value: np.full((800, 1), value, dtype=np.int16)
        with patch.object(mic_probe.time, "monotonic", lambda: clock[0]):
            callback = probe._callback("mic")
            callback(block(7000), 800, None, None)       # stream-open click
            clock[0] += 0.6
            for value in (20, 20, 6000, 20, 20):         # one-block spike
                callback(block(value), 800, None, None)
            self.assertEqual(probe.best(), "")
            for value in (1500, 1600, 1400, 1500):       # sustained speech
                callback(block(value), 800, None, None)
            self.assertEqual(probe.best(), "mic")


class CableInstallerTests(unittest.TestCase):
    @staticmethod
    def _zip(files):
        buffer = io.BytesIO()
        with zipfile.ZipFile(buffer, "w") as zf:
            for name, data in files.items():
                zf.writestr(name, data)
        return buffer.getvalue()

    @staticmethod
    def _opener(data):
        class Response(io.BytesIO):
            def __enter__(self):
                return self

            def __exit__(self, *_exc):
                return False
        return lambda request, timeout, context: Response(data)

    def test_download_unpacks_the_official_layout(self):
        with tempfile.TemporaryDirectory() as tmp:
            pack = cable_installer.download_pack(pathlib.Path(tmp), opener=self._opener(
                self._zip({"VBCABLE_Setup_x64.exe": b"MZ", "VBCABLE_Setup.exe": b"MZ"})))
            with patch.dict(os.environ, {"PROCESSOR_ARCHITECTURE": "AMD64"}, clear=False):
                self.assertEqual(cable_installer.setup_program(pack).name, "VBCABLE_Setup_x64.exe")

    def test_download_rejects_path_traversal_and_junk(self):
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaises(cable_installer.InstallError) as caught:
                cable_installer.download_pack(pathlib.Path(tmp), opener=self._opener(
                    self._zip({"../evil.exe": b"MZ"})))
            self.assertEqual(caught.exception.kind, "download")
            with self.assertRaises(cable_installer.InstallError):
                cable_installer.download_pack(pathlib.Path(tmp), opener=self._opener(b"<html>"))

    def test_only_vb_audio_signatures_are_accepted(self):
        def run(stdout):
            return lambda *a, **k: mock.Mock(stdout=stdout)
        exe = pathlib.Path("VBCABLE_Setup_x64.exe")
        subject = "CN=BUREL VINCENT Entrepreneur individuel, O=BUREL VINCENT"
        self.assertIn("BUREL", cable_installer.verify_signature(exe, run=run(f"Valid|{subject}")))
        for stdout in ("NotSigned|", f"HashMismatch|{subject}", "Valid|CN=Someone Else"):
            with self.assertRaises(cable_installer.InstallError) as caught:
                cable_installer.verify_signature(exe, run=run(stdout))
            self.assertEqual(caught.exception.kind, "signature")

    def test_install_runs_the_verified_setup_and_cleans_up(self):
        stages, seen = [], {}
        with tempfile.TemporaryDirectory() as tmp:
            work = pathlib.Path(tmp) / "work"
            work.mkdir()

            def fake_download(dest):
                pack = dest / "pack"
                pack.mkdir()
                (pack / "VBCABLE_Setup_x64.exe").write_bytes(b"MZ")
                return pack

            with patch.object(cable_installer.tempfile, "mkdtemp", return_value=str(work)), \
                    patch.object(cable_installer, "download_pack", fake_download), \
                    patch.object(cable_installer, "verify_signature",
                                 lambda path: seen.setdefault("verified", path.name)), \
                    patch.object(cable_installer, "run_elevated",
                                 lambda path: seen.setdefault("ran", path.name) and 0), \
                    patch.object(cable_installer, "wait_for_endpoint", lambda: False), \
                    patch.object(cable_installer, "driver_installed", lambda: True), \
                    patch.dict(os.environ, {"PROCESSOR_ARCHITECTURE": "AMD64"}, clear=False):
                self.assertIs(cable_installer.install(stages.append), False)
            self.assertEqual(stages, ["download", "verify", "install", "devices"])
            self.assertEqual(seen, {"verified": "VBCABLE_Setup_x64.exe",
                                    "ran": "VBCABLE_Setup_x64.exe"})
            self.assertFalse(work.exists())


class CableDeviceTests(unittest.TestCase):
    class Reg:
        HKEY_LOCAL_MACHINE = 1

        def __init__(self, services=(), devices=()):
            self.services = set(services)
            self.devices = list(devices)   # (state, desc, iface)

        class Key:
            def __init__(self, reg, path):
                self.reg, self.path = reg, path

            def __enter__(self):
                return self

            def __exit__(self, *_exc):
                return False

        def OpenKey(self, parent, path):
            if parent == self.HKEY_LOCAL_MACHINE and "Services" in path:
                if path.rsplit("\\", 1)[-1] not in self.services:
                    raise OSError("no service")
                return self.Key(self, path)
            if parent == self.HKEY_LOCAL_MACHINE:
                return self.Key(self, "render")
            if parent.path == "render":
                return self.Key(self, path)
            return self.Key(self, parent.path + "/props")

        def CloseKey(self, _key):
            pass

        def EnumKey(self, key, index):
            if index >= len(self.devices):
                raise OSError("done")
            return str(index)

        def QueryValueEx(self, key, name):
            state, desc, iface = self.devices[int(key.path.split("/")[0])]
            if name == "DeviceState":
                return state, 4
            return (desc if name.endswith("},2") else iface), 1

    def test_driver_and_endpoint_detection(self):
        self.assertTrue(cable_installer.driver_installed(self.Reg(services={"VBAudioVACMME"})))
        self.assertFalse(cable_installer.driver_installed(self.Reg()))
        active = self.Reg(devices=[(1, "Speakers", "Realtek(R) Audio"),
                                   (1, "CABLE Input", "VB-Audio Virtual Cable")])
        self.assertTrue(cable_installer.endpoint_active(active))
        pending = self.Reg(devices=[(4, "CABLE Input", "VB-Audio Virtual Cable")])
        self.assertFalse(cable_installer.endpoint_active(pending))

    def test_wait_for_endpoint_gives_windows_a_moment(self):
        clock = [0.0]
        answers = iter([False, False, True])
        self.assertTrue(cable_installer.wait_for_endpoint(
            5, check=lambda: next(answers), clock=lambda: clock[0],
            sleep=lambda s: clock.__setitem__(0, clock[0] + s)))
        clock[0] = 0.0
        self.assertFalse(cable_installer.wait_for_endpoint(
            2, check=lambda: False, clock=lambda: clock[0],
            sleep=lambda s: clock.__setitem__(0, clock[0] + s)))

    def test_installed_driver_without_devices_asks_for_a_restart(self):
        waiting = env(outputs=["", SPEAKERS], cable_driver=True)
        cable = by_key(setup_check.run_checks(cfg(), waiting))["cable"]
        self.assertEqual((cable.status, cable.state), (setup_check.WARN, "restart"))
        self.assertTrue(i18n.has("setup_cable_restart"))


class ConfigTests(unittest.TestCase):
    def test_existing_configs_keep_the_full_dashboard(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = pathlib.Path(tmp) / "config.yaml"
            with patch.object(config, "CONFIG_PATH", path):
                self.assertTrue(config.load()["ui"]["easy_mode"])      # new install
                path.write_text(yaml.safe_dump({"ui": {"lang": "ko"}}), encoding="utf-8")
                self.assertFalse(config.load()["ui"]["easy_mode"])     # saved before
                path.write_text(yaml.safe_dump({"ui": {"easy_mode": True}}), encoding="utf-8")
                self.assertTrue(config.load()["ui"]["easy_mode"])


class OscStatusTests(unittest.TestCase):
    def test_any_vrchat_packet_marks_osc_alive(self):
        from pythonosc.udp_client import SimpleUDPClient
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as probe:
            probe.bind(("127.0.0.1", 0))
            port = probe.getsockname()[1]
        control = OscControl(AppState(), listen_port=port)
        self.assertIsNone(control.bound)
        self.assertTrue(control.start())
        try:
            self.assertTrue(control.bound)
            client = SimpleUDPClient("127.0.0.1", port)
            client.send_message("/avatar/parameters/Viseme", 3)
            client._sock.close()
            deadline = time.time() + 3
            while not control.last_rx and time.time() < deadline:
                time.sleep(0.02)
            self.assertGreater(control.last_rx, 0)
            busy = OscControl(AppState(), listen_port=port)
            self.assertFalse(busy.start())
            self.assertIs(busy.bound, False)
        finally:
            control.stop()


class FakeProbe:
    def __init__(self, names):
        self.names = list(names)
        self.levels = {name: 0.0 for name in self.names}
        self.failed = ["Broken mic"] if "Broken mic" in self.names else []
        self.loudest = ""
        self.started = self.stopped = False

    def start(self):
        self.started = True

    def best(self):
        return self.loudest

    def stop(self):
        self.stopped = True


class VrMicFinderTests(unittest.TestCase):
    def test_dashboard_finds_and_applies_the_speaking_mic(self):
        from vrclt.vr import dashboard_panel
        probes, applied = [], []

        def factory(names):
            probes.append(FakeProbe(names))
            return probes[-1]

        panel = dashboard_panel.DashboardPanel(
            AppState(), ["en"], get_provider=lambda: "soniox",
            get_devices=lambda: (["", REALTEK, QUEST, CABLE_OUT], ["", CABLE_IN]),
            get_mic_device=lambda: "", get_tts_device=lambda: CABLE_IN,
            set_audio_devices=lambda mic, out, done: applied.append((mic, out)) or done(True),
            mic_probe_factory=factory)
        status = (True, "status_running", "")
        try:
            panel._page = "audio"
            panel._on_click("mic_find")
            probe = probes[0]
            self.assertTrue(probe.started)
            self.assertNotIn(CABLE_OUT, probe.names)          # never the loop
            panel._render(status)                              # listening prompt
            now = time.time()
            probe.loudest = QUEST
            panel._tick_mic_probe(now)
            panel._tick_mic_probe(now + 0.5)
            self.assertIsNotNone(panel._mic_probe)             # not confirmed yet
            panel._render(status)
            panel._tick_mic_probe(now + 1.5)
            self.assertIsNone(panel._mic_probe)
            self.assertTrue(probe.stopped)
            self.assertEqual(panel._probe_result[:2], ("found", QUEST))
            panel._maybe_apply_devices(time.time() + 1)
            self.assertEqual(applied, [(QUEST, None)])
            panel._render(status)                              # result caption
            # nobody speaks: the search ends on its own
            panel._on_click("mic_find")
            panel._tick_mic_probe(time.time() + dashboard_panel.PROBE_TIMEOUT_SEC + 1)
            self.assertEqual(panel._probe_result[0], "none")
            self.assertTrue(probes[1].stopped)
            # pressing again while listening cancels without applying
            panel._on_click("mic_find")
            panel._on_click("mic_find")
            self.assertIsNone(panel._mic_probe)
            self.assertIsNone(panel._probe_result)
            self.assertEqual(len(applied), 1)
        finally:
            panel.detach()


class QtTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.app = QtWidgets.QApplication.instance() or QtWidgets.QApplication([])
        # auto setup writes VRChat's settings: tests must never reach the registry
        cls._patches = [patch.object(vrchat_settings, "write", mock.Mock(return_value={})),
                        patch.object(vrchat_settings, "running", mock.Mock(return_value=False))]
        for patcher in cls._patches:
            patcher.start()

    @classmethod
    def tearDownClass(cls):
        for patcher in cls._patches:
            patcher.stop()

    def test_mic_finder_preselects_the_speaking_mic(self):
        from vrclt.ui.mic_finder import MicFinderDialog
        probes = []

        def factory(names):
            probes.append(FakeProbe(names))
            return probes[-1]

        tr = lambda key: i18n.tr("en", key)
        dialog = MicFinderDialog(tr, [REALTEK, QUEST, "Broken mic"], current=REALTEK,
                                 probe_factory=factory)
        dialog.show()
        probe = probes[0]
        self.assertTrue(probe.started)
        self.assertFalse(dialog._rows["Broken mic"][0].isEnabled())
        probe.loudest = QUEST
        probe.levels[QUEST] = 1800.0
        dialog._tick()
        self.assertTrue(dialog._rows[QUEST][0].isChecked())
        self.assertIn(QUEST, dialog._status.text())
        dialog.accept()
        self.assertEqual(dialog.selected, QUEST)
        self.assertTrue(probe.stopped)

    def test_mic_finder_respects_a_manual_choice(self):
        from vrclt.ui.mic_finder import MicFinderDialog
        probe = FakeProbe([REALTEK, QUEST])
        dialog = MicFinderDialog(lambda key: i18n.tr("en", key), probe.names,
                                 probe_factory=lambda names: probe)
        dialog.show()
        dialog._rows[REALTEK][0].click()
        probe.loudest = QUEST
        dialog._tick()
        self.assertTrue(dialog._rows[REALTEK][0].isChecked())
        dialog.reject()
        self.assertTrue(probe.stopped)

    def _window(self, easy: bool, environment):
        from smoke_qt import StubController
        from vrclt.qt_ui import MainWindow
        from vrclt.ui.update_banner import UpdateBanner
        ctl = StubController()
        ctl.cfg["ui"]["easy_mode"] = easy
        ctl.cfg["hotkeys"]["enabled"] = False
        ctl.cfg["api_key"] = "x" * 39
        with patch.object(UpdateBanner, "start_check", return_value=False), \
                patch.object(MainWindow, "_collect_setup", lambda self: environment):
            win = MainWindow(ctl, pathlib.Path(__file__).parent / "nonexistent.log")
        win._timer.stop()
        win._desktop_overlay._timer.stop()
        win._collect_setup = lambda: environment
        return ctl, win

    def test_easy_and_full_dashboard(self):
        ctl, win = self._window(True, env())
        try:
            win.show()
            win._refresh()
            self.app.processEvents()
            full_only = (win._text_only, win._out_language_options,
                         win._in_language_options, win._audio_output_column,
                         win._close_action, win._audio_group,
                         win._engine_button)
            self.assertTrue(all(widget.isHidden() for widget in full_only))
            # subtitle size/position and the UI language: both views, unfolded
            both_views = (win._overlay_font_size, win._btn_overlay_move,
                          win._btn_overlay_reset, win._ui_lang)
            self.assertTrue(all(not widget.isHidden() for widget in both_views))
            self.assertFalse(win._easy_mic_meter.isHidden())
            self.assertFalse(win._easy_mic_wrap.isHidden())          # mic + Find
            self.assertFalse(win._btn_easy_find_mic.isHidden())
            self.assertFalse(win._mode_group.isHidden())   # VRChat / Discord / custom
            self.assertFalse(win._btn_restart.isHidden())
            self.assertFalse(win._setup_panel.isHidden())  # always: one line when ready
            self.assertTrue(win._setup_panel._body.isHidden())
            self.assertTrue(win._view_buttons[True].isChecked())
            win._view_buttons[False].click()
            self.assertFalse(ctl.cfg["ui"]["easy_mode"])
            self.assertTrue(all(not widget.isHidden() for widget in full_only))
            self.assertTrue(all(not widget.isHidden() for widget in both_views))
            self.assertTrue(win._easy_mic_meter.isHidden())
            self.assertTrue(win._easy_mic_wrap.isHidden())
            self.assertFalse(win._btn_find_mic.isHidden())           # full view's own
            self.assertFalse(win._setup_panel.isHidden())
            # Qwen 3.5 requires the spoken language even in the easy view
            win._view_buttons[True].click()
            ctl.cfg["provider"] = "qwen"
            ctl.cfg["qwen"]["model"] = "qwen3.5-livetranslate-flash-realtime"
            win._refresh()
            self.assertFalse(win._src_lang.isHidden())
            ctl.cfg["provider"] = "soniox"   # optional hints stay in the full view
            win._refresh()
            self.assertTrue(win._src_lang.isHidden())
            self.assertTrue(win._keep_speaker_context.isHidden())
            # the setup panel folds to one line when everything is ready
            self.assertEqual(win._setup_panel.issue_count(), 0)
            self.assertTrue(win._setup_panel._body.isHidden())
            self.assertIn(REALTEK, win._mic_device.lineEdit().placeholderText())
            win._inputs = INPUTS
            win._sync_dashboard_devices(force=True)
            for combo in (win._mic_device, win._easy_mic_device):
                offered = [combo.itemText(i) for i in range(combo.count())]
                self.assertNotIn(CABLE_OUT, offered)
                self.assertIn(QUEST, offered)
            ctl.cfg["outbound"]["mic_device"] = CABLE_OUT   # still shown when configured
            win._sync_dashboard_devices(force=True)
            self.assertEqual(win._mic_device.currentText(), CABLE_OUT)
            ctl.cfg["outbound"]["mic_device"] = ""
            self.assertIn("CABLE Output", win._route_hint.text())
        finally:
            win._quitting = True
            win.close()

    def test_auto_setup_saves_and_restarts(self):
        broken = env(default_input=CABLE_OUT, vrchat=VRChatSettings(QUEST, None, True))
        ctl, win = self._window(True, broken)
        restarts = []
        ctl.restart = lambda cfg: restarts.append(copy.deepcopy(cfg)) or True
        try:
            ctl.cfg["outbound"]["tts_device"] = ""
            win._setup_checked_at = 0.0
            win._refresh()
            self.assertEqual(win._setup_panel.issue_count(), 3)
            self.assertFalse(win._setup_panel._body.isHidden())
            asked = []
            win._confirm_auto_setup = lambda lines, vrchat: asked.append(lines) or True
            with patch.object(config, "save") as save:
                win._setup_panel._btn_fix.click()
                deadline = time.time() + 5
                while not restarts and time.time() < deadline:
                    self.app.processEvents()
                    time.sleep(0.02)
            self.assertTrue(save.called)
            saved = save.call_args.args[0]["outbound"]
            self.assertEqual((saved["tts_device"], saved["mic_device"]), ("CABLE Input", QUEST))
            self.assertEqual(restarts[0]["outbound"]["mic_device"], QUEST)
            vrchat_settings.write.assert_called_once_with(mic=CABLE_OUT)
            self.assertEqual(len(asked[0]), 3)   # voice output, mic, VRChat mic
            win._device_done(True)
            self.assertIn("VRChat", win._dashboard_note.text())
            self.assertFalse(win._setup_panel._btn_undo.isHidden())
        finally:
            vrchat_settings.write.reset_mock()
            win._quitting = True
            win.close()

    def test_cancelled_auto_setup_changes_nothing_and_undo_restores(self):
        broken = env(default_input=CABLE_OUT, vrchat=VRChatSettings(QUEST, None, True))
        ctl, win = self._window(True, broken)
        restarts = []
        ctl.restart = lambda cfg: restarts.append(copy.deepcopy(cfg)) or True
        try:
            ctl.cfg["outbound"]["tts_device"] = ""
            win._confirm_auto_setup = lambda lines, vrchat: False
            with patch.object(config, "save") as save:
                win._on_auto_setup_clicked()
            save.assert_not_called()
            vrchat_settings.write.assert_not_called()
            self.assertIsNone(win._autofix_undo)
            # apply, then undo
            vrchat_settings.write.return_value = {"VRC_INPUT_MIC_DEVICE_NAME_VR_h2729556311": QUEST}
            win._confirm_auto_setup = lambda lines, vrchat: True
            with patch.object(config, "save"):
                win._on_auto_setup_clicked()
                self._wait(restarts, 1)
                win._device_done(True)
            with patch.object(vrchat_settings, "restore") as restore, \
                    patch.object(config, "save") as save:
                win._setup_panel._btn_undo.click()
                self._wait(restarts, 2)
            restore.assert_called_once_with({"VRC_INPUT_MIC_DEVICE_NAME_VR_h2729556311": QUEST})
            undone = save.call_args.args[0]["outbound"]
            self.assertEqual((undone["tts_device"], undone["mic_device"]), ("", ""))
            win._device_done(True)
            self.assertTrue(win._setup_panel._btn_undo.isHidden())
            self.assertEqual(win._dashboard_note.text(),
                             i18n.tr(ctl.state.ui_lang, "setup_undo_done"))
        finally:
            vrchat_settings.write.reset_mock()
            vrchat_settings.write.return_value = {}
            win._quitting = True
            win.close()

    def _wait(self, items, count):
        deadline = time.time() + 5
        while len(items) < count and time.time() < deadline:
            self.app.processEvents()
            time.sleep(0.02)
        self.assertGreaterEqual(len(items), count)

    def test_auto_setup_leaves_a_running_vrchat_alone(self):
        wrong = env(vrchat=VRChatSettings(QUEST, "", False))
        ctl, win = self._window(True, wrong)
        ctl.cfg["outbound"]["mic_device"] = REALTEK   # nothing for the app config
        try:
            vrchat_settings.running.return_value = True
            win._run_auto_setup()   # the post-install path: no confirmation
            vrchat_settings.write.assert_not_called()
            self.assertEqual(win._dashboard_note.text(),
                             i18n.tr(ctl.state.ui_lang, "setup_vrchat_running"))
            vrchat_settings.running.return_value = False
            win._run_auto_setup()
            vrchat_settings.write.assert_called_once_with(mic=CABLE_OUT, osc=True)
        finally:
            vrchat_settings.running.return_value = False
            vrchat_settings.write.reset_mock()
            win._quitting = True
            win.close()

    def test_cable_install_outcomes(self):
        ctl, win = self._window(True, env(outputs=["", SPEAKERS]))
        tr = lambda key: i18n.tr(ctl.state.ui_lang, key)
        try:
            win._cable_done("cancelled", "")
            self.assertEqual(win._dashboard_note.text(), tr("msg_cable_cancelled"))
            win._cable_done("signature", "NotSigned")
            self.assertIn("NotSigned", win._dashboard_note.text())
            self.assertIn("NotSigned", win._cable_box.text())   # failure in a window
            win._cable_box.close()
            win._show_cable_progress(tr("msg_cable_downloading"))
            win._cable_progress("msg_cable_installing")
            self.assertTrue(win._cable_progress_dialog.isVisible())
            self.assertEqual(win._cable_progress_dialog.labelText(), tr("msg_cable_installing"))
            with patch.object(win, "_reload_devices") as reload:
                win._cable_done("", "")
            reload.assert_called_once()
            self.assertTrue(win._cable_followup)
            self.assertEqual(win._cable_progress_dialog.labelText(), tr("msg_cable_done"))
            # devices reread: no cable visible yet -> ask for a reboot
            with patch("vrclt.qt_ui._device_names", return_value=(INPUTS, ["", SPEAKERS])):
                win._devices_reloaded(True)
            self.assertIsNone(win._cable_progress_dialog)
            self.assertEqual(win._dashboard_note.text(), tr("msg_cable_not_found"))
            self.assertEqual(win._cable_box.text(), tr("msg_cable_not_found"))
            win._cable_box.close()
            win._cable_followup = True
            with patch("vrclt.qt_ui._device_names", return_value=(INPUTS, ["", SPEAKERS, CABLE_IN])), \
                    patch.object(win, "_run_auto_setup") as auto:
                win._devices_reloaded(True)
            auto.assert_called_once()
        finally:
            win._quitting = True
            win.close()

    def test_restart_needed_then_auto_setup_resumes(self):
        ctl, win = self._window(True, env(outputs=["", SPEAKERS], cable_driver=True))
        try:
            win._cable_followup = True
            with patch("vrclt.qt_ui._device_names", return_value=(INPUTS, ["", SPEAKERS])):
                win._devices_reloaded(True)
            self.assertTrue(ctl.cfg["meta"]["auto_setup_pending"])
            actions = [b.property("actionKey") for b in
                       win._setup_panel.findChildren(QtWidgets.QPushButton)
                       if b.property("actionKey")]
            self.assertNotIn("setup_btn_install_cable", actions)   # no reinstall offer
            # the next start: CABLE is there now
            win._collect_setup = env
            with patch.object(win, "_run_auto_setup") as auto:
                win._sync_setup_panel(force=True)
                self.app.processEvents()
            auto.assert_called_once()
            self.assertFalse(ctl.cfg["meta"]["auto_setup_pending"])
        finally:
            win._quitting = True
            win.close()

    def test_window_fits_the_screen(self):
        ctl, win = self._window(True, env())
        try:
            area = win.screen().availableGeometry()
            self.assertLessEqual(win.height(), area.height())
            self.assertLessEqual(win.width(), area.width())
            self.assertLessEqual(win.minimumHeight(), area.height())
        finally:
            win._quitting = True
            win.close()


if __name__ == "__main__":
    unittest.main(verbosity=2)
