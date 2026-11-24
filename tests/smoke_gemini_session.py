"""Gemini Live protocol regression tests against a local WebSocket server.

The server plays the BidiGenerateContent endpoint the way the live service
was observed to: `setupComplete` first, JSON in binary frames, close codes
with a reason for a bad key. No credentials or paid requests are required.
Run with .venv/Scripts/python tests/smoke_gemini_session.py.
"""
import asyncio
import base64
import json
import pathlib
import sys
import unittest
from unittest.mock import patch

import websockets

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent.parent))

from vrclt.gemini import session as gs
from vrclt.session_base import FatalSessionError

TRANSLATE = "gemini-3.5-live-translate-preview"
PATH = "/ws/google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContent"


class FakeSource:
    """Speech for a few chunks, then quiet for `quiet` seconds."""

    def __init__(self, chunks: int = 3):
        self.buffer = [b"\x01\x02" * 1600 for _ in range(chunks)]
        self.requeued: list[bytes] = []
        self.trimmed = False
        self.quiet = 0.0

    def drain(self):
        out, self.buffer = self.buffer, []
        return out

    def requeue(self, chunks):
        self.requeued.extend(chunks)

    def active(self, timeout: float = 2.0) -> bool:
        return timeout >= self.quiet

    def trim_to(self, seconds: float) -> None:
        self.trimmed = True


def frame(event: dict) -> bytes:
    return json.dumps(event).encode("utf-8")      # the service sends binary frames


class MockGemini:
    def __init__(self, *behaviors):
        self.behaviors = list(behaviors)          # one per connection, then "talk"
        self.keys, self.paths, self.setups = [], [], []
        self.speech = bytearray()
        self.silence = 0
        self.stream_ends = 0
        self.replied = asyncio.Event()
        self.stream_ended = asyncio.Event()

    async def handler(self, ws):
        self.keys.append(ws.request.headers.get("x-goog-api-key"))
        self.paths.append(ws.request.path)
        first = await ws.recv()
        assert isinstance(first, str), "client frames are JSON text"
        self.setups.append(json.loads(first)["setup"])
        behavior = self.behaviors.pop(0) if self.behaviors else "talk"
        if isinstance(behavior, tuple):           # (close code, reason)
            await ws.close(*behavior)
            return
        await ws.send(frame({"setupComplete": {}}))
        if behavior == "go_away":
            await ws.send(frame({"goAway": {"timeLeft": "5s"}}))
        async for raw in ws:
            message = json.loads(raw)["realtime_input"]
            if message.get("audioStreamEnd"):
                self.stream_ends += 1
                self.stream_ended.set()
                continue
            assert message["audio"]["mimeType"] == "audio/pcm;rate=16000"
            pcm = base64.b64decode(message["audio"]["data"])
            if not any(pcm):
                self.silence += len(pcm)
                continue
            self.speech += pcm
            if self.replied.is_set() or behavior == "mute":
                continue
            for event in (
                {"sessionResumptionUpdate": {"newHandle": "h", "resumable": True}},
                {},
                {"serverContent": {"inputTranscription": {"text": "안녕", "languageCode": "ko"}}},
                {"serverContent": {"outputTranscription": {"text": "hello",
                                                           "languageCode": "en"}}},
                {"serverContent": {"modelTurn": {"parts": [{"inlineData": {
                    "data": base64.b64encode(b"\x11\x22" * 480).decode(),
                    "mimeType": "audio/pcm;rate=24000"}}]}}},
                {"serverContent": {}, "usageMetadata": {"totalTokenCount": 100}},
                {"serverContent": {"interrupted": True}},
                {"serverContent": {"generationComplete": True}},
                {"serverContent": {"turnComplete": True}},
            ):
                await ws.send(frame(event))
            self.replied.set()


def make_session(source=None, sink=None, **overrides):
    sink = sink if sink is not None else {}
    kwargs = dict(
        api_key=" test-gemini-key ", model=TRANSLATE, source=source or FakeSource(),
        name="test", get_target_language=lambda: "en", send_interval_ms=50,
        idle_disconnect_sec=30.0,
        on_src=lambda text, lang: sink.setdefault("src", []).append((text, lang)),
        on_dst=lambda text: sink.setdefault("dst", []).append(text),
        on_audio=lambda pcm: sink.setdefault("audio", []).append(pcm),
        on_turn_complete=lambda: sink.setdefault("turns", []).append(1),
        on_interrupted=lambda: sink.setdefault("interrupted", []).append(1),
        on_session_state=lambda up: sink.setdefault("state", []).append(up))
    kwargs.update(overrides)
    return gs.LiveTranslateSession(**kwargs)


