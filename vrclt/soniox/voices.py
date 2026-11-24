"""Soniox cloned voices: create, list, prepare and delete over the REST API.

A cloned voice is one stored reference clip. POST /v1/voices (multipart name
+ file) answers at once with the voice's UUID and prepares it for each TTS
model in the background; the TTS `voice` field then takes that UUID in place
of a stock voice name. Voices belong to the API key's project.
https://docs.soniox.com/tts/concepts/voice-cloning
https://docs.soniox.com/api-reference/tts/voices/create_voice
"""
import json
import mimetypes
import re
import time
import urllib.error
import uuid
from dataclasses import dataclass
from urllib.parse import quote
from urllib.request import Request, urlopen

from .. import __version__

API_BASE = "https://api.soniox.com"
MAX_UPLOAD_BYTES = 35 * 1024 * 1024
MAX_NAME_CHARS = 128
# fallback only: GET /v1/tts-models reports the real per-model limit
MAX_REFERENCE_SEC = 120.0
READY, PROCESSING, NOT_COMPUTED, FAILED = "ready", "processing", "not_computed", "failed"

_UUID_RE = re.compile(
    r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}", re.IGNORECASE)


def is_voice_id(value) -> bool:
    """Soniox treats a UUID `voice` as a cloned voice, anything else as a
    stock voice name."""
    return bool(_UUID_RE.fullmatch(str(value or "").strip()))


class VoiceApiError(RuntimeError):
    def __init__(self, status: int, kind: str, message: str):
        self.status = status
        self.kind = kind
        super().__init__(message or kind)


@dataclass(frozen=True)
class Voice:
    id: str
    name: str
    filename: str = ""
    created_at: str = ""
    models: tuple = ()      # ((model, status, error_type, error_message), ...)

    @classmethod
    def from_json(cls, data) -> "Voice":
        if not isinstance(data, dict) or not data.get("id"):
            raise VoiceApiError(0, "parse", "Soniox returned an invalid voice.")
        models = tuple(
            (str(m.get("model") or ""), str(m.get("status") or NOT_COMPUTED),
             str(m.get("error_type") or ""), str(m.get("error_message") or ""))
            for m in data.get("models") or [] if isinstance(m, dict))
        return cls(id=str(data["id"]), name=str(data.get("name") or ""),
                   filename=str(data.get("filename") or ""),
                   created_at=str(data.get("created_at") or ""), models=models)

    def _entry(self, model: str):
        return next((m for m in self.models if m[0] == model), None)

    def status(self, model: str) -> str:
        """A voice the service has not prepared for `model` is not_computed."""
        entry = self._entry(model)
        return entry[1] if entry else NOT_COMPUTED

    def error(self, model: str) -> str:
        entry = self._entry(model)
        return (entry[3] or entry[2]) if entry else ""


