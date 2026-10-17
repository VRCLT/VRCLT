"""Run every regression script in tests/ and report the failures.

Each script runs in its own process, as it would by hand, so one script's Qt
application or patched modules never leak into the next. Used by CI and
before a release.

Run: .venv/Scripts/python scripts/run_tests.py [name ...]
"""
import os
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TIMEOUT_SEC = 300


def main(argv: list[str]) -> int:
    scripts = sorted(p for p in (ROOT / "tests").glob("*.py")
                     if p.name.startswith(("smoke_", "test_")))
    if argv:
        scripts = [p for p in scripts if p.stem in argv or p.name in argv]
        if not scripts:
            print(f"no test script matches {' '.join(argv)}")
            return 2
    # no GitHub update check from the windows the tests build
    env = dict(os.environ, PYTHONUTF8="1", PYTHONIOENCODING="utf-8",
               VRCLT_NO_UPDATE_CHECK="1")
    failed = []
    for script in scripts:
        started = time.monotonic()
        try:
            result = subprocess.run([sys.executable, str(script)], cwd=ROOT, env=env,
                                    capture_output=True, text=True, encoding="utf-8",
                                    errors="replace", timeout=TIMEOUT_SEC)
            ok, output = result.returncode == 0, result.stdout + result.stderr
            status = "ok" if ok else f"FAILED (exit {result.returncode})"
        except subprocess.TimeoutExpired as e:
            ok, status = False, f"FAILED (timed out after {TIMEOUT_SEC}s)"
            output = "".join(str(part or "") for part in (e.stdout, e.stderr))
        print(f"{script.name:<36} {status}  {time.monotonic() - started:.1f}s", flush=True)
        if not ok:
            failed.append(script.name)
            print(output.rstrip(), flush=True)
    print(f"\n{len(scripts) - len(failed)}/{len(scripts)} passed")
    if failed:
        print("failed: " + ", ".join(failed))
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
