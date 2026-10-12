"""Qwen LiveTranslate protocol regression tests (qwen3.8 and qwen3.5).

A local websockets server plays the DashScope realtime endpoint, so no
credentials, paid requests or audio devices are needed.
Run with .venv/Scripts/python tests/smoke_qwen_session.py.
"""
import asyncio
import base64
import copy
import json
import pathlib
import sys
import time
import unittest
from unittest.mock import patch

import websockets

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent.parent))

from vrclt import config
from vrclt.pipeline import InboundPipeline, Segmenter, SpeakerSegmenter
from vrclt.qwen import session as qs
from vrclt.state import AppState
from vrclt.subtitles import SubtitleLine, SubtitleStore

QWEN38 = "qwen3.8-livetranslate-flash-realtime"
QWEN35 = "qwen3.5-livetranslate-flash-realtime"


class FakeSource:
    def __init__(self):
        self.buffer = [b"\x01\x02" * 800]
        self.requeued = []
        self.trimmed = []

    def drain(self):
        result, self.buffer = self.buffer, []
        return result

    def requeue(self, chunks):
        self.requeued.extend(chunks)

    def trim_to(self, seconds):
        self.trimmed.append(seconds)

    def active(self, timeout=2.0):
        return True


def make_session(model=QWEN38, **overrides):
    kwargs = dict(api_key="sk-test", model=model, source=FakeSource(), name="test",
                  get_target_language=lambda: "ko", send_interval_ms=50)
    kwargs.update(overrides)
    return qs.QwenLiveTranslateSession(**kwargs)


class Events:
    """Async-iterable stand-in for the websocket the receiver reads."""

    def __init__(self, events):
        self._events = [json.dumps(e) for e in events]

    def __aiter__(self):
        return self._iter()

    async def _iter(self):
        for raw in self._events:
            yield raw


class QwenConfigTests(unittest.TestCase):
    def test_model_detection_and_engine_capabilities(self):
        self.assertTrue(config.is_qwen38_model(QWEN38))
        self.assertTrue(config.is_qwen38_model("qwen3.10-livetranslate-flash-realtime"))
        self.assertFalse(config.is_qwen38_model(QWEN35))
        self.assertFalse(config.is_qwen38_model(QWEN35 + "-2026-05-19"))
        self.assertEqual(config.DEFAULTS["qwen"]["model"], QWEN38)
        cfg = copy.deepcopy(config.DEFAULTS)
        cfg["provider"] = "qwen"
        self.assertTrue(config.has_speaker_ids(cfg))
        self.assertFalse(config.uses_source_language(cfg))
        cfg["qwen"]["model"] = QWEN35
        self.assertFalse(config.has_speaker_ids(cfg))
        self.assertTrue(config.uses_source_language(cfg))
        cfg["qwen"]["model"] = ""   # blank falls back to the default model
        self.assertTrue(config.has_speaker_ids(cfg))
        cfg["provider"] = "soniox"
        self.assertTrue(config.has_speaker_ids(cfg))
        self.assertTrue(config.uses_source_language(cfg))
        cfg["provider"] = "gemini"
        self.assertFalse(config.has_speaker_ids(cfg))
        self.assertFalse(config.uses_source_language(cfg))

    def test_qwen38_session_update_shape(self):
        session = make_session(glossary="AI=인공지능", voice_clone="once")
        cfg = session._session_config("ko", "", audio_out=True)
        self.assertEqual(cfg, {
            "output_modalities": ["text", "audio"],
            "audio": {"input": {"turn_detection": {"type": "speaker_detection"}}},
            "translation": {"language": "ko", "corpus": {"phrases": {"AI": "인공지능"}}},
            "voice": "default",
            "enable_voice_clone": True,
            "voice_clone_options": {"frequency": "once"},
        })
        text_only = make_session(voice_clone="off")._session_config("uk", "", audio_out=False)
        self.assertEqual(text_only, {
            "output_modalities": ["text"],
            "audio": {"input": {"turn_detection": {"type": "speaker_detection"}}},
            "translation": {"language": "uk"},
        })

    def test_qwen35_session_update_is_unchanged(self):
        cfg = make_session(QWEN35, voice_clone="off", voice="Cherry")._session_config(
            "ko", "en", audio_out=True)
        self.assertEqual(cfg, {
            "modalities": ["text", "audio"],
            "input_audio_format": "pcm",
            "translation": {"language": "ko"},
            "input_audio_transcription": {"model": qs.ASR_MODEL, "language": "en"},
            "output_audio_format": "pcm",
            "voice": "Cherry",
            "enable_voice_clone": True,
            "voice_clone_options": {"frequency": "never"},
        })


