"""Notifications: what Bagholder tells the person about while they are not looking.

Every event starts here on the server, since the server is what keeps watching while
the page sits in a background tab or is closed: an order read back filled, rejected
or expired, a bracket that ended or a leg Wealthsimple refused, a session that
expired, a sync that keeps failing, a new release, a new disclosure on a ticker the
book knows. Each becomes one row, keyed so the same event is never told twice.

Where the computer running Bagholder has a desktop, the server posts the row itself
as the system's own notification, under Bagholder's name and icon: on a Mac through
a small app bundle it builds for itself in the home folder (an applet compiled with
the system's own `osacompile`, so the banner is Bagholder's, not a browser's or Script
Editor's), on Windows through a toast registered under Bagholder's name, on Linux
through the desktop's notification service. Where it has none (the container), every
open page listens on a stream and shows the row through the browser's Notification
API instead. Which kinds are told is a setting on the server, so every browser that
opens the app agrees.
"""
from __future__ import annotations

import hashlib
import json
import os
import queue
import shutil
import subprocess
import sys
import tempfile
import threading
from datetime import datetime, timedelta, timezone
from pathlib import Path

import store

KINDS = ("fills", "problems", "connection", "updates", "disclosures")
SETTINGS_KEY = "notify_settings"
RECENT_MINUTES = 10      # a page that connects is told what happened this recently, never a backlog
HEARTBEAT_SEC = 15.0     # a comment on the stream this often keeps the connection through proxies and sleeps
MODE_ENV = "BAGHOLDER_NOTIFY"   # "browser": never post from this process (a scratch copy beside the real one), the page shows them
APP_NAME = "Bagholder"
MAC_BUNDLE_ID = "com.bagholder.notifier"

_cond = threading.Condition()
_queue = queue.Queue()
_worker = None
_worker_lock = threading.Lock()
_url = "http://127.0.0.1:8765/"
_icon = Path(__file__).resolve().parent / "favicon.png"


def configure(url=None, icon=None):
    """What a banner opens when clicked, and the icon it carries."""
    global _url, _icon
    if url:
        _url = str(url)
    if icon:
        _icon = Path(icon)


def settings():
    """Which kinds are on: every kind off until it is turned on from the menu."""
    try:
        raw = json.loads(store.get_meta(SETTINGS_KEY) or "{}")
    except (ValueError, TypeError):
        raw = {}
    if not isinstance(raw, dict):
        raw = {}
    return {k: bool(raw.get(k)) for k in KINDS}


def set_settings(patch):
    """Turn kinds on or off; unknown keys and non-booleans are ignored. Returns the settings."""
    cur = settings()
    for k, v in (patch or {}).items():
        if k in KINDS and isinstance(v, bool):
            cur[k] = v
    store.set_meta(SETTINGS_KEY, json.dumps(cur))
    return cur


def status():
    """The kinds, and how they are delivered from here: `native` names the channel
    ("mac", "windows", "linux") or is empty where the page must show them."""
    out = settings()
    out["native"] = native_channel()
    return out


def native_channel():
    """The system's own notifications from this process, when the computer has a
    desktop to show them on; empty where the page is the only way (the container, a
    headless box, or a copy told to stand aside with BAGHOLDER_NOTIFY=browser)."""
    if (os.environ.get(MODE_ENV) or "").strip().lower() in ("browser", "off", "0", "none"):
        return ""
    if sys.platform == "darwin":
        return "mac" if shutil.which("osascript") else ""
    if sys.platform == "win32":
        return "windows" if (shutil.which("powershell") or shutil.which("pwsh")) else ""
    if shutil.which("notify-send") and (os.environ.get("DISPLAY") or os.environ.get("WAYLAND_DISPLAY")):
        return "linux"
    return ""


def emit(kind, key, title, body, extra=None):
    """One notification, if its kind is on and this key has not been told before.
    Returns the row, or None."""
    if kind not in KINDS or not settings().get(kind):
        return None
    return _post(kind, key, title, body, extra)


def test_notification():
    """The row the menu's test sends, whatever the kinds say: the way to see one arrive."""
    stamp = datetime.now(timezone.utc).strftime("%Y%m%d%H%M%S%f")
    return _post("test", "test:" + stamp, APP_NAME, "Notifications reach you here.")


def _post(kind, key, title, body, extra=None):
    channel = native_channel()
    # posted from here, the row is the server's own to show: seen from the start, so no page shows it too
    row = store.add_notification(kind, key, title, body, extra, seen=bool(channel))
    if not row:
        return None
    if channel:
        _enqueue(row, channel)
    else:
        with _cond:
            _cond.notify_all()
    return row


def _enqueue(row, channel):
    global _worker
    with _worker_lock:
        if _worker is None or not _worker.is_alive():
            _worker = threading.Thread(target=_work, name="bagholder-notify", daemon=True)
            _worker.start()
    _queue.put((row, channel))


