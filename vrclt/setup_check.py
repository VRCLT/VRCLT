"""Setup checklist behind the dashboard's setup panel.

The rules are pure functions over the device lists, the config and VRChat's
saved settings, so they test without audio hardware, a registry, or Qt;
collect() gathers the live inputs.

VB-Audio Virtual Cable naming trips people up: "CABLE Input" is a PLAYBACK
device (vrclt plays the translated voice into it) and "CABLE Output" is a
RECORDING device (VRChat/Discord use it as their microphone). Whatever is
played into the Input comes out of the Output.
"""
import copy
import logging
import re
from dataclasses import dataclass, field

from . import config as config_mod
from . import platform_support

log = logging.getLogger(__name__)

VB_CABLE_URL = "https://vb-audio.com/Cable/"

OK, WARN, ERROR, INFO = "ok", "warn", "error", "info"
_SEVERITY = {OK: 0, INFO: 1, WARN: 2, ERROR: 3}

# Loopback/virtual endpoints that carry app audio, never a person's voice.
# "Virtual" alone is deliberately absent: Meta Quest's headset mic is the
# "Oculus Virtual Audio Device".
_VIRTUAL_RE = re.compile(
    r"\bCABLE\b|VB-Audio|Voicemeeter|BlackHole|Steam Streaming|Stereo Mix"
    r"|스테레오 믹스|What U Hear", re.IGNORECASE)


def is_virtual_device(name: str) -> bool:
    return bool(name) and bool(_VIRTUAL_RE.search(name))


def real_inputs(names) -> list[str]:
    return [n for n in names if n and not is_virtual_device(n)]


def mic_choices(names, cfg: dict) -> list[str]:
    """Inputs offered as "my microphone": all but the recording side of the
    cable the translated voice goes into, which would only feed VRCLT its own
    voice back. Other virtual devices (VoiceMeeter etc.) may be deliberate
    routing and stay listed; a hidden device that is already configured is
    still shown by the pickers as the current value."""
    hidden = set()
    for playback in (platform_support.default_loopback_device(),
                     str(cfg.get("outbound", {}).get("tts_device") or "")):
        # only a loopback has a recording side; "Speakers" must hide nothing
        if playback and (is_virtual_device(playback)
                         or playback == platform_support.default_loopback_device()):
            hidden.add(app_mic_name(playback).lower())
    hidden.discard("")
    return [n for n in names if not n or not any(h in n.lower() for h in hidden)]


def find_device(names, wanted: str) -> str:
    """The listed device a config substring resolves to ("" if none): the
    first-match rule of audio.devices."""
    wanted = str(wanted or "").strip().lower()
    if not wanted:
        return ""
    return next((n for n in names if n and wanted in n.lower()), "")


def _same_device(a: str, b: str) -> bool:
    a, b = str(a or "").lower(), str(b or "").lower()
    return bool(a and b) and (a in b or b in a)


def app_mic_name(loopback: str) -> str:
    """The recording side the target app must use for a loopback playback
    device: "CABLE Input (VB-Audio ...)" -> "CABLE Output". BlackHole uses
    one name for both sides."""
    # PortAudio may drop the space: "CABLE Input(VB-Audio Virtual Cable)"
    core = str(loopback or "").split("(", 1)[0].strip()
    return re.sub(r"\bInput\b", "Output", core)


@dataclass
class Environment:
    api_key_present: bool
    app_mode: str = "vrchat"
    app_name: str = "VRChat"
    inputs: list = field(default_factory=list)
    outputs: list = field(default_factory=list)
    default_input: str = ""
    default_output: str = ""
    vrchat: object = None            # vrchat_settings.VRChatSettings | None
    osc_bound: bool | None = None    # None = OSC listener not running
    osc_last_rx: float = 0.0         # when VRChat last sent us OSC (0 = never)
    cable_driver: bool = False       # VB-CABLE driver installed (devices may need a restart)
    missing_key_provider: str = ""   # the engine whose key is missing ("" = none)


@dataclass(frozen=True)
class Item:
    key: str             # api_key | cable | voice_out | mic | vrchat_mic | vrchat_osc
    status: str          # ok | info | warn | error
    state: str           # selects the message: setup_<key>_<state>
    args: dict = field(default_factory=dict, hash=False)
    fixable: bool = False   # resolved by auto_fix()


def worst(items) -> str:
    return max((item.status for item in items), key=_SEVERITY.__getitem__, default=OK)


