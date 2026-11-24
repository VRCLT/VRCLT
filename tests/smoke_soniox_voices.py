"""Soniox voice cloning: REST client against a local HTTP server, the mic
sample recorder, the cloning dialog and the Settings voice field.

No credentials, paid requests or audio devices are required.
Run with .venv/Scripts/python tests/smoke_soniox_voices.py.
"""
import io
import json
import os
import pathlib
import sys
import tempfile
import threading
import unittest
import wave
from email.parser import BytesParser
from email.policy import HTTP
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from unittest.mock import patch

os.environ["QT_QPA_PLATFORM"] = "offscreen"
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent.parent))

import numpy as np
from PySide6 import QtWidgets

from vrclt import config as config_mod
from vrclt import i18n
from vrclt.audio import voice_sample
from vrclt.session_base import FatalSessionError
from vrclt.soniox import session as sx
from vrclt.soniox import voices
from vrclt.ui import soniox_voices as ui

sys.path.insert(0, os.path.dirname(__file__))
from smoke_qt import StubController

KEY = "test-soniox-key"
MODEL = "tts-rt-v2"
ID_A = "11111111-1111-4111-8111-111111111111"
ID_B = "22222222-2222-4222-8222-222222222222"
ID_NEW = "33333333-3333-4333-8333-333333333333"


def voice_json(voice_id, name, status=voices.READY, **error):
    return {"id": voice_id, "name": name, "filename": name + ".wav",
            "created_at": "2026-11-24T00:00:00Z",
            "models": [{"model": MODEL, "status": status,
                        "error_type": error.get("error_type"),
                        "error_message": error.get("error_message")}]}


class FakeSoniox(ThreadingHTTPServer):
    """Just enough of api.soniox.com for the voice endpoints."""

    def __init__(self):
        super().__init__(("127.0.0.1", 0), _Handler)
        self.voices = {ID_A: voice_json(ID_A, "Alpha"), ID_B: voice_json(ID_B, "Beta")}
        self.requests = []
        self.uploads = []
        self.polls_until_ready = 0
        self.page_size = 1
        self.thread = threading.Thread(target=self.serve_forever, daemon=True)
        self.thread.start()

    @property
    def url(self):
        return f"http://127.0.0.1:{self.server_address[1]}"

    def close(self):
        self.shutdown()
        self.server_close()


