"""Notifications: what Bagholder tells the person about while they are not looking.

Every event starts here on the server, since the server is what keeps watching while
the page sits in a background tab: an order read back filled, rejected or expired, a
bracket that ended or a leg Wealthsimple refused, a session that expired, a sync that
keeps failing, a new release. Each becomes one row, keyed so the same event is never
told twice. Every open page listens on a stream and shows each row through the
browser's own Notification API, which is how a web app reaches the system's
notification centre on every platform; nothing here touches an operating system.
Which kinds are told is a setting on the server, so every browser that opens the app
agrees; whether a browser may show them at all is the browser's own permission,
asked once from the menu.
"""
from __future__ import annotations

import json
import threading
from datetime import datetime, timedelta, timezone

import store

KINDS = ("fills", "problems", "connection", "updates")
SETTINGS_KEY = "notify_settings"
RECENT_MINUTES = 10      # a page that connects is told what happened this recently, never a backlog
HEARTBEAT_SEC = 15.0     # a comment on the stream this often keeps the connection through proxies and sleeps

_cond = threading.Condition()


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


def emit(kind, key, title, body, extra=None):
    """One notification, if its kind is on and this key has not been told before.
    Returns the row, or None."""
    if kind not in KINDS or not settings().get(kind):
        return None
    return _post(kind, key, title, body, extra)


def test_notification():
    """The row the menu's test sends, whatever the kinds say: the way to see one arrive."""
    stamp = datetime.now(timezone.utc).strftime("%Y%m%d%H%M%S%f")
    return _post("test", "test:" + stamp, "Bagholder", "Notifications reach you here.")


def _post(kind, key, title, body, extra=None):
    row = store.add_notification(kind, key, title, body, extra)
    if row:
        with _cond:
            _cond.notify_all()
    return row


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
