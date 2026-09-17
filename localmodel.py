"""A local language model the app runs itself, for filing summaries — automatic and
transparent: the user installs nothing and types no commands.

Resolution order, cheapest first:
1. An endpoint the user already runs — `BAGHOLDER_LLM_URL`, or Ollama on its default
   port. If one answers, it is used and nothing is downloaded.
2. Otherwise the app provisions its own: it downloads a single self-contained model
   executable (a llamafile: model plus runtime in one portable file, Mac/Linux/
   Windows) into the app's home, verifies it against a pinned SHA-256, and runs it
   as a background server. This happens once, lazily, the first time a summary is
   asked for; later starts reuse the downloaded file.

Everything is best-effort and returns "" rather than raising, so a slow download, a
blocked binary, or no network simply means summaries are quiet — never a crash.

Security: the executable is fetched over HTTPS from a pinned Hugging Face URL and is
refused unless its SHA-256 matches the pin, so a tampered or truncated download is
never run. Override the artifact with `BAGHOLDER_LLAMAFILE_URL` /
`BAGHOLDER_LLAMAFILE_SHA256`, or point at your own server with `BAGHOLDER_LLM_URL`.
"""
from __future__ import annotations

import atexit
import hashlib
import json
import os
import shutil
import ssl
import stat
import subprocess
import sys
import threading
import time
from pathlib import Path
from urllib.request import Request, urlopen

# A small instruction-tuned model, enough for a one-sentence summary. Pinned by
# SHA-256 so only this exact file is ever executed.
LLAMAFILE_URL = os.environ.get(
    "BAGHOLDER_LLAMAFILE_URL",
    "https://huggingface.co/Mozilla/Llama-3.2-1B-Instruct-llamafile/resolve/main/Llama-3.2-1B-Instruct-Q4_K_M.llamafile",
)
LLAMAFILE_SHA256 = os.environ.get(
    "BAGHOLDER_LLAMAFILE_SHA256",
    "ac1c2864000bad7f62ee56ee908d3f55dd051a267d015b15fa6e831e69767b55",
)
USER_LLM_URL = os.environ.get("BAGHOLDER_LLM_URL", "").rstrip("/")
OLLAMA_URL = os.environ.get("BAGHOLDER_OLLAMA_URL", "http://127.0.0.1:11434").rstrip("/")
OLLAMA_MODEL = os.environ.get("BAGHOLDER_OLLAMA_MODEL", "llama3.2")
MANAGED_HOST = "127.0.0.1"
MANAGED_PORT = int(os.environ.get("BAGHOLDER_LLM_PORT", "8121"))
DOWNLOAD_TIMEOUT = 60 * 30       # a big file over a slow link
START_TIMEOUT = 120             # the server loading the model
CHAT_TIMEOUT = float(os.environ.get("BAGHOLDER_LLM_CHAT_TIMEOUT", "40"))
_ALLOWED_HOSTS = ("huggingface.co", "cdn-lfs.huggingface.co", "cdn-lfs-us-1.huggingface.co")

_lock = threading.Lock()
_state = {"phase": "off", "detail": "", "proc": None, "endpoint": "", "model": ""}
# phase: off | detecting | downloading | starting | ready | failed


def _home():
    return Path(os.environ.get("BAGHOLDER_HOME") or (Path.home() / ".bagholder"))


def _models_dir():
    import store                         # a test never makes or fills the person's model folder
    store.guard_home(_home())
    d = _home() / "models"
    d.mkdir(parents=True, exist_ok=True)
    return d


def _llamafile_path():
    return _models_dir() / "summarizer.llamafile"


def _ctx():
    try:
        import certifi
        return ssl.create_default_context(cafile=certifi.where())
    except Exception:
        for ca in ("/etc/ssl/cert.pem", "/etc/ssl/certs/ca-certificates.crt"):
            if os.path.exists(ca):
                return ssl.create_default_context(cafile=ca)
    return ssl.create_default_context()


def _get_ok(url, timeout=3):
    try:
        urlopen(Request(url, headers={"User-Agent": "Bagholder"}), timeout=timeout, context=_ctx()).read()
        return True
    except Exception:
        return False


# --------------------------------------------------------------------------- #
# Endpoint resolution
# --------------------------------------------------------------------------- #
def _detect_running():
    """A user-run endpoint, if one answers now. Returns (base_url, model) or None."""
    if USER_LLM_URL and _get_ok(USER_LLM_URL + "/v1/models", 2):
        return USER_LLM_URL, os.environ.get("BAGHOLDER_LLM_MODEL", "local")
    if _get_ok(OLLAMA_URL + "/api/tags", 2):
        return OLLAMA_URL, OLLAMA_MODEL
    return None


def status():
    """One of: off, detecting, downloading, starting, ready, failed — for the UI."""
    with _lock:
        if _state["endpoint"]:
            return "ready"
        return _state["phase"]


def available():
    return bool(endpoint())


COMING_UP = ("detecting", "starting")     # phases that finish in seconds, unlike a download


def wait_ready(seconds):
    """Wait, for at most `seconds`, for a model that is coming up right now, and say whether
    one is up. Asking for the first time is what starts it, so without this the first caller
    of a session always gets nothing. A download is never waited for: that takes minutes and
    the caller has a page to answer."""
    endpoint()                            # the ask that starts one, if none is up
    deadline = time.time() + max(0.0, seconds)
    while time.time() < deadline:
        if available():
            return True
        if status() not in COMING_UP:
            return False
        time.sleep(0.5)
    return available()


