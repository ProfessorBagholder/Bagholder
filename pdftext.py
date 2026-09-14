"""PDF text for filing summaries.

Issuer PDFs store their text as subsetted-font glyph codes that a naive reader
cannot turn back into words, so a real engine is needed. This uses pdfminer.six,
provisioned automatically and transparently the same way the local model is: if it
is not importable, it is pip-installed into ~/.bagholder/pylibs in the background on
first use and added to sys.path, so the user installs nothing. A system `pdftotext`
(poppler), when present, is used first as a fast path.

Standard library plus the one auto-provisioned package. Set BAGHOLDER_NO_PDF=1 to
disable both the install and extraction (subjects still work; summaries of PDFs do
not)."""
from __future__ import annotations

import io
import os
import shutil
import subprocess
import sys
import threading
from pathlib import Path

DISABLED = bool(os.environ.get("BAGHOLDER_NO_PDF"))
PIP_SPEC = os.environ.get("BAGHOLDER_PDFMINER_SPEC", "pdfminer.six==20260107")

_lock = threading.Lock()
_state = {"phase": "off", "detail": ""}   # off | installing | ready | failed


def _home():
    return Path(os.environ.get("BAGHOLDER_HOME") or (Path.home() / ".bagholder"))


def _libs_dir():
    d = _home() / "pylibs"
    d.mkdir(parents=True, exist_ok=True)
    return d


def _add_path():
    p = str(_libs_dir())
    if p not in sys.path:
        sys.path.insert(0, p)


def _extract_text():
    """The pdfminer entry point if it can be imported (looking in the private dir
    too), else None."""
    _add_path()
    try:
        from pdfminer.high_level import extract_text
        return extract_text
    except Exception:
        return None


def available():
    """True if pdfminer can be imported right now."""
    return _extract_text() is not None


def status():
    """off | installing | ready | failed — for the UI and for retry decisions."""
    if available():
        with _lock:
            _state["phase"] = "ready"
        return "ready"
    with _lock:
        return _state["phase"]


def pending():
    """True while the engine is still being provisioned (so a caller retries later)."""
    return status() == "installing"


def ensure():
    """Start installing pdfminer.six in the background if it is not available. Returns
    at once; progress shows through status()."""
    if DISABLED or available():
        return
    with _lock:
        if _state["phase"] == "installing":
            return
        _state["phase"] = "installing"
    threading.Thread(target=_install, name="bagholder-pdftext", daemon=True).start()


def _install():
    try:
        subprocess.run(
            [sys.executable, "-m", "pip", "install", "--quiet", "--disable-pip-version-check",
             "--target", str(_libs_dir()), PIP_SPEC],
            check=True, capture_output=True, timeout=900)
    except Exception as e:
        detail = getattr(e, "stderr", b"") or b""
        with _lock:
            _state["phase"], _state["detail"] = "failed", (detail.decode("utf-8", "replace")[:200] if detail else str(e)[:200])
        return
    with _lock:
        _state["phase"] = "ready" if _extract_text() else "failed"


def text(data):
    """Readable text from a PDF's bytes, or "" if no engine can read it yet (which
    kicks provisioning in the background) or the bytes are not a PDF. Never raises."""
    if DISABLED or not isinstance(data, (bytes, bytearray)) or data[:5] != b"%PDF-":
        return ""
    exe = shutil.which("pdftotext")   # fast path when the user already has poppler
    if exe:
        try:
            out = subprocess.run([exe, "-q", "-nopgbrk", "-", "-"], input=bytes(data),
                                 capture_output=True, timeout=30)
            got = out.stdout.decode("utf-8", "replace").strip()
            if got:
                return got
        except Exception:
            pass
    extract_text = _extract_text()
    if extract_text is None:
        ensure()
        return ""
    try:
        return (extract_text(io.BytesIO(bytes(data))) or "").strip()
    except Exception:
        return ""
