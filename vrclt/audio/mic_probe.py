"""Listen to every real microphone at once to find the one being spoken into.

Each device runs its own input stream (WASAPI shared mode lets this coexist
with the running pipeline and with VRChat holding the same mic). The score is
how far speech rises above that device's own noise floor, so a hissy device
that is always loud does not beat the headset the user is talking into.
Only sustained sound counts: the click many devices emit when a stream opens
(and other one-block spikes) is ignored.
Levels use the int16 RMS scale of MicCapture / audio.voice_rms_threshold.
"""
import collections
import logging
import time

import numpy as np
import sounddevice as sd

from . import devices

log = logging.getLogger(__name__)

BLOCK_SEC = 0.05
WARMUP_SEC = 0.5     # stream-open clicks and driver settling
SUSTAIN_BLOCKS = 3   # a level must hold this long (150 ms) to count
MIN_PEAK = 150.0     # quieter than this is not speech into that mic
MIN_RISE = 100.0     # peak above the device's own noise floor


class MicProbe:
    def __init__(self, names):
        self.names = [n for n in names if n]
        self.levels: dict[str, float] = {}
        self.peaks: dict[str, float] = {}
        self.floors: dict[str, float] = {}
        self.failed: list[str] = []
        self._streams = []

    def start(self) -> None:
        for name in self.names:
            idx = devices.find_input(name)
            if idx is None:
                self.failed.append(name)
                continue
            try:
                rate = int(round(float(sd.query_devices(idx)["default_samplerate"]))) or 48000
                stream = sd.InputStream(
                    device=idx, channels=1, samplerate=rate, dtype="int16",
                    blocksize=int(rate * BLOCK_SEC), callback=self._callback(name),
                    extra_settings=devices.extra_settings())
                stream.start()
            except Exception as e:
                log.info("mic probe: %s unavailable (%s)", name, e)
                self.failed.append(name)
                continue
            self._streams.append(stream)
            self.levels[name] = 0.0
            self.peaks[name] = 0.0
        log.info("mic probe: listening to %d device(s), %d unavailable",
                 len(self._streams), len(self.failed))

    def _callback(self, name: str):
        started = time.monotonic()
        recent = collections.deque(maxlen=SUSTAIN_BLOCKS)

        def callback(indata, _frames, _time, _status):
            rms = float(np.sqrt(np.mean(np.square(indata.astype(np.float32)))))
            # plain float writes: read lock-free from the Qt thread
            self.levels[name] = rms
            if time.monotonic() - started < WARMUP_SEC:
                return
            recent.append(rms)
            if len(recent) < SUSTAIN_BLOCKS:
                return
            sustained = min(recent)
            if sustained > self.peaks.get(name, 0.0):
                self.peaks[name] = sustained
            floor = self.floors.get(name)
            if floor is None or rms < floor:
                self.floors[name] = rms
        return callback

    def best(self) -> str:
        return pick_speaking_device(self.peaks, self.floors)

    def stop(self) -> None:
        for stream in self._streams:
            try:
                stream.stop()
                stream.close()
            except Exception:
                log.debug("mic probe: stream close failed", exc_info=True)
        self._streams = []


def pick_speaking_device(peaks: dict, floors: dict) -> str:
    """The device whose level rose furthest above its own floor, or "" when
    no device heard anything that looks like speech."""
    best, best_rise = "", 0.0
    for name, peak in peaks.items():
        rise = peak - floors.get(name, 0.0)
        if peak >= MIN_PEAK and rise >= MIN_RISE and rise > best_rise:
            best, best_rise = name, rise
    return best