class _Handler(BaseHTTPRequestHandler):
    def log_message(self, *_args):
        pass

    def _send(self, status, body=None):
        raw = b"" if body is None else json.dumps(body).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(raw)))
        self.end_headers()
        self.wfile.write(raw)

    def _error(self, status, kind, message, **extra):
        self._send(status, {"status_code": status, "error_type": kind, "message": message,
                            "validation_errors": extra.get("validation", []),
                            "request_id": "req-1"})

    def _route(self, method):
        server = self.server
        body = self.rfile.read(int(self.headers.get("Content-Length") or 0))
        server.requests.append((method, self.path))
        if self.headers.get("Authorization") != f"Bearer {KEY}":
            return self._error(401, "unauthenticated", f"Invalid API key {KEY}.")
        path, _, query = self.path.partition("?")
        parts = path.strip("/").split("/")
        if path == "/v1/tts-models" and method == "GET":
            return self._send(200, {"models": [
                {"id": MODEL, "supports_voice_cloning": True,
                 "voice_cloning_max_audio_duration_ms": 90000},
                {"id": "tts-old", "supports_voice_cloning": False,
                 "voice_cloning_max_audio_duration_ms": None}]})
        if path == "/v1/voices" and method == "GET":
            items = list(server.voices.values())
            start = int(dict(p.split("=") for p in query.split("&") if "=" in p)
                        .get("cursor", "0") or 0)
            page = items[start:start + server.page_size]
            more = start + server.page_size < len(items)
            return self._send(200, {"voices": page,
                                    "next_page_cursor": str(start + server.page_size)
                                    if more else None})
        if path == "/v1/voices" and method == "POST":
            message = BytesParser(policy=HTTP).parsebytes(
                b"Content-Type: " + self.headers["Content-Type"].encode() + b"\r\n\r\n" + body)
            fields = {part.get_param("name", header="content-disposition"): part
                      for part in message.iter_parts()}
            # like a browser form, the text part is raw UTF-8 with no charset
            name = fields["name"].get_payload(decode=True).decode("utf-8").strip()
            if any(v["name"] == name for v in server.voices.values()):
                return self._error(409, "voice_name_conflict", "Voice name already exists.")
            if name == "bad":
                return self._error(400, "invalid_request", "Invalid request.", validation=[
                    {"error_type": "invalid", "location": "file",
                     "message": "file is not audio"}])
            server.uploads.append((name, fields["file"].get_filename(),
                                   fields["file"].get_content_type(),
                                   fields["file"].get_payload(decode=True)))
            server.voices[ID_NEW] = voice_json(ID_NEW, name, voices.PROCESSING)
            return self._send(201, server.voices[ID_NEW])
        if len(parts) >= 3 and parts[:2] == ["v1", "voices"]:
            voice = server.voices.get(parts[2])
            if voice is None:
                return self._error(404, "voice_not_found", "Voice not found.")
            state = next(iter(voice["models"]), None)
            if len(parts) == 4 and parts[3] == "recompute" and method == "POST":
                assert json.loads(body or b"{}") == {"model": MODEL}
                if state is None:
                    voice["models"].append({"model": MODEL})
                voice["models"][0]["status"] = voices.PROCESSING
                return self._send(200, voice)
            if len(parts) == 3 and method == "GET":
                if state is not None and state["status"] == voices.PROCESSING:
                    if server.polls_until_ready <= 0:
                        state["status"] = voices.READY
                    server.polls_until_ready -= 1
                return self._send(200, voice)
            if len(parts) == 3 and method == "DELETE":
                del server.voices[parts[2]]
                self.send_response(204)
                self.send_header("Content-Length", "0")
                self.end_headers()
                return None
        return self._error(404, "not_found", "No such endpoint.")

    def do_GET(self):
        self._route("GET")

    def do_POST(self):
        self._route("POST")

    def do_DELETE(self):
        self._route("DELETE")