class QwenReceiverTests(unittest.IsolatedAsyncioTestCase):
    async def test_qwen38_items_carry_speakers_through_interleaved_turns(self):
        src, dst, turns = [], [], []
        session = make_session(
            on_src=lambda text, lang, speaker: src.append((text, lang, speaker)),
            on_dst=lambda text, speaker: dst.append((text, speaker)),
            on_turn_complete=lambda: turns.append(len(dst)),
            speaker_metadata=True)
        await session._receiver(Events([
            {"type": "input_audio_buffer.speech_started", "item_id": "s1", "speaker_id": 0},
            {"type": "conversation.item.input_audio_transcription.delta",
             "item_id": "s1", "delta": "ha"},
            # deltas are incremental: an identical repeat is new text
            {"type": "conversation.item.input_audio_transcription.delta",
             "item_id": "s1", "delta": "ha"},
            {"type": "conversation.item.created", "previous_item_id": "s1",
             "item": {"id": "t1", "role": "assistant", "content": []}},
            {"type": "input_audio_buffer.speech_started", "item_id": "s2", "speaker_id": 1},
            {"type": "conversation.item.input_audio_transcription.delta",
             "item_id": "s2", "delta": "Bye"},
            {"type": "response.audio_transcript.delta", "item_id": "t1", "delta": "하하"},
            {"type": "conversation.item.input_audio_transcription.completed",
             "item_id": "s1", "transcript": "haha", "language": "en"},
            {"type": "response.audio_transcript.done", "item_id": "t1",
             "transcript": "하하!"},
            # s2 is still being transcribed: ending the turn would split it
            {"type": "response.done", "response": {"status": "completed"}},
            {"type": "conversation.item.input_audio_transcription.completed",
             "item_id": "s2", "transcript": "Bye.", "language": "en"},
            {"type": "conversation.item.created", "previous_item_id": "s2",
             "item": {"id": "t2", "role": "assistant", "content": []}},
            {"type": "response.text.delta", "item_id": "t2", "delta": "잘 가"},
            # a final without item_id cannot be matched: never re-emit it
            {"type": "response.text.done", "text": "잘 가"},
            {"type": "response.done", "response": {"status": "completed"}},
            {"type": "session.finished"},
        ]))
        self.assertEqual(src, [("ha", None, "0"), ("ha", None, "0"),
                               ("Bye", None, "1"), (".", "en", "1")])
        self.assertEqual(dst, [("하하", "0"), ("!", "0"), ("잘 가", "1")])
        self.assertEqual(turns, [3])
        self.assertTrue(session._finished.is_set())

    async def test_lost_completed_event_stops_holding_the_turn(self):
        turns = []
        session = make_session(on_turn_complete=lambda: turns.append(True))
        await session._receiver(Events([
            {"type": "conversation.item.input_audio_transcription.delta",
             "item_id": "s1", "delta": "never completed"},
        ]))
        await session._receiver(Events([{"type": "response.done"}]))
        self.assertEqual(turns, [])
        session._src_open["s1"] = time.monotonic() - qs.SOURCE_IN_FLIGHT_SEC - 1
        await session._receiver(Events([{"type": "response.done"}]))
        self.assertEqual(turns, [True])

    async def test_qwen35_text_events_keep_legacy_handling(self):
        src, dst, turns = [], [], []
        session = make_session(QWEN35, get_source_language=lambda: "en",
                               on_src=lambda text, lang: src.append((text, lang)),
                               on_dst=dst.append,
                               on_turn_complete=lambda: turns.append(True))
        session._src_display_lang = "en"
        await session._receiver(Events([
            {"type": "conversation.item.input_audio_transcription.text",
             "item_id": "s1", "text": "Hello", "stash": " there"},
            {"type": "conversation.item.input_audio_transcription.completed",
             "item_id": "s1", "transcript": "Hello there"},
            {"type": "response.audio_transcript.text", "item_id": "t1", "text": "안녕"},
            {"type": "response.audio_transcript.done", "transcript": "안녕하세요"},
            {"type": "response.done"},
        ]))
        self.assertEqual(src, [("Hello", "en"), (" there", "en")])
        self.assertEqual("".join(dst), "안녕하세요")
        self.assertEqual(turns, [True])


