"""VRChat's saved settings, for the setup checklist and auto setup.

VRChat keeps its options as Unity PlayerPrefs in
HKCU\\Software\\VRChat\\VRChat. Each value name carries a "_h<hash>" suffix
(32-bit djb2-xor of the key); strings are REG_BINARY UTF-8 with a trailing
NUL, integers are DWORDs. VRChat may only write a changed option back when it
saves its preferences, so a value can lag behind the in-game menu - and a
running VRChat overwrites anything written meanwhile when it exits, which is
why write() is only offered while VRChat is closed.
"""
import logging
import re
from dataclasses import dataclass

from . import platform_support

log = logging.getLogger(__name__)

REG_PATH = r"Software\VRChat\VRChat"
_MIC_VR = "VRC_INPUT_MIC_DEVICE_NAME_VR"
_MIC_DESKTOP = "VRC_INPUT_MIC_DEVICE_NAME_Desktop"
_OSC = "VRC_INPUT_OSC"


@dataclass(frozen=True)
class VRChatSettings:
    # None = never saved; "" = VRChat's "Default" (the Windows default mic)
    mic_vr: str | None = None
    mic_desktop: str | None = None
    osc_enabled: bool | None = None


def _find(values: dict, prefix: str):
    for name, value in values.items():
        if name.startswith(prefix + "_h"):
            return value
    return None


def _text(value) -> str | None:
    if value is None:
        return None
    if isinstance(value, (bytes, bytearray)):
        value = bytes(value).decode("utf-8", errors="replace")
    return str(value).split("\0", 1)[0].strip()


def unity_hash(key: str) -> int:
    """The PlayerPrefs value-name suffix Unity derives from a key."""
    value = 5381
    for byte in key.encode("utf-8"):
        value = ((value * 33) ^ byte) & 0xFFFFFFFF
    return value


def vrchat_mic_name(device: str) -> str:
    """VRChat lists devices as 'Name (Adapter)' even where Windows (or
    PortAudio) drops the space: 'CABLE Output(VB-Audio...)'."""
    return re.sub(r"\s*\(", " (", str(device or "").strip(), count=1)


def running() -> bool:
    try:
        import psutil
        return any(str(proc.info.get("name") or "").lower() == "vrchat.exe"
                   for proc in psutil.process_iter(["name"]))
    except Exception:
        log.debug("process scan failed", exc_info=True)
        return False


def write(*, mic: str | None = None, osc: bool | None = None, _winreg=None) -> dict:
    """Set VRChat's microphone (VR and desktop) and/or OSC switch. Returns
    {value name: previous data} for the log. Raises OSError when VRChat has
    never run here (no settings key)."""
    winreg = _winreg
    if winreg is None:
        import winreg
    previous = {}
    with winreg.OpenKey(winreg.HKEY_CURRENT_USER, REG_PATH, 0,
                        winreg.KEY_READ | winreg.KEY_SET_VALUE) as key:
        names = {}
        index = 0
        while True:
            try:
                name, data, _kind = winreg.EnumValue(key, index)
            except OSError:
                break
            names[name] = data
            index += 1

        def value_name(prefix: str) -> str:
            existing = next((n for n in names if n.startswith(prefix + "_h")), "")
            return existing or f"{prefix}_h{unity_hash(prefix)}"

        if mic is not None:
            data = mic.encode("utf-8") + b"\0"
            for prefix in (_MIC_VR, _MIC_DESKTOP):
                name = value_name(prefix)
                previous[name] = _text(names.get(name))
                winreg.SetValueEx(key, name, 0, winreg.REG_BINARY, data)
        if osc is not None:
            name = value_name(_OSC)
            previous[name] = names.get(name)
            winreg.SetValueEx(key, name, 0, winreg.REG_DWORD, int(bool(osc)))
    log.info("VRChat settings written (mic=%r osc=%r); previous: %r", mic, osc, previous)
    return previous


def restore(previous: dict, _winreg=None) -> None:
    """Put back the values write() replaced (None = the value did not exist)."""
    winreg = _winreg
    if winreg is None:
        import winreg
    with winreg.OpenKey(winreg.HKEY_CURRENT_USER, REG_PATH, 0,
                        winreg.KEY_SET_VALUE) as key:
        for name, data in previous.items():
            if data is None:
                try:
                    winreg.DeleteValue(key, name)
                except OSError:
                    pass
            elif name.startswith(_OSC + "_h"):
                winreg.SetValueEx(key, name, 0, winreg.REG_DWORD, int(data))
            else:
                winreg.SetValueEx(key, name, 0, winreg.REG_BINARY,
                                  str(data).encode("utf-8") + b"\0")
    log.info("VRChat settings restored: %r", previous)


def parse(values: dict) -> VRChatSettings:
    """Build settings from {value name: data} as read from the registry."""
    osc = _find(values, _OSC)
    try:
        osc_enabled = None if osc is None else bool(int(osc))
    except (TypeError, ValueError):
        osc_enabled = None
    return VRChatSettings(mic_vr=_text(_find(values, _MIC_VR)),
                          mic_desktop=_text(_find(values, _MIC_DESKTOP)),
                          osc_enabled=osc_enabled)


def read() -> VRChatSettings | None:
    """VRChat's saved settings, or None where VRChat has never run here."""
    if not platform_support.IS_WINDOWS:
        return None
    import winreg
    try:
        key = winreg.OpenKey(winreg.HKEY_CURRENT_USER, REG_PATH)
    except OSError:
        return None
    values = {}
    with key:
        index = 0
        while True:
            try:
                name, data, _kind = winreg.EnumValue(key, index)
            except OSError:
                break
            if name.startswith((_MIC_VR, _MIC_DESKTOP, _OSC + "_h")):
                values[name] = data
            index += 1
    return parse(values)