def _work():
    while True:
        row, channel = _queue.get()
        try:
            ok = deliver(channel, row["title"], row["body"])
        except Exception as e:
            ok = False
            sys.stderr.write("bagholder notify: %s\n" % (str(e) or e.__class__.__name__))
        if not ok:
            sys.stderr.write("bagholder notify: %s not shown (%s)\n" % (row["title"], channel))


def deliver(channel, title, body):
    """Post one notification through the system; True when the system took it."""
    if channel == "mac":
        return _mac_deliver(title, body)
    if channel == "windows":
        return _windows_deliver(title, body)
    if channel == "linux":
        return _linux_deliver(title, body)
    return False


# --- macOS: an applet of Bagholder's own, so the banner carries its name and icon ---

MAC_SCRIPT = '''on run
	set t to system attribute "BAGHOLDER_TITLE"
	if t is "" then
		open location "%s"
	else
		display notification (system attribute "BAGHOLDER_BODY") with title t
	end if
end run
'''
# launched by a click on its banner (no title in the environment), the applet opens the app


def _mac_script():
    return MAC_SCRIPT % _url


def mac_app_path():
    return Path(store.home()) / (APP_NAME + ".app")


def mac_app():
    """The applet, built once into the home folder and again whenever its script, the
    app's address or the icon changes. None when it cannot be built."""
    app = mac_app_path()
    stamp = app / "Contents" / "Resources" / "bagholder.stamp"
    want = _mac_stamp()
    try:
        if (app / "Contents" / "MacOS" / "applet").exists() and stamp.read_text() == want:
            return app
    except OSError:
        pass
    try:
        return _mac_build(app, want)
    except Exception as e:
        sys.stderr.write("bagholder notify: the notifier app could not be built: %s\n" % (str(e) or e.__class__.__name__))
        return None


def _mac_stamp():
    h = hashlib.sha1(_mac_script().encode("utf-8"))
    try:
        h.update(_icon.read_bytes())
    except OSError:
        pass
    return h.hexdigest()


def _mac_build(app, stamp):
    if not shutil.which("osacompile"):
        return None
    work = Path(tempfile.mkdtemp(prefix="bagholder-notifier-"))
    try:
        script = work / "notifier.applescript"
        script.write_text(_mac_script())
        built = work / (APP_NAME + ".app")
        subprocess.run(["osacompile", "-o", str(built), str(script)], check=True, capture_output=True, timeout=60)
        plist = built / "Contents" / "Info.plist"
        subprocess.run(["plutil", "-replace", "CFBundleIdentifier", "-string", MAC_BUNDLE_ID, str(plist)], check=True, capture_output=True, timeout=30)
        subprocess.run(["plutil", "-replace", "CFBundleDisplayName", "-string", APP_NAME, str(plist)], check=True, capture_output=True, timeout=30)
        icns = _mac_icon(work)
        if icns:
            res = built / "Contents" / "Resources"
            shutil.copyfile(icns, res / "applet.icns")
            car = res / "Assets.car"
            if car.exists():
                car.unlink()
            subprocess.run(["plutil", "-remove", "CFBundleIconName", str(plist)], capture_output=True, timeout=30)
        (built / "Contents" / "Resources" / "bagholder.stamp").write_text(stamp)
        if shutil.which("codesign"):
            # sealed last, with the stamp inside the seal
            subprocess.run(["codesign", "--force", "--sign", "-", str(built)], capture_output=True, timeout=60)
        if app.exists():
            shutil.rmtree(app)
        app.parent.mkdir(parents=True, exist_ok=True)
        shutil.move(str(built), str(app))
        return app
    finally:
        shutil.rmtree(work, ignore_errors=True)


def _mac_icon(work):
    """The app's icon from the favicon, through the system's own icon tools; None when they are missing."""
    if not (shutil.which("sips") and shutil.which("iconutil") and _icon.exists()):
        return None
    iconset = work / "icon.iconset"
    iconset.mkdir()
    for size, names in ((16, ("icon_16x16.png",)), (32, ("icon_16x16@2x.png", "icon_32x32.png")), (64, ("icon_32x32@2x.png",)),
                        (128, ("icon_128x128.png",)), (256, ("icon_128x128@2x.png", "icon_256x256.png")), (512, ("icon_256x256@2x.png", "icon_512x512.png"))):
        first = iconset / names[0]
        subprocess.run(["sips", "-z", str(size), str(size), str(_icon), "--out", str(first)], check=True, capture_output=True, timeout=30)
        for other in names[1:]:
            shutil.copyfile(first, iconset / other)
    icns = work / "icon.icns"
    subprocess.run(["iconutil", "-c", "icns", str(iconset), "-o", str(icns)], check=True, capture_output=True, timeout=30)
    return icns if icns.exists() else None