class ClientTests(unittest.TestCase):
    def setUp(self):
        self.server = FakeSoniox()
        self.addCleanup(self.server.close)
        self.client = voices.VoiceClient(KEY, base_url=self.server.url, timeout=5)

    def test_voice_ids_are_uuids(self):
        self.assertTrue(voices.is_voice_id(f"  {ID_A.upper()} "))
        for value in ("Daniel", "", None, ID_A[:-1], ID_A + "0"):
            self.assertFalse(voices.is_voice_id(value))

    def test_list_follows_pages_and_reads_model_states(self):
        self.server.voices[ID_B] = voice_json(
            ID_B, "Beta", voices.FAILED, error_type="voice_audio_too_long",
            error_message="The clip is longer than 2 minutes.")
        listed = self.client.list()
        self.assertEqual([v.name for v in listed], ["Alpha", "Beta"])
        self.assertEqual(sum(1 for r in self.server.requests if r[0] == "GET"), 2)
        self.assertEqual(listed[0].status(MODEL), voices.READY)
        self.assertEqual(listed[1].status(MODEL), voices.FAILED)
        self.assertEqual(listed[1].error(MODEL), "The clip is longer than 2 minutes.")
        # a model the service never prepared the voice for
        self.assertEqual(listed[0].status("tts-rt-v9"), voices.NOT_COMPUTED)

    def test_create_uploads_multipart_and_waits_until_ready(self):
        self.server.polls_until_ready = 2
        audio = bytes(range(256)) * 4 + b"\r\n--boundary\r\n"
        voice = self.client.create(" 내 목소리 ", audio, 'my "clip".wav')
        self.assertEqual(voice.id, ID_NEW)
        self.assertEqual(voice.status(MODEL), voices.PROCESSING)
        name, filename, media, payload = self.server.uploads[0]
        self.assertEqual((name, filename, payload), ("내 목소리", "my _clip_.wav", audio))
        self.assertIn(media, ("audio/x-wav", "audio/wav"))
        sleeps = []
        ready = self.client.wait_ready(ID_NEW, MODEL, sleep=sleeps.append)
        self.assertEqual(ready.status(MODEL), voices.READY)
        self.assertEqual(len(sleeps), 2)

    def test_wait_ready_recomputes_once_and_reports_failures(self):
        self.server.voices[ID_A]["models"] = []          # never prepared for MODEL
        ready = self.client.wait_ready(ID_A, MODEL, sleep=lambda _s: None)
        self.assertEqual(ready.status(MODEL), voices.READY)
        self.assertEqual(
            [r for r in self.server.requests if r[0] == "POST"],
            [("POST", f"/v1/voices/{ID_A}/recompute")])
        self.server.voices[ID_B] = voice_json(ID_B, "Beta", voices.FAILED,
                                              error_type="voice_invalid_audio")
        with self.assertRaises(voices.VoiceApiError) as failed:
            self.client.wait_ready(ID_B, MODEL)
        self.assertEqual(failed.exception.kind, "voice_failed")
        self.assertEqual(str(failed.exception), "voice_invalid_audio")

    def test_wait_ready_stops_on_timeout_and_cancel(self):
        self.server.voices[ID_A]["models"][0]["status"] = voices.PROCESSING
        self.server.polls_until_ready = 1000
        ticks = iter(range(0, 1000, 50))
        with self.assertRaises(voices.VoiceApiError) as late:
            self.client.wait_ready(ID_A, MODEL, timeout=120, sleep=lambda _s: None,
                                   clock=lambda: next(ticks))
        self.assertEqual(late.exception.kind, "timeout")
        with self.assertRaises(voices.VoiceApiError) as cancelled:
            self.client.wait_ready(ID_A, MODEL, cancelled=lambda: True,
                                   sleep=lambda _s: None)
        self.assertEqual(cancelled.exception.kind, "cancelled")

    def test_delete_and_model_limits(self):
        self.client.delete(ID_A)
        self.assertNotIn(ID_A, self.server.voices)
        with self.assertRaises(voices.VoiceApiError) as missing:
            self.client.get(ID_A)
        self.assertEqual((missing.exception.status, missing.exception.kind),
                         (404, "voice_not_found"))
        self.assertEqual(self.client.max_reference_sec(MODEL), 90.0)
        self.assertEqual(self.client.max_reference_sec("tts-old"), 0.0)
        self.assertIsNone(self.client.max_reference_sec("unknown"))

    def test_errors_are_classified_and_never_echo_the_key(self):
        with self.assertRaises(voices.VoiceApiError) as conflict:
            self.client.create("Alpha", b"audio", "a.wav")
        self.assertEqual((conflict.exception.status, conflict.exception.kind),
                         (409, "voice_name_conflict"))
        with self.assertRaises(voices.VoiceApiError) as invalid:
            self.client.create("bad", b"audio", "a.wav")
        self.assertEqual(str(invalid.exception), "file is not audio")
        wrong = voices.VoiceClient("other-key", base_url=self.server.url)
        with self.assertRaises(voices.VoiceApiError) as denied:
            wrong.list()
        self.assertEqual(denied.exception.status, 401)
        # the server's message quoted a key: the client's own key is redacted
        with patch.object(_Handler, "_error", lambda h, status, kind, message, **_e: h._send(
                status, {"error_type": kind, "message": f"bad key {KEY}"})):
            self.server.voices.clear()
            with self.assertRaises(voices.VoiceApiError) as quoted:
                self.client.get(ID_A)
        self.assertNotIn(KEY, str(quoted.exception))
        self.server.close()
        with self.assertRaises(voices.VoiceApiError) as offline:
            voices.VoiceClient(KEY, base_url=self.server.url, timeout=2).list()
        self.assertEqual(offline.exception.kind, "network")

    def test_create_validates_before_any_request(self):
        for name, audio, kind in (("", b"a", "invalid_name"), ("n" * 129, b"a", "invalid_name"),
                                  ("ok", b"", "invalid_audio")):
            with self.assertRaises(voices.VoiceApiError) as raised:
                self.client.create(name, audio, "a.wav")
            self.assertEqual(raised.exception.kind, kind)
        with patch.object(voices, "MAX_UPLOAD_BYTES", 4), \
                self.assertRaises(voices.VoiceApiError) as large:
            self.client.create("ok", b"12345", "a.wav")
        self.assertEqual(large.exception.kind, "audio_too_large")
        self.assertEqual(self.server.requests, [])