class SetupTests(unittest.TestCase):
    def test_translate_setup_matches_the_wire_format(self):
        model, setup = make_session(echo_target_language=True)._model_and_setup()
        self.assertEqual(model, TRANSLATE)
        self.assertEqual(setup, {"setup": {
            "model": "models/" + TRANSLATE,
            "generationConfig": {
                "responseModalities": ["AUDIO"],
                "translationConfig": {"echo_target_language": True,
                                      "target_language_code": "en"}},
            "inputAudioTranscription": {},
            "outputAudioTranscription": {},
        }})
        # a glossary travels as the system instruction, and only then
        _model, setup = make_session(glossary="VRChat=브이알챗")._model_and_setup()
        instruction = setup["setup"]["systemInstruction"]
        self.assertEqual(instruction["role"], "user")
        self.assertTrue(instruction["parts"][0]["text"].endswith("VRChat=브이알챗"))
        self.assertFalse(
            setup["setup"]["generationConfig"]["translationConfig"]["echo_target_language"])
        # a model already given as a resource name is not prefixed twice
        _model, setup = make_session(model="models/custom")._model_and_setup()
        self.assertEqual(setup["setup"]["model"], "models/custom")

    def test_cantonese_uses_the_interpreter_fallback(self):
        model, setup = make_session(get_target_language=lambda: "yue",
                                    glossary="A=B")._model_and_setup()
        self.assertEqual(model, gs.AGENT_MODEL)
        body = setup["setup"]
        self.assertEqual(body["model"], "models/" + gs.AGENT_MODEL)
        self.assertEqual(body["generationConfig"], {"responseModalities": ["AUDIO"]})
        text = body["systemInstruction"]["parts"][0]["text"]
        self.assertIn("into Cantonese", text)
        self.assertTrue(text.endswith("A=B"))

    def test_error_classifiers_read_close_codes_and_reasons(self):
        bad_key = gs._ServerClosed(1007, "API key not valid. Please pass a valid API key.")
        self.assertTrue(gs._is_invalid_api_key_error(bad_key))
        self.assertFalse(gs._is_invalid_api_key_error(gs._ServerClosed(1008, "model not found")))
        for reason in ("Invalid value at 'setup.system_instruction'",
                       "systemInstruction is not supported for this model"):
            self.assertTrue(gs._looks_like_glossary_rejection(gs._ServerClosed(1007, reason)))
        self.assertFalse(gs._looks_like_glossary_rejection(gs._ServerClosed(1011, "internal")))
        self.assertEqual(gs._classify_session_error(
            gs._ServerClosed(1011, "Resource exhausted. Please try again later.")), "quota")
        self.assertEqual(gs._classify_session_error(gs._ServerClosed(429, "")), "quota")
        self.assertEqual(gs._classify_session_error(gs._ServerClosed(1006, "connection lost")),
                         "network")