def endpoint():
    """The base URL of a working local model, or "" if none is up yet. Never blocks
    on a download; if provisioning is needed, kick it in the background and return "".
    """
    with _lock:
        if _state["endpoint"]:
            return _state["endpoint"]
    found = _detect_running()
    if found:
        with _lock:
            _state["endpoint"], _state["model"], _state["phase"] = found[0], found[1], "ready"
        return found[0]
    ensure()
    return ""


# --------------------------------------------------------------------------- #
# Provisioning (background)
# --------------------------------------------------------------------------- #
def ensure():
    """Start provisioning the managed model if it is not already under way. Returns
    at once; progress is visible through status()."""
    with _lock:
        if _state["phase"] in ("detecting", "downloading", "starting") or _state["endpoint"]:
            return
        _state["phase"] = "detecting"
    threading.Thread(target=_provision, name="bagholder-localmodel", daemon=True).start()


def _set(phase, detail=""):
    with _lock:
        _state["phase"], _state["detail"] = phase, detail


def _provision():
    try:
        found = _detect_running()
        if found:
            with _lock:
                _state["endpoint"], _state["model"], _state["phase"] = found[0], found[1], "ready"
            return
        path = _llamafile_path()
        if not _verified(path):
            _set("downloading")
            if not _download(path):
                _set("failed", "download failed")
                return
        if not _verified(path):
            _set("failed", "checksum mismatch")
            try:
                path.unlink()
            except OSError:
                pass
            return
        _set("starting")
        if _spawn(path) and _wait_ready():
            with _lock:
                _state["endpoint"] = "http://%s:%d" % (MANAGED_HOST, MANAGED_PORT)
                _state["model"] = "local"
                _state["phase"] = "ready"
        else:
            _set("failed", "server did not start")
    except Exception as e:
        _set("failed", str(e))


def _verified(path):
    if not path.exists() or not LLAMAFILE_SHA256:
        return False
    h = hashlib.sha256()
    try:
        with open(path, "rb") as fh:
            for chunk in iter(lambda: fh.read(1 << 20), b""):
                h.update(chunk)
    except OSError:
        return False
    return h.hexdigest() == LLAMAFILE_SHA256.lower()


def _download(path):
    from urllib.parse import urlparse
    if urlparse(LLAMAFILE_URL).hostname not in _ALLOWED_HOSTS:
        return False
    tmp = path.with_suffix(".part")
    try:
        req = Request(LLAMAFILE_URL, headers={"User-Agent": "Bagholder"})
        with urlopen(req, timeout=DOWNLOAD_TIMEOUT, context=_ctx()) as r, open(tmp, "wb") as out:
            shutil.copyfileobj(r, out, length=1 << 20)
        tmp.replace(path)
        path.chmod(path.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP)
        # a downloaded executable is quarantined on macOS; clear it so it can run
        if sys.platform == "darwin":
            try:
                subprocess.run(["xattr", "-d", "com.apple.quarantine", str(path)], capture_output=True, timeout=10)
            except Exception:
                pass
        return True
    except Exception:
        try:
            tmp.unlink()
        except OSError:
            pass
        return False


def _spawn(path):
    try:
        proc = subprocess.Popen(
            [str(path), "--server", "--nobrowser", "--host", MANAGED_HOST, "--port", str(MANAGED_PORT),
             "-ngl", "0", "--log-disable"],
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, stdin=subprocess.DEVNULL,
        )
    except Exception:
        # some hosts must run the APE via a shell
        try:
            proc = subprocess.Popen(
                ["sh", str(path), "--server", "--nobrowser", "--host", MANAGED_HOST, "--port", str(MANAGED_PORT), "--log-disable"],
                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, stdin=subprocess.DEVNULL,
            )
        except Exception:
            return False
    with _lock:
        _state["proc"] = proc
    return True


def _wait_ready():
    base = "http://%s:%d" % (MANAGED_HOST, MANAGED_PORT)
    deadline = time.time() + START_TIMEOUT
    while time.time() < deadline:
        with _lock:
            proc = _state["proc"]
        if proc and proc.poll() is not None:
            return False
        if _get_ok(base + "/health", 2) or _get_ok(base + "/v1/models", 2):
            return True
        time.sleep(2)
    return False


def shutdown():
    with _lock:
        proc = _state["proc"]
        _state["proc"] = None
    if proc and proc.poll() is None:
        try:
            proc.terminate()
            proc.wait(timeout=5)
        except Exception:
            try:
                proc.kill()
            except Exception:
                pass


atexit.register(shutdown)


# --------------------------------------------------------------------------- #
# Chat
# --------------------------------------------------------------------------- #
def chat(prompt, max_tokens=90):
    """One completion from the local model (OpenAI-compatible /v1/chat/completions,
    which both Ollama and llamafile speak), or "" on any failure or if no model is up
    yet. Kicks provisioning when nothing is running."""
    base = endpoint()
    if not base:
        return ""
    with _lock:
        model = _state["model"] or "local"
    body = json.dumps({
        "model": model,
        "messages": [{"role": "user", "content": prompt}],
        "temperature": 0.1,
        "max_tokens": int(max_tokens),
        "stream": False,
    }).encode("utf-8")
    try:
        req = Request(base + "/v1/chat/completions", data=body, headers={"Content-Type": "application/json"})
        resp = json.loads(urlopen(req, timeout=CHAT_TIMEOUT, context=_ctx()).read().decode("utf-8", "replace"))
        return str(((resp.get("choices") or [{}])[0].get("message") or {}).get("content") or "").strip()
    except Exception:
        return ""