class VoiceClient:
    def __init__(self, api_key: str, *, base_url: str = API_BASE,
                 timeout: float = 30.0, opener=urlopen):
        self._api_key = str(api_key or "").strip()
        self._base = base_url.rstrip("/")
        self._timeout = timeout
        self._opener = opener

    def _request(self, method: str, path: str, *, body: bytes | None = None,
                 content_type: str = "", timeout: float | None = None):
        from ..update_check import _ssl_context
        headers = {"Authorization": f"Bearer {self._api_key}",
                   "Accept": "application/json",
                   "User-Agent": f"vrclt/{__version__}"}
        if content_type:
            headers["Content-Type"] = content_type
        request = Request(self._base + path, data=body, method=method, headers=headers)
        kwargs = {"timeout": timeout or self._timeout}
        context = _ssl_context()
        if context is not None:
            kwargs["context"] = context
        try:
            with self._opener(request, **kwargs) as response:
                raw = response.read()
        except urllib.error.HTTPError as e:
            raise self._http_error(e) from None
        except (urllib.error.URLError, OSError) as e:
            raise VoiceApiError(0, "network", str(getattr(e, "reason", None) or e)) from None
        if not raw:
            return None
        try:
            return json.loads(raw.decode("utf-8"))
        except (UnicodeDecodeError, ValueError):
            raise VoiceApiError(0, "parse", "Soniox returned an invalid response.") from None

    def _http_error(self, error: urllib.error.HTTPError) -> VoiceApiError:
        try:
            data = json.loads(error.read().decode("utf-8", errors="replace"))
        except (OSError, ValueError):
            data = None
        data = data if isinstance(data, dict) else {}
        kind = str(data.get("error_type") or f"http_{error.code}")
        details = [str(v.get("message")) for v in data.get("validation_errors") or []
                   if isinstance(v, dict) and v.get("message")]
        message = "; ".join(details) or str(data.get("message") or f"HTTP {error.code}")
        if self._api_key:
            message = message.replace(self._api_key, "[redacted]")
        return VoiceApiError(error.code, kind, message[:300])

    def list(self) -> list[Voice]:
        voices, cursor = [], ""
        for _page in range(20):
            path = "/v1/voices?limit=100" + (f"&cursor={quote(cursor, safe='')}" if cursor else "")
            data = self._request("GET", path) or {}
            voices.extend(Voice.from_json(item) for item in data.get("voices") or [])
            cursor = str(data.get("next_page_cursor") or "")
            if not cursor:
                break
        return voices

    def get(self, voice_id: str) -> Voice:
        return Voice.from_json(self._request("GET", f"/v1/voices/{quote(voice_id, safe='')}"))

    def create(self, name: str, audio: bytes, filename: str) -> Voice:
        name = str(name or "").strip()
        if not name or len(name) > MAX_NAME_CHARS:
            raise VoiceApiError(0, "invalid_name",
                                f"The voice name must be 1 to {MAX_NAME_CHARS} characters.")
        if not audio:
            raise VoiceApiError(0, "invalid_audio", "The voice sample is empty.")
        if len(audio) > MAX_UPLOAD_BYTES:
            raise VoiceApiError(0, "audio_too_large",
                                "The voice sample is larger than 35 MB.")
        boundary = "vrclt" + uuid.uuid4().hex
        safe_name = re.sub(r'[\r\n"\\]', "_", str(filename or "voice.wav")) or "voice.wav"
        media = mimetypes.guess_type(safe_name)[0] or "application/octet-stream"
        body = b"".join((
            f'--{boundary}\r\nContent-Disposition: form-data; name="name"\r\n\r\n'.encode(),
            name.encode("utf-8"),
            (f'\r\n--{boundary}\r\nContent-Disposition: form-data; name="file"; '
             f'filename="{safe_name}"\r\nContent-Type: {media}\r\n\r\n').encode("utf-8"),
            audio,
            f"\r\n--{boundary}--\r\n".encode(),
        ))
        return Voice.from_json(self._request(
            "POST", "/v1/voices", body=body,
            content_type=f"multipart/form-data; boundary={boundary}", timeout=180.0))

    def recompute(self, voice_id: str, model: str = "") -> Voice:
        body = json.dumps({"model": model} if model else {}).encode("utf-8")
        return Voice.from_json(self._request(
            "POST", f"/v1/voices/{quote(voice_id, safe='')}/recompute",
            body=body, content_type="application/json"))

    def delete(self, voice_id: str) -> None:
        self._request("DELETE", f"/v1/voices/{quote(voice_id, safe='')}")

    def max_reference_sec(self, model: str) -> float | None:
        """Longest reference clip `model` accepts; 0 when it cannot clone,
        None when the service does not say."""
        data = self._request("GET", "/v1/tts-models") or {}
        for entry in data.get("models") or []:
            if isinstance(entry, dict) and entry.get("id") == model:
                if entry.get("supports_voice_cloning") is False:
                    return 0.0
                limit = entry.get("voice_cloning_max_audio_duration_ms")
                return float(limit) / 1000.0 if limit else None
        return None

    def wait_ready(self, voice_id: str, model: str, *, timeout: float = 120.0,
                   interval: float = 1.0, cancelled=lambda: False,
                   sleep=time.sleep, clock=time.monotonic) -> Voice:
        """Poll until the voice is ready for `model`. A voice that was never
        prepared for it is recomputed once; a failed one cannot be recovered."""
        deadline = clock() + timeout
        recomputed = False
        while True:
            voice = self.get(voice_id)
            status = voice.status(model)
            if status == READY:
                return voice
            if status == FAILED:
                raise VoiceApiError(0, "voice_failed", voice.error(model)
                                    or "Soniox could not prepare this voice.")
            if status == NOT_COMPUTED and not recomputed:
                recomputed = True
                self.recompute(voice_id, model)
            if cancelled():
                raise VoiceApiError(0, "cancelled", "Cancelled.")
            if clock() >= deadline:
                raise VoiceApiError(0, "timeout",
                                    "Soniox is still preparing this voice. Try again shortly.")
            sleep(interval)
