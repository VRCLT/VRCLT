"""Gemini Live translate session with auto-reconnect.

Protocol facts this code is built around (Gemini Live API over a raw
WebSocket; the frames match what google-genai 2.8.0 put on the wire, which
this module used before, and were re-checked against the live service):
- BidiGenerateContent endpoint, API key in the x-goog-api-key upgrade header.
  The first client frame is `setup`; the server answers `setupComplete`
  before anything else.
- A bad key or model is not an HTTP error: the upgrade succeeds and the server
  closes with 1007/1008 and a reason text.
- Server frames are JSON in *binary* frames. Client frames are JSON text; the
  audio goes in as base64 PCM under realtime_input.
- A normal close (1000/1001) ends iteration quietly, anything else raises ->
  a _closing flag distinguishes intentional closes from genuine failures.
- Turn boundaries come from the server's automatic VAD. The client-side gates
  (mic RMS gate, Silero VAD) strip nearly all silence from the stream, so that
  VAD never hears a pause. Cutting the turn instead with audio_stream_end ends
  generation mid-phrase: the model is interpreting simultaneously and is always
  a few words behind, so the tail was stranded until the next utterance
  ("last word never translated"). The sender therefore rebuilds the pauses the
  gates swallowed - zeroed PCM streamed in real time until the server closes
  the turn itself - and only falls back to audio_stream_end if it never does.
"""
import asyncio
import base64
import json
import logging
import time

import websockets

from ..session_base import (AudioSource, FatalSessionError,
                            RECONNECT_MIN_BACKOFF, RECONNECT_MAX_BACKOFF,
                            sleep_interruptible)

log = logging.getLogger(__name__)
# The upgrade request carries the API key. Do not let the websockets DEBUG
# handshake logger expose it even when the application's level is DEBUG.
_wire_log = logging.getLogger("vrclt.gemini.wire")
_wire_log.setLevel(logging.WARNING)

LIVE_URL = ("wss://generativelanguage.googleapis.com/ws/"
            "google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContent")
SETUP_TIMEOUT_SEC = 10.0
AUDIO_MIME = "audio/pcm;rate=16000"
STREAM_END = json.dumps({"realtime_input": {"audioStreamEnd": True}})

# languages the dedicated translate model does NOT support; for these we fall
# back to a conversational live model with an interpreter system instruction
# (works, but speaks with a stock voice instead of replicating the speaker)
AGENT_FALLBACK_LANGUAGES = {
    "yue": "Cantonese",
}
AGENT_MODEL = "gemini-3.8-live"
AGENT_INSTRUCTION = (
    "You are a professional simultaneous interpreter. Translate everything "
    "you hear into {language}. Speak ONLY the translation - never answer "
    "questions, never add commentary. Keep the original meaning and tone."
)

INPUT_RATE = 16000  # the AudioSource contract: 16 kHz mono int16
# pause reconstruction for the server VAD: once real audio stopped flowing,
# wait GRACE seconds (drain jitter, not a pause yet), then stream zeroed PCM at
# the send cadence until the server ends the turn - or MAX seconds pass, which
# means its VAD never fired and the turn has to be closed explicitly.
GAP_FILL_GRACE_SEC = 0.25
GAP_FILL_MAX_SEC = 2.5


class _ServerClosed(RuntimeError):
    """The service refused or dropped the session: a WebSocket close code (or
    the HTTP status of a refused upgrade) with the server's reason text."""

    def __init__(self, code, reason: str):
        self.code = code
        super().__init__(f"{code} {reason}".strip())


def _closed_error(exc: websockets.ConnectionClosed) -> _ServerClosed:
    frame = exc.rcvd
    if frame is None:
        return _ServerClosed(1006, "connection lost")
    return _ServerClosed(frame.code, frame.reason or "")


def _decode(raw) -> dict:
    event = json.loads(raw)
    return event if isinstance(event, dict) else {}


def _audio_message(pcm: bytes) -> str:
    return json.dumps({"realtime_input": {"audio": {
        "data": base64.b64encode(pcm).decode("ascii"), "mimeType": AUDIO_MIME}}})


def _is_invalid_api_key_error(exc: Exception) -> bool:
    text = str(exc).lower()
    code = getattr(exc, "code", None) or getattr(exc, "status_code", None)
    return (str(code) == "1007" and "api key" in text) or "api key not valid" in text


def _looks_like_glossary_rejection(exc: Exception) -> bool:
    # the reason names the field as system_instruction or systemInstruction
    text = str(exc).lower().replace("_", "")
    code = str(getattr(exc, "code", None) or getattr(exc, "status_code", None))
    return code in ("400", "1007", "1008") and "systeminstruction" in text