def suggest_mic(env: Environment) -> tuple[str, str]:
    """(device, source) for the microphone the user most likely speaks into.
    source: "vrchat" (the mic VRChat used before it was pointed at the
    cable), "default" (the system default recording device), "first", or ""
    when there is no real microphone at all."""
    real = real_inputs(env.inputs)
    vrchat = env.vrchat
    if vrchat is not None:
        for hint in (vrchat.mic_vr, vrchat.mic_desktop):
            if hint and not is_virtual_device(hint):
                match = next((n for n in real if _same_device(n, hint)), "")
                if match:
                    return match, "vrchat"
    if env.default_input in real:
        return env.default_input, "default"
    return (real[0], "first") if real else ("", "")


def _check_api_key(env: Environment) -> Item:
    return Item("api_key", OK if env.api_key_present else ERROR,
                "ok" if env.api_key_present else "missing")


def _check_voice_out(cfg: dict, env: Environment, loopback: str, cable: str) -> Item:
    wanted = str(cfg.get("outbound", {}).get("tts_device") or "").strip()
    current = find_device(env.outputs, wanted) if wanted else env.default_output
    args = {"name": current or wanted, "cable": cable or loopback}
    if current and loopback.lower() in current.lower():
        return Item("voice_out", OK, "ok", args)
    if not cable:
        return Item("voice_out", WARN, "needs_cable", args)
    if wanted and not current:
        return Item("voice_out", ERROR, "missing", args, fixable=True)
    if is_virtual_device(current):
        # another loopback (VoiceMeeter etc.) may well be intentional
        return Item("voice_out", WARN, "other_virtual", args, fixable=True)
    return Item("voice_out", ERROR, "not_cable", args, fixable=True)


def _check_mic(cfg: dict, env: Environment) -> Item:
    wanted = str(cfg.get("outbound", {}).get("mic_device") or "").strip()
    suggestion = suggest_mic(env)[0]
    if not real_inputs(env.inputs) and not env.default_input:
        return Item("mic", ERROR, "none")
    current = find_device(env.inputs, wanted) if wanted else env.default_input
    if not current:
        return Item("mic", WARN, "missing", {"name": wanted or "-"},
                    fixable=bool(suggestion))
    if is_virtual_device(current):
        # the app's own translated voice would come straight back in
        return Item("mic", ERROR, "virtual", {"name": current},
                    fixable=bool(suggestion))
    return Item("mic", OK, "ok" if wanted else "default", {"name": current})


def _check_vrchat_mic(env: Environment, expected: str, cable_mic: str) -> Item:
    vrchat = env.vrchat
    args = {"expected": expected}
    # auto setup can write VRChat's settings once VRChat has created them
    fixable = vrchat is not None and bool(cable_mic)
    if vrchat is None or (vrchat.mic_vr is None and vrchat.mic_desktop is None):
        return Item("vrchat_mic", INFO, "unknown", args, fixable=fixable)
    wrong = []
    for mode, value in (("vr", vrchat.mic_vr), ("desktop", vrchat.mic_desktop)):
        if value is None:
            continue
        effective = value or env.default_input   # "" = VRChat's Default
        if expected.lower() not in str(effective).lower():
            wrong.append((mode, value, effective))
    if not wrong:
        return Item("vrchat_mic", OK, "ok", args)
    return Item("vrchat_mic", WARN, "wrong", {**args, "modes": tuple(wrong)},
                fixable=fixable)


def _check_vrchat_osc(cfg: dict, env: Environment) -> Item:
    enabled = env.vrchat.osc_enabled if env.vrchat is not None else None
    port = cfg.get("control", {}).get("osc_listen_port", 9001)
    if enabled is False:
        return Item("vrchat_osc", WARN, "off", fixable=True)
    if env.osc_bound is False:
        return Item("vrchat_osc", WARN, "port_busy", {"port": port})
    if env.osc_last_rx:
        return Item("vrchat_osc", OK, "receiving")
    if enabled:
        return Item("vrchat_osc", OK, "enabled")
    return Item("vrchat_osc", INFO, "unknown", fixable=env.vrchat is not None)


def _check_system_sound(env: Environment) -> Item | None:
    """Installing VB-Cable can make Windows switch its default playback
    device to the cable: then the PC itself plays nothing audible."""
    if is_virtual_device(env.default_output):
        return Item("system_sound", WARN, "output", {"name": env.default_output})
    return None