class WireTests(unittest.IsolatedAsyncioTestCase):
    async def serve(self, server):
        ws_server = await websockets.serve(server.handler, "127.0.0.1", 0)
        self.addAsyncCleanup(self._close, ws_server)
        port = ws_server.sockets[0].getsockname()[1]
        for name, value in (("LIVE_URL", f"ws://127.0.0.1:{port}{PATH}"),
                            ("RECONNECT_MIN_BACKOFF", 0.05)):
            patcher = patch.object(gs, name, value)
            patcher.start()
            self.addCleanup(patcher.stop)
        return server

    @staticmethod
    async def _close(ws_server):
        ws_server.close()
        await ws_server.wait_closed()

    async def run_until(self, session, event, settle=0.3):
        stop = asyncio.Event()
        task = asyncio.ensure_future(session.run(stop))
        await asyncio.wait_for(event.wait(), timeout=10.0)
        await asyncio.sleep(settle)
        stop.set()
        await asyncio.wait_for(task, timeout=10.0)

    async def test_translate_session_streams_both_ways_and_closes_cleanly(self):
        server = await self.serve(MockGemini())
        source, sink = FakeSource(), {}
        session = make_session(source, sink)
        await self.run_until(session, server.replied)
        self.assertEqual(server.keys, ["test-gemini-key"])          # header, trimmed
        self.assertEqual(server.paths, [PATH])                      # and never in the URL
        self.assertEqual(server.setups[0]["model"], "models/" + TRANSLATE)
        self.assertEqual(bytes(server.speech), b"\x01\x02" * 1600 * 3)
        self.assertEqual(sink["src"], [("안녕", "ko")])
        self.assertEqual(sink["dst"], ["hello"])
        self.assertEqual(sink["audio"], [b"\x11\x22" * 480])
        self.assertEqual((len(sink["turns"]), len(sink["interrupted"])), (1, 1))
        self.assertEqual(sink["state"], [True, False])
        self.assertTrue(source.trimmed)
        self.assertFalse(session.connected)
        self.assertEqual((session.last_error, session.error_class), ("", ""))

    async def test_invalid_key_is_fatal(self):
        await self.serve(MockGemini(
            (1007, "API key not valid. Please pass a valid API key.")))
        session = make_session()
        with self.assertRaises(FatalSessionError) as fatal:
            await asyncio.wait_for(session.run(asyncio.Event()), timeout=10.0)
        self.assertIn("API key is invalid", str(fatal.exception))
        self.assertFalse(session.connected)

    async def test_rejected_glossary_is_dropped_and_the_session_recovers(self):
        server = await self.serve(MockGemini(
            (1007, "Invalid value at 'setup.system_instruction' for this model")))
        session = make_session(glossary="VRChat=브이알챗")
        with self.assertLogs("vrclt.gemini.session", level="WARNING"):
            await self.run_until(session, server.replied)
        self.assertIn("systemInstruction", server.setups[0])
        self.assertNotIn("systemInstruction", server.setups[1])
        self.assertTrue(session._glossary_disabled)
        self.assertEqual(session.last_error, "")                    # cleared on reconnect

    async def test_go_away_reconnects_without_an_error(self):
        server = await self.serve(MockGemini("go_away"))
        source = FakeSource(chunks=0)
        session = make_session(source)
        stop = asyncio.Event()
        task = asyncio.ensure_future(session.run(stop))
        for _ in range(100):
            if len(server.setups) >= 2:
                break
            await asyncio.sleep(0.05)
        self.assertGreaterEqual(len(server.setups), 2)
        self.assertEqual((session.last_error, session.error_class), ("", ""))
        stop.set()
        await asyncio.wait_for(task, timeout=10.0)

    async def test_quota_close_is_reported_then_retried(self):
        server = await self.serve(MockGemini(
            (1011, "Resource exhausted. Please try again later.")))
        seen, waits = [], []
        session = make_session(on_session_state=lambda up: seen.append(
            (up, session.error_class)))
        real_sleep = gs.sleep_interruptible

        async def backoff(duration, stop):          # what the status UI shows meanwhile
            waits.append((session.error_class, session.last_error))
            await real_sleep(duration, stop)

        with self.assertLogs("vrclt.gemini.session", level="ERROR"), \
                patch.object(gs, "sleep_interruptible", backoff):
            await self.run_until(session, server.replied)
        self.assertEqual(waits, [("quota", "1011 Resource exhausted. Please try again later.")])
        self.assertEqual(len(server.setups), 2)
        self.assertEqual(seen[0], (True, ""))       # the retry connected and cleared it

    async def test_a_pause_is_rebuilt_with_silence_then_the_stream_is_ended(self):
        server = await self.serve(MockGemini("mute"))       # never ends the turn itself
        source = FakeSource()
        session = make_session(source)
        with patch.object(gs, "GAP_FILL_GRACE_SEC", 0.0), \
                patch.object(gs, "GAP_FILL_MAX_SEC", 0.2):
            stop = asyncio.Event()
            task = asyncio.ensure_future(session.run(stop))
            for _ in range(100):
                if server.speech:
                    break
                await asyncio.sleep(0.02)
            source.quiet = 1.0      # past the turn-end silence, inside the idle limit
            await asyncio.wait_for(server.stream_ended.wait(), timeout=10.0)
            stop.set()
            await asyncio.wait_for(task, timeout=10.0)
        self.assertGreater(server.silence, 0)
        self.assertEqual(server.stream_ends, 1)

    async def test_unconfirmed_setup_is_an_error_not_a_session(self):
        class Silent(MockGemini):
            async def handler(self, ws):
                self.setups.append(json.loads(await ws.recv())["setup"])
                await ws.send(frame({"serverContent": {}}))   # anything but setupComplete
                await ws.wait_closed()

        server = await self.serve(Silent())
        session = make_session()
        stop = asyncio.Event()
        with self.assertLogs("vrclt.gemini.session", level="ERROR"):
            task = asyncio.ensure_future(session.run(stop))
            for _ in range(100):
                if session.last_error:
                    break
                await asyncio.sleep(0.05)
            stop.set()
            await asyncio.wait_for(task, timeout=10.0)
        self.assertIn("did not confirm", session.last_error)
        self.assertEqual(session.error_class, "network")
        self.assertGreaterEqual(len(server.setups), 1)


if __name__ == "__main__":
    unittest.main(verbosity=2)
