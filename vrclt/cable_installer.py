"""One-click VB-Audio Virtual Cable install (Windows).

Only on the user's request: the official, unmodified VB-CABLE driver pack is
downloaded from VB-Audio, the setup program's Authenticode signature must be
VB-Audio's, and VB-Audio's own setup window is launched elevated, so the user
sees VB-CABLE, its license and its donationware model. Nothing is bundled and
nothing installs silently: the pack's license does not allow folding it into
another program's install procedure without the author's agreement, and the
UI names the origin (www.vb-cable.com) and the donationware model as VB-Audio
asks (https://vb-audio.com/Services/licensing.htm).
"""
import ctypes
import logging
import os
import shutil
import subprocess
import tempfile
import time
import zipfile
from ctypes import wintypes
from pathlib import Path
from urllib.request import Request, urlopen

from . import __version__

log = logging.getLogger(__name__)

PACK_URL = "https://download.vb-audio.com/Download_CABLE/VBCABLE_Driver_Pack45.zip"
PAGE_URL = "https://vb-audio.com/Cable/"
SIGNER = "BUREL VINCENT"          # VB-Audio's code-signing subject
MAX_BYTES = 20 * 1024 * 1024
INSTALL_TIMEOUT_MS = 20 * 60 * 1000
_ERROR_CANCELLED = 1223           # the user declined the UAC prompt


# The driver registers this service at install time; its audio endpoints
# often appear seconds later, and on some PCs only after a restart.
_SERVICE_KEY = r"SYSTEM\CurrentControlSet\Services\VBAudioVACMME"
_RENDER_KEY = r"SOFTWARE\Microsoft\Windows\CurrentVersion\MMDevices\Audio\Render"
_DEVICE_DESC = "{a45c254e-df1c-4efd-8020-67d146a850e0},2"     # "CABLE Input"
_INTERFACE_NAME = "{b3f8fa53-0004-438e-9003-51a46e139bfc},6"  # "VB-Audio Virtual Cable"


def driver_installed(_winreg=None) -> bool:
    """True once the VB-CABLE driver is installed, before its devices exist."""
    try:
        winreg = _winreg
        if winreg is None:
            import winreg
        winreg.CloseKey(winreg.OpenKey(winreg.HKEY_LOCAL_MACHINE, _SERVICE_KEY))
        return True
    except OSError:
        return False


def endpoint_active(_winreg=None) -> bool:
    """True when Windows lists an active "CABLE Input" playback endpoint."""
    winreg = _winreg
    if winreg is None:
        import winreg
    try:
        root = winreg.OpenKey(winreg.HKEY_LOCAL_MACHINE, _RENDER_KEY)
    except OSError:
        return False
    with root:
        index = 0
        while True:
            try:
                name = winreg.EnumKey(root, index)
            except OSError:
                return False
            index += 1
            try:
                with winreg.OpenKey(root, name) as device:
                    state = winreg.QueryValueEx(device, "DeviceState")[0]
                    with winreg.OpenKey(device, "Properties") as props:
                        desc = str(winreg.QueryValueEx(props, _DEVICE_DESC)[0])
                        iface = str(winreg.QueryValueEx(props, _INTERFACE_NAME)[0])
            except OSError:
                continue
            if state == 1 and desc.startswith("CABLE Input") and "VB-Audio" in iface:
                return True