def run_checks(cfg: dict, env: Environment) -> list[Item]:
    items = [_check_api_key(env)]
    out = cfg.get("outbound", {})
    outbound = bool(out.get("enabled", True))
    loopback = platform_support.default_loopback_device()
    cable = find_device(env.outputs, loopback) if loopback else ""
    if loopback:
        if cable:
            items.append(Item("cable", OK, "ok", {"name": cable}))
        elif env.cable_driver:
            # installed, but Windows brings the devices up only after a restart
            items.append(Item("cable", WARN, "restart", {"name": loopback}))
        else:
            items.append(Item("cable", ERROR, "missing", {"name": loopback}))
        if outbound:
            items.append(_check_voice_out(cfg, env, loopback, cable))
    if outbound:
        items.append(_check_mic(cfg, env))
    sound = _check_system_sound(env)
    if sound is not None:
        items.append(sound)
    if env.app_mode == "vrchat" and platform_support.IS_WINDOWS:
        if outbound and loopback:
            items.append(_check_vrchat_mic(env, app_mic_name(cable or loopback),
                                           cable_recording_device(env)))
        if out.get("chatbox", True) or cfg.get("control", {}).get("enabled", True):
            items.append(_check_vrchat_osc(cfg, env))
    return items


def cable_recording_device(env: Environment) -> str:
    """The cable's recording side (what the game must use as its mic)."""
    loopback = platform_support.default_loopback_device()
    cable = find_device(env.outputs, loopback) if loopback else ""
    return find_device(env.inputs, app_mic_name(cable)) if cable else ""


def vrchat_changes(cfg: dict, env: Environment) -> dict:
    """What auto setup would write into VRChat's settings: {"mic": name,
    "osc": True}, only for the VRChat items it can fix."""
    from .vrchat_settings import vrchat_mic_name
    items = {item.key: item for item in run_checks(cfg, env)}
    changes = {}
    mic = items.get("vrchat_mic")
    if mic is not None and mic.fixable and mic.status != OK:
        changes["mic"] = vrchat_mic_name(cable_recording_device(env))
    osc = items.get("vrchat_osc")
    if osc is not None and osc.fixable and osc.status != OK:
        changes["osc"] = True
    return changes


def auto_fix(cfg: dict, env: Environment) -> tuple[dict, list[str]]:
    """A copy of cfg with every fixable item resolved, plus the changed
    config paths (empty when there is nothing to do)."""
    new = copy.deepcopy(cfg)
    out = new.setdefault("outbound", {})
    items = {item.key: item for item in run_checks(cfg, env)}
    changed = []
    voice = items.get("voice_out")
    if voice is not None and voice.fixable:
        # the substring, like the default: survives driver renames
        out["tts_device"] = platform_support.default_loopback_device()
        changed.append("outbound.tts_device")
    mic_item = items.get("mic")
    mic, source = suggest_mic(env)
    following_default = not str(out.get("mic_device") or "").strip()
    if mic_item is not None and mic and (
            mic_item.fixable
            # the default device is not the mic the user picked in VRChat
            # (e.g. a webcam vs. the headset): pin the one they actually use
            or (mic_item.status == OK and following_default and source == "vrchat"
                and mic != env.default_input)):
        out["mic_device"] = mic
        changed.append("outbound.mic_device")
    return new, changed


def app_name(cfg: dict) -> str:
    mode = str(cfg.get("app", {}).get("mode", "vrchat") or "vrchat")
    if mode == "vrchat":
        return "VRChat"
    if mode == "discord":
        return "Discord"
    process = str(cfg.get("app", {}).get("profiles", {}).get("custom", {})
                  .get("process") or "").strip()
    return process[:-4] if process.lower().endswith(".exe") else process


def collect(cfg: dict, *, inputs, outputs, osc_status=(None, 0.0)) -> Environment:
    """Live inputs for run_checks(): defaults, VRChat's settings, OSC state."""
    from .audio import devices
    from . import cable_installer, vrchat_settings
    try:
        vrchat = vrchat_settings.read()
    except Exception:
        log.debug("VRChat settings unreadable", exc_info=True)
        vrchat = None
    # the translation engine's key, and the subtitle engine's when it differs
    missing = next((prov for prov in config_mod.providers_in_use(cfg)
                    if not config_mod.api_key_for(cfg, prov)), "")
    return Environment(
        api_key_present=not missing,
        missing_key_provider=missing,
        app_mode=str(cfg.get("app", {}).get("mode", "vrchat") or "vrchat"),
        app_name=app_name(cfg),
        inputs=list(inputs), outputs=list(outputs),
        default_input=devices.default_input_name(),
        default_output=devices.default_output_name(),
        vrchat=vrchat, osc_bound=osc_status[0], osc_last_rx=osc_status[1],
        cable_driver=platform_support.IS_WINDOWS and cable_installer.driver_installed())