class SessionErrorTests(unittest.TestCase):
    def test_cloned_voice_errors_stop_instead_of_retrying(self):
        session = sx.SonioxLiveTranslateSession(
            api_key=KEY, model="", source=None, name="test",
            get_target_language=lambda: "en", voice=ID_A)
        self.assertEqual(session._voice, ID_A)
        for code, kind in ((404, "voice_not_found"), (503, "voice_failed"),
                           (409, "voice_not_prepared")):
            with self.assertRaises(FatalSessionError) as fatal:
                session._check_error({"error_code": code, "error_type": kind,
                                      "error_message": "Voice problem."})
            self.assertIn("cloned voice", str(fatal.exception))
        with self.assertRaises(sx._ServerError):       # still preparing: retried
            session._check_error({"error_code": 503, "error_type": "voice_not_ready",
                                  "error_message": "Voice is processing."})


class FakeStream:
    def __init__(self, **kwargs):
        self.kwargs = kwargs
        self.running = False

    def start(self):
        self.running = True

    def stop(self):
        self.running = False

    def close(self):
        pass


class RecorderTests(unittest.TestCase):
    def test_records_mono_wav_at_the_device_rate_up_to_the_limit(self):
        streams = []

        def stream(**kwargs):
            streams.append(FakeStream(**kwargs))
            return streams[-1]

        found = []
        with patch.object(voice_sample.devices, "find_input",
                          lambda name: found.append(name) or (None if name else 3)), \
                patch.object(voice_sample.sd, "query_devices",
                             lambda idx: {"default_samplerate": 44100.0}), \
                patch.object(voice_sample.sd, "InputStream", stream):
            recorder = voice_sample.VoiceSampleRecorder("Unplugged mic", max_sec=1.0)
            recorder.start()
        self.assertEqual(found, ["Unplugged mic", ""])     # falls back to the default mic
        self.assertEqual((streams[0].kwargs["samplerate"], streams[0].kwargs["channels"]),
                         (44100, 1))
        block = np.full((22050, 1), 1000, dtype=np.int16)
        callback = streams[0].kwargs["callback"]
        callback(block, len(block), None, None)
        self.assertAlmostEqual(recorder.seconds, 0.5)
        self.assertAlmostEqual(recorder.level, 1000.0, places=1)
        self.assertFalse(recorder.full)
        callback(block, len(block), None, None)
        callback(block, len(block), None, None)            # past the limit: dropped
        self.assertTrue(recorder.full)
        recorder.stop()
        self.assertFalse(streams[0].running)
        with wave.open(io.BytesIO(recorder.wav_bytes()), "rb") as clip:
            self.assertEqual((clip.getnchannels(), clip.getsampwidth(), clip.getframerate(),
                              clip.getnframes()), (1, 2, 44100, 44100))

    def test_no_microphone_is_reported(self):
        with patch.object(voice_sample.devices, "find_input", lambda name: None), \
                self.assertRaises(RuntimeError):
            voice_sample.VoiceSampleRecorder().start()


