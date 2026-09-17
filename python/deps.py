"""Runtime dependencies, provisioned invisibly.

Bagholder's desktop app is a checkout run with `python3 bagholder.py`, and it must
never ask the user to run pip. On startup it makes sure its few third-party packages
are importable, installing any that are missing into a private directory
(`~/.bagholder/pylibs`) that is added to `sys.path`. The user's global environment is
never touched and there is nothing to run. It all happens in a background thread; the
app is fully usable meanwhile, with the features that need a given package simply
waiting until it is present.

`activate()` runs at import, before the modules that use these packages load, so a
package installed on an earlier launch is importable on this one. `provision()` does
any install, once, in the background. Standard library only."""
from __future__ import annotations

import importlib
import os
import subprocess
import sys
import threading
from pathlib import Path

# pip spec -> the module name that proves it is importable
_REQUIRE = {
    "curl_cffi>=0.7": "curl_cffi",         # SEDAR+ provider's browser TLS fingerprint
    "pdfminer.six==20260107": "pdfminer",  # text of issuer PDFs, for summaries
}
if sys.platform.startswith("win"):
    _REQUIRE["tzdata"] = "tzdata"          # time zones the OS does not ship

DISABLED = bool(os.environ.get("BAGHOLDER_NO_DEPS"))

_lock = threading.Lock()
_state = {"phase": "off", "detail": ""}    # off | installing | ready | failed
_started = False


def libs_dir():
    d = Path(os.environ.get("BAGHOLDER_HOME") or (Path.home() / ".bagholder")) / "pylibs"
    try:
        d.mkdir(parents=True, exist_ok=True)
    except Exception:
        pass
    return d


def activate():
    """Put the private package dir on sys.path (idempotent), so a package installed on
    an earlier launch is importable now. Safe to call more than once."""
    p = str(libs_dir())
    if p not in sys.path:
        sys.path.insert(0, p)


def _missing():
    out = []
    for spec, mod in _REQUIRE.items():
        try:
            importlib.import_module(mod)
        except Exception:
            out.append(spec)
    return out


def status():
    """off | installing | ready | failed — for diagnostics."""
    with _lock:
        return _state["phase"]


def provision():
    """Ensure every runtime dependency is importable, installing any that are missing
    into the private dir. Returns at once; the install runs in the background, once."""
    global _started
    if DISABLED:
        return
    with _lock:
        if _started:
            return
        _started = True
    threading.Thread(target=_run, name="bagholder-deps", daemon=True).start()


def _run():
    activate()
    missing = _missing()
    if not missing:
        with _lock:
            _state["phase"] = "ready"
        return
    with _lock:
        _state["phase"] = "installing"
    try:
        subprocess.run(
            [sys.executable, "-m", "pip", "install", "--quiet", "--disable-pip-version-check",
             "--target", str(libs_dir()), *missing],
            check=True, capture_output=True, timeout=1800)
    except Exception as e:
        detail = getattr(e, "stderr", b"") or b""
        with _lock:
            _state["phase"] = "failed"
            _state["detail"] = detail.decode("utf-8", "replace")[:300] if detail else str(e)[:300]
        return
    importlib.invalidate_caches()   # so a fresh import this session finds the new files
    with _lock:
        _state["phase"] = "ready" if not _missing() else "failed"
