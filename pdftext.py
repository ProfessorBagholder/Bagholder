"""PDF text for filing summaries.

Issuer PDFs store their text as subsetted-font glyph codes a naive reader cannot turn
back into words, so a real engine is needed. This uses a system `pdftotext` (poppler)
when present, otherwise pdfminer.six — which `deps.py` provisions invisibly at startup
into ~/.bagholder/pylibs, so this module simply uses it once it is importable. There is
nothing for the user to install or trigger.

Set BAGHOLDER_NO_PDF=1 to disable extraction (subjects still work; PDF summaries do
not). Standard library plus the auto-provisioned pdfminer.six."""
from __future__ import annotations

import io
import os
import shutil
import subprocess

import deps

DISABLED = bool(os.environ.get("BAGHOLDER_NO_PDF"))


def _extract_text():
    """pdfminer's entry point if it can be imported (looking in the private dir too),
    else None."""
    deps.activate()
    try:
        from pdfminer.high_level import extract_text
        return extract_text
    except Exception:
        return None


def available():
    """True if a PDF engine (pdfminer) can be imported right now."""
    return _extract_text() is not None


def status():
    """ready | installing | off | failed — reflects the shared dependency install."""
    if available():
        return "ready"
    return deps.status()


def pending():
    """True while the engine is still being provisioned, so a caller retries later."""
    return status() == "installing"


def ensure():
    """Make sure provisioning is under way (normally already kicked at startup)."""
    if not DISABLED:
        deps.provision()


def text(data):
    """Readable text from a PDF's bytes, or "" if no engine can read it yet or the
    bytes are not a PDF. Never raises."""
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