def _mac_deliver(title, body):
    app = mac_app()
    if app:
        r = subprocess.run(["open", "-n", "-W", "--env", "BAGHOLDER_TITLE=" + title, "--env", "BAGHOLDER_BODY=" + body, str(app)], capture_output=True, timeout=60)
        if r.returncode == 0:
            return True
        sys.stderr.write("bagholder notify: the notifier app refused: %s\n" % (r.stderr.decode("utf-8", "replace").strip() or r.returncode))
    # without the app: the system's plain notification, under Script Editor's name
    env = dict(os.environ, BAGHOLDER_TITLE=title, BAGHOLDER_BODY=body)
    r = subprocess.run(["osascript", "-e", 'display notification (system attribute "BAGHOLDER_BODY") with title (system attribute "BAGHOLDER_TITLE")'], env=env, capture_output=True, timeout=60)
    return r.returncode == 0


# --- Windows: a toast under an app id registered as Bagholder, with its icon ---

WINDOWS_APP_ID = APP_NAME
WINDOWS_SCRIPT = r'''$ErrorActionPreference = 'Stop'
[Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime] | Out-Null
[Windows.Data.Xml.Dom.XmlDocument, Windows.Data.Xml.Dom.XmlDocument, ContentType = WindowsRuntime] | Out-Null
$xml = New-Object Windows.Data.Xml.Dom.XmlDocument
$xml.LoadXml('<toast activationType="protocol" launch="__URL__"><visual><binding template="ToastGeneric"><text></text><text></text></binding></visual></toast>')
$t = $xml.GetElementsByTagName('text')
$t.Item(0).AppendChild($xml.CreateTextNode($env:BAGHOLDER_TITLE)) | Out-Null
$t.Item(1).AppendChild($xml.CreateTextNode($env:BAGHOLDER_BODY)) | Out-Null
$toast = New-Object Windows.UI.Notifications.ToastNotification $xml
[Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier('__APP__').Show($toast)
'''
_windows_registered = False


def _windows_register():
    """The app id a toast is shown under, with Bagholder's name and icon, under the
    person's own registry hive (no elevation); once per process."""
    global _windows_registered
    if _windows_registered:
        return
    import winreg  # Windows only
    key = winreg.CreateKey(winreg.HKEY_CURRENT_USER, r"Software\Classes\AppUserModelId\%s" % WINDOWS_APP_ID)
    try:
        winreg.SetValueEx(key, "DisplayName", 0, winreg.REG_SZ, APP_NAME)
        if _icon.exists():
            winreg.SetValueEx(key, "IconUri", 0, winreg.REG_SZ, str(_icon))
    finally:
        winreg.CloseKey(key)
    _windows_registered = True


def windows_script():
    return WINDOWS_SCRIPT.replace("__URL__", _url).replace("__APP__", WINDOWS_APP_ID)


def _windows_deliver(title, body):
    try:
        _windows_register()
    except Exception as e:
        sys.stderr.write("bagholder notify: app id not registered: %s\n" % (str(e) or e.__class__.__name__))
    shell = shutil.which("powershell") or shutil.which("pwsh") or "powershell"
    env = dict(os.environ, BAGHOLDER_TITLE=title, BAGHOLDER_BODY=body)
    r = subprocess.run([shell, "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-WindowStyle", "Hidden", "-Command", windows_script()], env=env, capture_output=True, timeout=60)
    return r.returncode == 0


# --- Linux: the desktop's notification service ---

def _linux_deliver(title, body):
    cmd = ["notify-send", "--app-name=" + APP_NAME]
    if _icon.exists():
        cmd.append("--icon=" + str(_icon))
    r = subprocess.run(cmd + [title, body], capture_output=True, timeout=60)
    return r.returncode == 0


# --- the page's channel, where the server has none ---

def stream(after=None, alive=lambda: True, heartbeat=None):
    """text/event-stream chunks: the rows of the last few minutes not yet seen (and
    after the given id when the page brings one), then each new row as it is made,
    with a comment between them every `heartbeat` seconds so the connection is kept.
    Ends when `alive()` says no, or when the reader goes."""
    heartbeat = HEARTBEAT_SEC if heartbeat is None else heartbeat
    since = (datetime.now(timezone.utc) - timedelta(minutes=RECENT_MINUTES)).strftime("%Y-%m-%dT%H:%M:%SZ")
    last = int(after or 0)
    yield ": bagholder\n\n"
    while alive():
        with _cond:
            rows = store.list_notifications(after_id=last, since=since, unseen=True)
            if not rows:
                _cond.wait(heartbeat)
        if not rows:
            yield ": ping\n\n"
            continue
        for r in rows:
            last = max(last, int(r["id"]))
            yield "id: %d\ndata: %s\n\n" % (r["id"], json.dumps(r))
