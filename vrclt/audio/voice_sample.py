"""Record a short microphone clip to use as a voice-cloning reference.

Opens its own input stream (shared mode coexists with the running pipeline,
like the mic finder) and keeps the device's native rate: resampling adds
nothing to a reference clip. Levels use the int16 RMS scale of MicCapture.
"""
import io
import logging
import wave

import numpy as np
import sounddevice as sd

from . import devices

log = logging.getLogger(__name__)

BLOCK_SEC = 0.05


class VoiceSampleRecorder:
    def __init__(self, device: str = "", max_sec: float = 120.0):
        self._device = str(device or "")
        self._max_sec = float(max_sec)
        self._chunks: list[bytes] = []
        self._frames = 0
        self._stream = None
        self.rate = 48000
        self.level = 0.0

    @property
    def seconds(self) -> float:
        return self._frames / float(self.rate)

    @property
    def full(self) -> bool:
        return self.seconds >= self._max_sec

    def start(self) -> None:
        idx = devices.find_input(self._device)
        if idx is None and self._device:
            idx = devices.find_input("")    # the saved mic is unplugged
        if idx is None:
            raise RuntimeError("no microphone found")
        self.rate = int(round(float(sd.query_devices(idx)["default_samplerate"]))) or 48000
        limit = int(self._max_sec * self.rate)

        def callback(indata, frames, _time, _status):
            # plain float/int writes: read lock-free from the Qt thread
            self.level = float(np.sqrt(np.mean(np.square(indata.astype(np.float32)))))
            room = limit - self._frames
            if room > 0:
                self._chunks.append(indata[:room].tobytes())
                self._frames += min(frames, room)

        self._stream = sd.InputStream(
            device=idx, channels=1, samplerate=self.rate, dtype="int16",
            blocksize=int(self.rate * BLOCK_SEC), callback=callback,
            extra_settings=devices.extra_settings())
        self._stream.start()

    def stop(self) -> None:
        stream, self._stream = self._stream, None
        if stream is None:
            return
        try:
            stream.stop()
            stream.close()
        except Exception:
            log.debug("voice sample: stream close failed", exc_info=True)
        self.level = 0.0

    def wav_bytes(self) -> bytes:
        buffer = io.BytesIO()
        with wave.open(buffer, "wb") as out:
            out.setnchannels(1)
            out.setsampwidth(2)
            out.setframerate(self.rate)
            out.writeframes(b"".join(self._chunks))
        return buffer.getvalue()