def _classify_session_error(exc: Exception) -> str:
    """Short reason class for UI display: 'quota' | 'network'."""
    text = str(exc).lower()
    code = getattr(exc, "code", None) or getattr(exc, "status_code", None)
    if str(code) == "429" or "resource_exhausted" in text or "resource exhausted" in text \
            or "quota" in text or "rate limit" in text:
        return "quota"
    return "network"


class LiveTranslateSession:
    def __init__(self, *, api_key: str, model: str, source: AudioSource, name: str,
                 get_target_language, echo_target_language: bool = False,
                 enabled=lambda: True,
                 send_interval_ms: int = 100, idle_disconnect_sec: float = 15.0,
                 turn_end_silence_sec: float = 0.55,
                 glossary: str = "",
                 on_src=None, on_dst=None, on_audio=None, on_turn_complete=None,
                 on_interrupted=None, on_session_state=None):
        self._api_key = str(api_key or "").strip()
        self._model = model
        self._get_target = get_target_language
        self._echo = echo_target_language
        self._enabled = enabled
        self._source = source
        self.name = name
        self._interval = max(0.05, send_interval_ms / 1000.0)
        self._idle_disconnect = idle_disconnect_sec
        self._turn_end_silence = max(self._interval, float(turn_end_silence_sec))
        self._on_src = on_src
        self._on_dst = on_dst
        self._on_audio = on_audio
        self._on_turn_complete = on_turn_complete
        self._on_interrupted = on_interrupted
        self._on_session_state = on_session_state
        glossary = str(glossary or "").strip()
        if len(glossary) > 2000:
            log.warning("[%s] glossary truncated to 2000 chars", name)
            glossary = glossary[:2000]
        self._glossary = glossary
        self._glossary_disabled = False
        self.connected = False
        # last-failure diagnostics for the status UIs (written on the session
        # asyncio thread; read lock-free from the Qt/VR threads)
        self.last_error = ""
        self.error_class = ""   # "" | "quota" | "network"
        self.next_retry_at = 0.0
        self._closing = False
        self._restart = False
        # True between the first audio of a turn and the server's turn_complete;
        # the sender stops padding silence as soon as the turn closes on its own
        self._turn_open = False
        # diagnostics (logged every 15s by the watchdog while connected)
        self._st_sent = 0
        self._st_sent_bytes = 0
        self._st_recv = 0
        self._st_src = 0
        self._st_dst = 0

    def request_restart(self) -> None:
        """Apply changed settings (e.g. target language) by reconnecting."""
        self._restart = True

    def _glossary_instruction(self) -> str:
        if not self._glossary or self._glossary_disabled:
            return ""
        return ("Glossary: always translate the following terms exactly as "
                "specified (source=target), overriding your default wording:\n"
                + self._glossary)

    def _model_and_setup(self) -> tuple[str, dict]:
        """The model and the `setup` frame that opens a session for it."""
        target = self._get_target()
        instr = self._glossary_instruction()
        generation: dict = {"responseModalities": ["AUDIO"]}
        if target in AGENT_FALLBACK_LANGUAGES:
            model = AGENT_MODEL
            system = AGENT_INSTRUCTION.format(
                language=AGENT_FALLBACK_LANGUAGES[target])
            if instr:
                system += "\n\n" + instr
        else:
            model = self._model
            generation["translationConfig"] = {
                "echo_target_language": bool(self._echo),
                "target_language_code": target,
            }
            # only attached when a glossary is set, so the empty-glossary
            # setup stays identical to the pre-glossary behavior
            system = instr
        setup: dict = {
            "model": model if "/" in model else f"models/{model}",
            "generationConfig": generation,
        }
        if system:
            setup["systemInstruction"] = {"parts": [{"text": system}], "role": "user"}
        setup["inputAudioTranscription"] = {}
        setup["outputAudioTranscription"] = {}
        return model, {"setup": setup}

    async def run(self, stop: asyncio.Event) -> None:
        """Supervisor: wait for voice, run sessions, reconnect with backoff."""
        backoff = RECONNECT_MIN_BACKOFF
        waiting_logged = False
        while not stop.is_set():
            if not (self._enabled() and self._source.active()):
                if not waiting_logged:
                    waiting_logged = True
                    log.info("[%s] idle (%s) - session closed",
                             self.name, "disabled" if not self._enabled() else "no voice")
                await asyncio.sleep(0.2)
                continue
            if waiting_logged:
                waiting_logged = False
                log.info("[%s] voice detected - connecting", self.name)

            # drop stale buffered silence; keep ~1s pre-roll of speech onset
            self._source.trim_to(1.0)
            self._restart = False
            try:
                clean = await self._session_once(stop)
            except Exception as e:
                if _is_invalid_api_key_error(e):
                    raise FatalSessionError(
                        "Gemini API key is invalid. Update the API key in Settings."
                    ) from e
                clean = False
                self.error_class = _classify_session_error(e)
                self.last_error = str(e)[:200]
                if self._glossary and not self._glossary_disabled \
                        and _looks_like_glossary_rejection(e):
                    # self-heal: never loop forever on a server that rejects
                    # system_instruction for the translate model
                    self._glossary_disabled = True
                    log.warning("[%s] model rejected the glossary "
                                "system_instruction - continuing without it",
                                self.name)
                log.exception("[%s] session error", self.name)
            if stop.is_set():
                break
            if clean:
                backoff = RECONNECT_MIN_BACKOFF
                await asyncio.sleep(0.2)
            else:
                log.info("[%s] reconnecting in %.0fs", self.name, backoff)
                self.next_retry_at = time.time() + backoff
                await sleep_interruptible(backoff, stop)
                backoff = min(backoff * 2, RECONNECT_MAX_BACKOFF)

    async def _session_once(self, stop: asyncio.Event) -> bool:
        """One Live session. Returns True if it ended cleanly (goAway/idle/stop)."""
        target = self._get_target()
        model, setup = self._model_and_setup()
        log.info("[%s] connecting (model=%s target=%s echo=%s)",
                 self.name, model, target, self._echo)
        self._closing = False
        try:
            return await self._connected_session(setup, target, stop)
        except websockets.exceptions.InvalidStatus as e:
            # a refused upgrade: keep the status for the error classifiers
            body = (e.response.body or b"")[:300].decode("utf-8", errors="replace")
            raise _ServerClosed(e.response.status_code, body) from None
        except websockets.ConnectionClosed as e:
            code = e.rcvd.code if e.rcvd else None
            if self._closing or code in (1000, 1001):
                log.info("[%s] connection closed cleanly (code=%s)", self.name, code)
                return True
            raise _closed_error(e) from None

    async def _connected_session(self, setup: dict, target: str,
                                 stop: asyncio.Event) -> bool:
        async with websockets.connect(
            LIVE_URL, additional_headers={"x-goog-api-key": self._api_key},
            open_timeout=10, close_timeout=2, max_size=2**24, logger=_wire_log,
        ) as session:
            await session.send(json.dumps(setup, ensure_ascii=False))
            first = _decode(await asyncio.wait_for(session.recv(), SETUP_TIMEOUT_SEC))
            if "setupComplete" not in first:
                raise RuntimeError("Gemini did not confirm the session setup.")
            self._turn_open = False
            self.connected = True
            # clear on successful connect (not on retry start) so the status
            # UI doesn't flicker between backoff sleeps and connect attempts
            self.last_error = ""
            self.error_class = ""
            self.next_retry_at = 0.0
            if self._on_session_state:
                self._on_session_state(True)
            log.info("[%s] session started (target=%s)", self.name, target)
            recv_t = asyncio.ensure_future(self._receiver(session))
            send_t = asyncio.ensure_future(self._sender(session))
            watch_t = asyncio.ensure_future(self._watchdog(session, stop))
            tasks = {recv_t, send_t, watch_t}
            try:
                done, pending = await asyncio.wait(tasks, return_when=asyncio.FIRST_COMPLETED)
                for t in pending:
                    t.cancel()
                # tasks must finish before the websocket closes under them, and
                # cancelled tasks must be awaited so exceptions are retrieved
                await asyncio.gather(*pending, return_exceptions=True)
                error = None
                for t in done:
                    if t.cancelled():
                        continue
                    exc = t.exception()
                    if exc is not None and error is None:
                        error = exc
                if error is not None:
                    if self._closing:
                        return True
                    raise error   # a ConnectionClosed is sorted out by the caller
                return True  # receiver returned (goAway/close) or watchdog closed intentionally
            finally:
                self.connected = False
                if self._on_session_state:
                    self._on_session_state(False)
                log.info("[%s] session ended", self.name)

    async def _receiver(self, session) -> None:
        # iteration ends by itself on a normal close and raises on any other
        async for raw in session:
            event = _decode(raw)
            self._st_recv += 1
            sc = event.get("serverContent")
            if isinstance(sc, dict):
                it = sc.get("inputTranscription") or {}
                if it.get("text"):
                    self._st_src += 1
                    if self._on_src:
                        self._on_src(it["text"], it.get("languageCode"))
                ot = sc.get("outputTranscription") or {}
                if ot.get("text"):
                    self._st_dst += 1
                    if self._on_dst:
                        self._on_dst(ot["text"])
                if self._on_audio:
                    for part in (sc.get("modelTurn") or {}).get("parts") or []:
                        data = (part.get("inlineData") or {}).get("data")
                        if data:
                            self._on_audio(base64.b64decode(data))
                if sc.get("interrupted"):
                    log.info("[%s] interrupted (barge-in)", self.name)
                    if self._on_interrupted:
                        self._on_interrupted()
                if sc.get("turnComplete"):
                    self._turn_open = False
                    if self._on_turn_complete:
                        self._on_turn_complete()
            if event.get("goAway") is not None:
                log.info("[%s] goAway (time_left=%s) - reconnecting",
                         self.name, event["goAway"].get("timeLeft"))
                return

    async def _sender(self, session) -> None:
        speaking = False
        last_audio = 0.0   # when real audio last went out
        gap_filled = 0.0   # zeroed seconds streamed into the current pause
        fill_message = _audio_message(b"\x00" * (int(INPUT_RATE * self._interval) * 2))
        # the connect pre-roll must be flushed even when the handshake took
        # longer than turn_end_silence (active() already false) - otherwise a
        # short utterance that triggered this very connection is dropped
        flush_preroll = True
        while True:
            await asyncio.sleep(self._interval)
            if self._restart or not self._enabled():
                if speaking:
                    try:
                        await session.send(STREAM_END)
                    except Exception:
                        pass
                return
            chunks = self._source.drain()
            if chunks and (flush_preroll or self._source.active(self._turn_end_silence)):
                flush_preroll = False
                pcm = b"".join(chunks)
                sent = False
                try:
                    await session.send(_audio_message(pcm))
                    sent = True
                    self._st_sent += 1
                    self._st_sent_bytes += len(pcm)
                finally:
                    if not sent:
                        # connection died or task cancelled mid-send:
                        # requeue so the next session resends
                        self._source.requeue(chunks)
                speaking = True
                self._turn_open = True
                last_audio = time.time()
                gap_filled = 0.0
            elif speaking:
                # The source went quiet (chunks drained on these ticks are
                # sub-gate hangover bridge audio - dropped). The gates stripped
                # the pause the server VAD needs, so rebuild it with zeroed PCM
                # at the send cadence: the model hears a natural pause, finishes
                # the phrase it is still interpreting and ends the turn itself.
                if not self._turn_open:
                    speaking = False  # server already closed it - stop padding
                    continue
                if (time.time() - last_audio) < GAP_FILL_GRACE_SEC:
                    continue  # drain jitter, not a real pause yet
                if gap_filled >= GAP_FILL_MAX_SEC:
                    # the VAD never fired on the reconstructed silence; close
                    # the turn explicitly rather than let the translation hang
                    # until the session does (which stranded the sentence tail)
                    speaking = False
                    self._turn_open = False
                    try:
                        await session.send(STREAM_END)
                    except Exception:
                        return
                    continue
                try:
                    await session.send(fill_message)
                except Exception:
                    return
                gap_filled += self._interval

    async def _watchdog(self, session, stop: asyncio.Event) -> None:
        last_stats = time.time()
        while True:
            await asyncio.sleep(0.2)
            if (time.time() - last_stats) >= 15.0:
                last_stats = time.time()
                qlen = len(getattr(self._source, "buffer", ()))
                log.info("[%s] stats(15s): sent=%d msg/%.0fKB recv=%d fr "
                         "src+%d dst+%d queue=%d",
                         self.name, self._st_sent, self._st_sent_bytes / 1024,
                         self._st_recv, self._st_src, self._st_dst, qlen)
                self._st_sent = self._st_sent_bytes = 0
                self._st_recv = self._st_src = self._st_dst = 0
            if stop.is_set() or self._restart or not self._enabled():
                self._closing = True
                await session.close()
                return
            if not self._source.active(self._idle_disconnect):
                log.info("[%s] voice idle %.0fs - flushing and closing",
                         self.name, self._idle_disconnect)
                try:
                    await session.send(STREAM_END)
                except Exception:
                    pass
                close_at = time.time() + 2.0  # let final transcripts arrive
                while time.time() < close_at:
                    await asyncio.sleep(0.1)
                    if stop.is_set() or self._restart or not self._enabled():
                        self._closing = True
                        await session.close()
                        return
                    if self._source.active(self._idle_disconnect):
                        log.info("[%s] voice resumed during idle close - keeping session open",
                                 self.name)
                        break
                else:
                    self._closing = True
                    await session.close()
                    return