def endpoint_present(*, run=subprocess.run) -> bool:
    """True when Plug and Play lists a working "CABLE Input" audio endpoint.
    Right after the install the MMDevices entry can already say "active"
    while Windows audio does not offer the device yet (seen in Windows
    Sandbox); PnP agrees with what PortAudio will find."""
    script = ("Get-PnpDevice -Class AudioEndpoint -PresentOnly -Status OK "
              "-ErrorAction SilentlyContinue | Where-Object FriendlyName -like "
              "'CABLE Input*' | Select-Object -First 1 -ExpandProperty FriendlyName")
    try:
        result = run([_powershell(), "-NoProfile", "-NonInteractive", "-Command", script],
                     capture_output=True, text=True, timeout=30,
                     creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
    except Exception:
        log.debug("PnP endpoint query failed", exc_info=True)
        return endpoint_active()
    return bool((result.stdout or "").strip())


def wait_for_endpoint(timeout: float = 30.0, *, check=endpoint_present,
                      clock=time.monotonic, sleep=time.sleep) -> bool:
    deadline = clock() + timeout
    while True:
        if check():
            return True
        if clock() >= deadline:
            return False
        sleep(0.5)


class InstallError(RuntimeError):
    """kind: download | signature | cancelled | failed"""

    def __init__(self, kind: str, detail: str = ""):
        self.kind = kind
        super().__init__(detail or kind)


def download_pack(dest: Path, *, url: str = PACK_URL, opener=urlopen) -> Path:
    """Download and unpack the driver pack into dest/pack."""
    from .update_check import _ssl_context
    request = Request(url, headers={"User-Agent": f"vrclt/{__version__}"})
    try:
        with opener(request, timeout=30, context=_ssl_context()) as response:
            data = response.read(MAX_BYTES + 1)
    except Exception as e:
        raise InstallError("download", str(e)) from e
    if len(data) > MAX_BYTES:
        raise InstallError("download", "package is unexpectedly large")
    archive = dest / "VBCABLE_Driver_Pack.zip"
    archive.write_bytes(data)
    pack = dest / "pack"
    try:
        with zipfile.ZipFile(archive) as zf:
            for member in zf.namelist():
                parts = Path(member).parts
                if member.startswith(("/", "\\")) or ".." in parts or ":" in member:
                    raise InstallError("download", "unexpected archive layout")
            zf.extractall(pack)
    except zipfile.BadZipFile as e:
        raise InstallError("download", "not a zip archive") from e
    return pack


def setup_program(pack: Path) -> Path:
    # ARM64 Windows runs the x64 setup, which carries the ARM64 driver
    is_64bit = os.environ.get("PROCESSOR_ARCHITEW6432") or \
        os.environ.get("PROCESSOR_ARCHITECTURE", "").upper() in ("AMD64", "ARM64")
    path = pack / ("VBCABLE_Setup_x64.exe" if is_64bit else "VBCABLE_Setup.exe")
    if not path.is_file():
        raise InstallError("download", f"{path.name} missing from the pack")
    return path


def _powershell() -> str:
    root = os.environ.get("SystemRoot", r"C:\Windows")
    path = Path(root) / "System32" / "WindowsPowerShell" / "v1.0" / "powershell.exe"
    return str(path) if path.is_file() else "powershell"


def verify_signature(path: Path, *, run=subprocess.run) -> str:
    """Raise unless the file carries a valid signature from VB-Audio."""
    literal = str(path).replace("'", "''")
    script = (f"$s = Get-AuthenticodeSignature -LiteralPath '{literal}'; "
              "\"$($s.Status)|$($s.SignerCertificate.Subject)\"")
    try:
        result = run([_powershell(), "-NoProfile", "-NonInteractive", "-Command", script],
                     capture_output=True, text=True, timeout=60,
                     creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
    except Exception as e:
        raise InstallError("signature", str(e)) from e
    status, _, subject = (result.stdout or "").strip().partition("|")
    if status != "Valid" or SIGNER not in subject.upper():
        raise InstallError("signature", f"{status or 'unknown'}: {subject[:80]}")
    return subject


class _ShellExecuteInfo(ctypes.Structure):
    _fields_ = [("cbSize", wintypes.DWORD), ("fMask", wintypes.ULONG),
                ("hwnd", wintypes.HWND), ("lpVerb", wintypes.LPCWSTR),
                ("lpFile", wintypes.LPCWSTR), ("lpParameters", wintypes.LPCWSTR),
                ("lpDirectory", wintypes.LPCWSTR), ("nShow", ctypes.c_int),
                ("hInstApp", wintypes.HINSTANCE), ("lpIDList", ctypes.c_void_p),
                ("lpClass", wintypes.LPCWSTR), ("hkeyClass", wintypes.HKEY),
                ("dwHotKey", wintypes.DWORD), ("hIconOrMonitor", wintypes.HANDLE),
                ("hProcess", wintypes.HANDLE)]


def run_elevated(path: Path) -> int:
    """Run the setup with a UAC prompt and wait for it to close."""
    shell32 = ctypes.WinDLL("shell32", use_last_error=True)
    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    info = _ShellExecuteInfo()
    info.cbSize = ctypes.sizeof(info)
    info.fMask = 0x00000040          # SEE_MASK_NOCLOSEPROCESS
    info.lpVerb = "runas"
    info.lpFile = str(path)
    info.lpDirectory = str(path.parent)
    info.nShow = 1                   # SW_SHOWNORMAL: VB-Audio's own window
    if not shell32.ShellExecuteExW(ctypes.byref(info)):
        error = ctypes.get_last_error()
        if error == _ERROR_CANCELLED:
            raise InstallError("cancelled")
        raise InstallError("failed", f"could not start setup (error {error})")
    if not info.hProcess:
        return 0
    try:
        kernel32.WaitForSingleObject(info.hProcess, INSTALL_TIMEOUT_MS)
        code = wintypes.DWORD()
        kernel32.GetExitCodeProcess(info.hProcess, ctypes.byref(code))
        return int(code.value)
    finally:
        kernel32.CloseHandle(info.hProcess)


def install(progress=lambda stage: None) -> bool:
    """Download, verify and run VB-Audio's setup, then give Windows a moment
    to bring up the new devices. Returns whether "CABLE Input" is active
    (False usually means the PC must restart first). progress() receives
    "download", "verify", "install" and "devices"."""
    work = Path(tempfile.mkdtemp(prefix="vrclt-vbcable-"))
    try:
        progress("download")
        pack = download_pack(work)
        setup = setup_program(pack)
        progress("verify")
        verify_signature(setup)
        progress("install")
        code = run_elevated(setup)
        log.info("VB-CABLE setup exited with code %s", code)
    finally:
        shutil.rmtree(work, ignore_errors=True)
    progress("devices")
    ready = wait_for_endpoint()
    log.info("VB-CABLE driver %s, CABLE Input %s",
             "installed" if driver_installed() else "not found",
             "active" if ready else "not active yet (restart needed)")
    return ready