class QwenWireTests(unittest.IsolatedAsyncioTestCase):
    async def test_qwen38_session_over_the_wire(self):
        seen = {}
        audio_in = []
        src, dst, audio = [], [], []
        got_dst = asyncio.Event()

        async def server(ws):
            seen["auth"] = ws.request.headers.get("Authorization")
            seen["path"] = ws.request.path
            await ws.send(json.dumps({"type": "session.created", "session": {}}))
            async for raw in ws:
                event = json.loads(raw)
                if event["type"] == "session.update":
                    seen["session"] = event["session"]
                    await ws.send(json.dumps({"type": "session.updated", "session": {}}))
                elif event["type"] == "input_audio_buffer.append":
                    pcm = base64.b64decode(event["audio"])
                    if any(pcm) and not audio_in:  # skip the zeroed pause padding
                        audio_in.append(pcm)
                        for reply in (
                            {"type": "input_audio_buffer.speech_started",
                             "item_id": "s1", "speaker_id": 2},
                            {"type": "conversation.item.input_audio_transcription.delta",
                             "item_id": "s1", "delta": "Hello"},
                            {"type": "conversation.item.created", "previous_item_id": "s1",
                             "item": {"id": "t1", "role": "assistant", "content": []}},
                            {"type": "response.audio_transcript.delta",
                             "item_id": "t1", "delta": "안녕"},
                            {"type": "response.audio.delta", "item_id": "t1",
                             "delta": base64.b64encode(b"\x11\x22" * 240).decode()},
                            {"type": "conversation.item.input_audio_transcription.completed",
                             "item_id": "s1", "transcript": "Hello", "language": "en"},
                            {"type": "response.audio_transcript.done",
                             "item_id": "t1", "transcript": "안녕"},
                            {"type": "response.done"},
                        ):
                            await ws.send(json.dumps(reply))
                elif event["type"] == "session.finish":
                    seen["finished"] = True
                    await ws.send(json.dumps({"type": "session.finished"}))
                    return

        stop = asyncio.Event()
        async with websockets.serve(server, "127.0.0.1", 0) as ws_server:
            port = ws_server.sockets[0].getsockname()[1]
            session = make_session(
                base_url=f"ws://127.0.0.1:{port}/api-ws/v1/realtime",
                get_source_language=lambda: "ja",   # ignored: 3.8 detects it
                on_src=lambda text, lang, speaker: src.append((text, lang, speaker)),
                on_dst=lambda text, speaker: (dst.append((text, speaker)), got_dst.set()),
                on_audio=audio.append, speaker_metadata=True)
            task = asyncio.create_task(session.run(stop))
            try:
                await asyncio.wait_for(got_dst.wait(), 5)
                await asyncio.sleep(0.1)
            finally:
                stop.set()
                await asyncio.wait_for(task, 5)
        self.assertEqual(seen["auth"], "Bearer sk-test")
        self.assertIn(f"model={QWEN38}", seen["path"])
        self.assertEqual(seen["session"]["output_modalities"], ["text", "audio"])
        self.assertNotIn("input_audio_transcription", seen["session"])
        self.assertNotIn("modalities", seen["session"])
        self.assertEqual(audio_in, [b"\x01\x02" * 800])
        self.assertEqual(src, [("Hello", None, "2")])
        self.assertEqual(dst, [("안녕", "2")])
        self.assertEqual(b"".join(audio), b"\x11\x22" * 240)
        self.assertTrue(seen.get("finished"))
        self.assertFalse(session.connected)
        self.assertFalse(session.last_error)


class QwenPipelineTests(unittest.TestCase):
    def test_inbound_pipeline_uses_speaker_turns_only_for_qwen38(self):
        cfg = copy.deepcopy(config.DEFAULTS)
        cfg["provider"] = "qwen"
        state, store = AppState(), SubtitleStore()
        with patch("vrclt.pipeline.GameAudioTap", return_value=FakeSource()):
            pipeline = InboundPipeline(cfg, "sk-test", store, state)
        try:
            self.assertIsInstance(pipeline.segmenter, SpeakerSegmenter)
            session = pipeline.session
            self.assertIsInstance(session, qs.QwenLiveTranslateSession)
            session._speaker_of.update({"s1": "0", "s2": "1"})
            session._source_of.update({"t1": "s1", "t2": "s2"})
            session._handle_item_event(
                "conversation.item.input_audio_transcription.delta",
                {"item_id": "s1", "delta": "hello"})
            session._handle_item_event(
                "conversation.item.input_audio_transcription.delta",
                {"item_id": "s2", "delta": "goodbye"})
            session._handle_item_event("response.text.delta",
                                       {"item_id": "t1", "delta": "안녕"})
            session._handle_item_event("response.text.delta",
                                       {"item_id": "t2", "delta": "잘 가"})
            pipeline.segmenter.turn_complete()
            finals, _partial = store.snapshot_with_speakers()
            self.assertEqual(finals, [SubtitleLine("hello", "안녕", "auto", "0"),
                                      SubtitleLine("goodbye", "잘 가", "auto", "1")])
        finally:
            pipeline.detach()
        cfg["qwen"]["model"] = QWEN35
        with patch("vrclt.pipeline.GameAudioTap", return_value=FakeSource()):
            pipeline = InboundPipeline(cfg, "sk-test", store, state)
        try:
            self.assertIs(type(pipeline.segmenter), Segmenter)
            self.assertFalse(pipeline.session._speaker_metadata)
        finally:
            pipeline.detach()


if __name__ == "__main__":
    unittest.main(verbosity=2)