class FakeClient:
    def __init__(self, api_key):
        self.api_key = api_key
        self.voices = [voices.Voice.from_json(voice_json(ID_A, "Alpha")),
                       voices.Voice.from_json(voice_json(ID_B, "Beta", voices.NOT_COMPUTED))]
        self.calls = []
        self.limit = 90.0
        self.fail = None

    def _check(self):
        if self.fail is not None:
            raise self.fail

    def max_reference_sec(self, model):
        return self.limit

    def list(self):
        self._check()
        self.calls.append(("list",))
        return list(self.voices)

    def create(self, name, audio, filename):
        self._check()
        self.calls.append(("create", name, audio, filename))
        voice = voices.Voice.from_json(voice_json(ID_NEW, name, voices.PROCESSING))
        self.voices.append(voice)
        return voice

    def wait_ready(self, voice_id, model, **_kwargs):
        self._check()
        self.calls.append(("wait_ready", voice_id, model))
        index = next(i for i, v in enumerate(self.voices) if v.id == voice_id)
        self.voices[index] = voices.Voice.from_json(
            voice_json(voice_id, self.voices[index].name))
        return self.voices[index]

    def delete(self, voice_id):
        self.calls.append(("delete", voice_id))
        self.voices = [v for v in self.voices if v.id != voice_id]


class FakeRecorder:
    def __init__(self, device, max_sec):
        self.device, self.max_sec = device, max_sec
        self.seconds = 0.0
        self.level = 0.0
        self.full = False
        self.stopped = False
        self.fail = None

    def start(self):
        if self.fail:
            raise RuntimeError(self.fail)

    def stop(self):
        self.stopped = True

    def wav_bytes(self):
        return b"RIFF-recorded"


class DialogTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.app = QtWidgets.QApplication.instance() or QtWidgets.QApplication([])

    def dialog(self, *, api_key=KEY, current="", **overrides):
        self.clients, self.recorders = [], []

        def client(key):
            self.clients.append(FakeClient(key))
            for name, value in overrides.items():
                setattr(self.clients[-1], name, value)
            return self.clients[-1]

        def recorder(device, max_sec):
            self.recorders.append(FakeRecorder(device, max_sec))
            return self.recorders[-1]

        dialog = ui.SonioxVoiceDialog(
            lambda key: i18n.tr("en", key), api_key=api_key, model=MODEL, current=current,
            mic_device="Headset", client_factory=client, recorder_factory=recorder,
            run_async=lambda work: work())
        self.addCleanup(dialog.close)
        dialog.show()
        return dialog

    def test_every_dialog_string_exists_in_every_language(self):
        keys = [k for k in i18n.STRINGS if k.startswith("sx_voice_")]
        self.assertGreater(len(keys), 30)
        for key in keys + ["f.soniox.voice.tip"]:
            for lang in i18n.LANGS:
                self.assertTrue(i18n.STRINGS[key].get(lang), (key, lang))
        source = pathlib.Path(ui.__file__).read_text(encoding="utf-8")
        import re
        for key in set(re.findall(r"[\"'](sx_voice_\w+)[\"']", source)):
            self.assertIn(key, i18n.STRINGS, key)

    def test_without_a_key_nothing_is_requested(self):
        dialog = self.dialog(api_key="  ")
        self.assertEqual(self.clients, [])
        self.assertEqual(dialog._status.text(), "Enter the Soniox API key first.")
        for button in (dialog._btn_refresh, dialog._btn_record, dialog._btn_file,
                       dialog._btn_create, dialog._btn_use, dialog._btn_delete):
            self.assertFalse(button.isEnabled())

    def test_lists_voices_and_uses_a_ready_one(self):
        dialog = self.dialog(current=ID_A)
        self.assertEqual(dialog._list.count(), 2)
        self.assertEqual(dialog._list.item(0).text(), "Alpha  ·  ready  ·  in use")
        self.assertEqual(dialog._list.item(1).text(), "Beta  ·  not prepared")
        self.assertEqual(dialog._voice().id, ID_A)          # the current voice is preselected
        self.assertIn("1:30", dialog._intro.text())         # the model's own clip limit
        self.assertEqual(dialog._list_box.title(), "My cloned voices (2)")
        dialog._btn_use.click()
        self.assertEqual(dialog.selected, ID_A)
        self.assertEqual(dialog.result(), QtWidgets.QDialog.DialogCode.Accepted)
        self.assertNotIn("wait_ready", [c[0] for c in self.clients[0].calls])

    def test_using_an_unprepared_voice_prepares_it_first(self):
        dialog = self.dialog()
        dialog._list.setCurrentRow(1)
        dialog._btn_use.click()
        self.assertIn(("wait_ready", ID_B, MODEL), self.clients[0].calls)
        self.assertEqual(dialog.selected, ID_B)

    def test_record_then_create_selects_the_new_voice(self):
        dialog = self.dialog()
        self.assertFalse(dialog._btn_create.isEnabled())
        dialog._btn_record.click()
        recorder = self.recorders[0]
        self.assertEqual((recorder.device, recorder.max_sec), ("Headset", 90.0))
        self.assertEqual(dialog._btn_record.text(), "Stop recording")
        self.assertFalse(dialog._btn_file.isEnabled())
        recorder.seconds, recorder.level = 1.0, 500.0
        dialog._tick()
        self.assertEqual(dialog._sample_label.text(), "Recording 0:01 / 1:30")
        dialog._btn_record.click()                          # too short to be a sample
        self.assertTrue(recorder.stopped)
        self.assertIn("too short", dialog._status.text())
        self.assertFalse(dialog._has_sample())
        dialog._btn_record.click()
        recorder = self.recorders[1]
        recorder.seconds, recorder.full = 90.0, True
        dialog._tick()                                      # the limit stops it by itself
        self.assertIsNone(dialog._recorder)
        self.assertEqual(dialog._sample_label.text(), "Recorded 1:30")
        self.assertFalse(dialog._btn_create.isEnabled())    # still needs a name
        dialog._name.setText("  Mine ")
        dialog._btn_create.click()
        client = self.clients[0]
        self.assertIn(("create", "Mine", b"RIFF-recorded", ui.RECORDED_FILENAME), client.calls)
        self.assertIn(("wait_ready", ID_NEW, MODEL), client.calls)
        self.assertEqual(dialog._voice().id, ID_NEW)
        self.assertEqual(dialog._list.currentItem().text(), "Mine  ·  ready")
        self.assertIn("'Mine' is ready", dialog._status.text())
        self.assertEqual(dialog._name.text(), "")
        self.assertFalse(dialog._has_sample())
        dialog._btn_use.click()
        self.assertEqual(dialog.selected, ID_NEW)

    def test_file_sample_and_size_limit(self):
        dialog = self.dialog()
        with tempfile.TemporaryDirectory() as folder:
            clip = pathlib.Path(folder) / "My Voice.mp3"
            clip.write_bytes(b"ID3-audio")
            with patch.object(QtWidgets.QFileDialog, "getOpenFileName",
                              return_value=(str(clip), "")):
                with patch.object(ui.voices_api, "MAX_UPLOAD_BYTES", 4):
                    dialog._btn_file.click()
                self.assertIn("too large", dialog._status.text())
                self.assertFalse(dialog._has_sample())
                dialog._btn_file.click()
            self.assertEqual(dialog._sample_label.text(), "My Voice.mp3")
            self.assertEqual(dialog._name.text(), "My Voice")      # prefilled from the file
            dialog._btn_create.click()
        self.assertIn(("create", "My Voice", b"ID3-audio", "My Voice.mp3"),
                      self.clients[0].calls)
        with patch.object(QtWidgets.QFileDialog, "getOpenFileName", return_value=("", "")):
            dialog._btn_file.click()                         # cancelled: nothing changes
        self.assertFalse(dialog._has_sample())

    def test_delete_asks_first_and_reports_the_deleted_voice(self):
        dialog = self.dialog(current=ID_A)
        buttons = QtWidgets.QMessageBox.StandardButton
        with patch.object(QtWidgets.QMessageBox, "question", return_value=buttons.No):
            dialog._btn_delete.click()
        self.assertNotIn(("delete", ID_A), self.clients[0].calls)
        with patch.object(QtWidgets.QMessageBox, "question",
                          return_value=buttons.Yes) as asked:
            dialog._btn_delete.click()
        self.assertIn("'Alpha'", asked.call_args.args[2])
        self.assertEqual(dialog.deleted, {ID_A})
        self.assertEqual(dialog._list.count(), 1)
        dialog.reject()
        self.assertEqual(dialog.selected, "")

    def test_errors_and_unsupported_models_are_shown(self):
        dialog = self.dialog(fail=voices.VoiceApiError(401, "unauthenticated", "Bad key."))
        self.assertEqual(dialog._status.text(), "Error: Bad key.")
        self.assertEqual(dialog._list.count(), 0)
        self.assertTrue(dialog._btn_refresh.isEnabled())     # retry stays possible
        dialog = self.dialog(limit=0.0)
        self.assertIn("does not support voice cloning", dialog._status.text())
        self.assertFalse(dialog._btn_record.isEnabled())
        dialog._list.setCurrentRow(0)
        self.assertTrue(dialog._btn_use.isEnabled())         # existing voices stay usable
        dialog = self.dialog()
        failing = FakeRecorder("", 1.0)
        failing.fail = "device busy"
        dialog._recorder_factory = lambda device, max_sec: failing
        dialog._btn_record.click()
        self.assertEqual(dialog._status.text(), "Could not open the microphone: device busy")
        self.assertIsNone(dialog._recorder)

    def test_closing_stops_a_running_recording(self):
        dialog = self.dialog()
        dialog._btn_record.click()
        dialog.reject()
        self.assertTrue(self.recorders[0].stopped)
        self.assertFalse(dialog._timer.isActive())


class SettingsFieldTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.app = QtWidgets.QApplication.instance() or QtWidgets.QApplication([])
        from vrclt.qt_ui import MainWindow
        cls.controller = StubController()
        cls.window = MainWindow(cls.controller, config_mod.APPDATA_DIR / "logs" / "vrclt.log")
        cls.form = cls.window._settings_form

    def test_voice_field_round_trips_and_opens_the_dialog(self):
        form = self.form
        field, spec = form._fields["soniox.voice"]
        self.assertIsInstance(field, ui.VoiceField)
        self.assertEqual(field.text(), "Daniel")
        form._fields["soniox.api_key"][0].setText("  draft-key ")
        opened = []

        class Dialog:
            selected = ID_NEW
            deleted = set()

            def __init__(self, tr, **kwargs):
                opened.append(kwargs)

            def exec(self):
                return 1

        with patch.dict(os.environ, {"SONIOX_API_KEY": "env-key"}):
            form._open_soniox_voices(field, dialog_factory=Dialog)
        self.assertEqual(opened[0]["api_key"], "draft-key")   # unsaved key, before the env one
        self.assertEqual((opened[0]["model"], opened[0]["current"]), ("tts-rt-v2", "Daniel"))
        self.assertEqual(field.text(), ID_NEW)
        self.assertEqual(config_mod.get_path(form.config_from_fields(), "soniox.voice"), ID_NEW)

        Dialog.selected, Dialog.deleted = "", {ID_NEW}        # the voice in use was deleted
        form._fields["soniox.api_key"][0].setText("")
        with patch.dict(os.environ, {"SONIOX_API_KEY": "env-key"}):
            form._open_soniox_voices(field, dialog_factory=Dialog)
        self.assertEqual(opened[1]["api_key"], "env-key")
        self.assertEqual(field.text(), "Daniel")
        form._set_field_widget_value(field, spec, ID_A)       # reset menu / config sync path
        self.assertEqual(field.text(), ID_A)
        form._set_field_widget_value(field, spec, "Daniel")


if __name__ == "__main__":
    unittest.main(verbosity=2)
