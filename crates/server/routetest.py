"""Every route the Rust server answers, against the Python functions behind
the same route, on one database.

The server is started on a copy, each route is called, and the payload is
compared with what `bagholder.py` would have produced from the same store. The
gate is checked too: a request with the wrong Host, and a write without the
page's own header, must both be refused.

    cargo build --release -p bagholder-server && python3 crates/server/routetest.py
"""
import json
import os
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, ROOT)
BIN = os.path.join(ROOT, "target", "release", "bagholder")
PORT = 8791
HOST = f"127.0.0.1:{PORT}"


def call(path, body=None, host=HOST, header=True):
    url = f"http://127.0.0.1:{PORT}{path}"
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(url, data=data, method="POST" if data is not None else "GET")
    req.add_header("Host", host)
    if data is not None and header:
        req.add_header("X-Bagholder", "1")
        req.add_header("Content-Type", "application/json")
    try:
        with urllib.request.urlopen(req, timeout=30) as r:
            raw = r.read()
            return r.status, (json.loads(raw) if raw[:1] in (b"{", b"[") else raw)
    except urllib.error.HTTPError as e:
        raw = e.read()
        try:
            return e.code, json.loads(raw)
        except ValueError:
            return e.code, raw


def norm(v):
    if isinstance(v, bool):
        return v
    if isinstance(v, (int, float)):
        return round(float(v), 6) + 0.0
    if isinstance(v, dict):
        return {k: norm(x) for k, x in v.items()}
    if isinstance(v, (list, tuple)):
        return [norm(x) for x in v]
    return v


def diff(path, want, got, out, skip=()):
    if len(out) > 20:
        return
    if isinstance(want, dict) and isinstance(got, dict):
        for k in sorted(set(want) | set(got)):
            if k in skip:
                continue
            if k not in want:
                out.append(f"{path}.{k}: only rust")
            elif k not in got:
                out.append(f"{path}.{k}: only python")
            else:
                diff(f"{path}.{k}", want[k], got[k], out, skip)
    elif isinstance(want, list) and isinstance(got, list):
        if len(want) != len(got):
            out.append(f"{path}: {len(want)} py, {len(got)} rs")
            return
        for i, (a, b) in enumerate(zip(want, got)):
            diff(f"{path}[{i}]", a, b, out, skip)
    elif norm(want) != norm(got):
        out.append(f"{path}: py={want!r} rs={got!r}")


def main():
    live = os.environ.get("BAGHOLDER_DB") or os.path.expanduser("~/.bagholder/bagholder.db")
    if not os.path.exists(live):
        print(f"no database at {live}; set BAGHOLDER_DB")
        return 0

    work = tempfile.mkdtemp(prefix="routetest-")
    rshome = os.path.join(work, "rs")
    pyhome = os.path.join(work, "py")
    os.makedirs(rshome)
    os.makedirs(pyhome)
    shutil.copy(live, os.path.join(rshome, "bagholder.db"))

    env = dict(os.environ, BAGHOLDER_HOME=rshome, BAGHOLDER_PORT=str(PORT))
    proc = subprocess.Popen([BIN], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    bad = []
    try:
        for _ in range(60):
            try:
                if call("/api/status")[0] == 200:
                    break
            except Exception:
                time.sleep(0.2)
        else:
            print("the server did not start")
            return 1

        # the gate
        if call("/api/status", host="evil.example")[0] != 403:
            bad.append("a wrong Host was not refused")
        if call("/api/groups", body={"groups": []}, header=False)[0] != 403:
            bad.append("a write without the page's header was not refused")
        if call("/api/nope")[0] != 404:
            bad.append("an unknown path did not answer 404")

        # the writes, then the state they left, compared against Python's
        # readers over the same file
        call("/api/journal", body={"id": "rt:route-test", "thesis": "A note.", "grade": "B", "tags": ["x"]})
        call("/api/groups", body={"groups": [{"id": "g-route", "members": ["a|b|1.00000000"], "locked": True}]})
        call("/api/notes", body={"notes": {"rt:route-test": {"thesis": "n", "tag": "t", "grade": "A"}}})
        call("/api/watchlist/add", body={"symbol": "ROUTE.TO", "exchange": "TSX", "name": "Route Inc",
                                         "currency": "CAD", "securityId": "sec-route"})
        call("/api/tiles/set", body={"tiles": [{"symbol": "SPX", "exchange": "INDEX"},
                                               {"symbol": "GC", "exchange": "COMEX"},
                                               {"symbol": "NOPE", "exchange": "X"}]})
        call("/api/notifications/settings", body={"fills": True, "problems": False, "nonsense": True})
        call("/api/notifications/read", body={"ids": []})

        shutil.copy(os.path.join(rshome, "bagholder.db"), os.path.join(pyhome, "bagholder.db"))
        import store
        import notify as notify_py
        store.set_home(pyhome)

        checks = [
            ("/api/orders", lambda: {
                "ok": True,
                "orders": [dict(o, exchange={s["id"]: s.get("primaryExchange") or ""
                                             for s in store.list_securities()}.get(o.get("securityId") or "", ""))
                           for o in store.list_orders()],
                "brackets": store.list_brackets(), "live": False, "refreshedAt": ""}),
            ("/api/notifications", lambda: {
                "ok": True, "settings": notify_py.status(), "kinds": list(notify_py.KINDS),
                "rows": store.list_notifications(limit=50, newest=True),
                "unread": store.unread_notifications()}),
            ("/api/filings?symbol=QNC", lambda: {
                "ok": True, "symbol": store.filing_key("QNC"), "filings": store.filings("QNC"),
                "fetchedAt": store.filings_fetched_at("QNC"), "profileNo": store.sedar_profile("QNC")}),
            ("/api/shorts/feed", lambda: {"ok": True, "rows": store.all_shorts()}),
            ("/api/news/symbol?symbol=QNC&exchange=TSX", lambda: {
                "ok": True, "ids": sorted(store.news_ids("QNC", "TSX")), "fetchedAt": store.news_fetched_at()}),
            ("/api/data", lambda: dict(store.data_summary(), ok=True, sessionPresent=False)),
        ]
        for path, want_fn in checks:
            code, got = call(path)
            if code != 200:
                bad.append(f"{path}: HTTP {code}")
                continue
            want = want_fn()
            if path.startswith("/api/news"):
                got["ids"] = sorted(got["ids"])
            diff(path, want, got, bad, skip=("path",))

        # the journal, groups, notes, watchlist and tiles as Python reads them back
        j = store.journal()
        if j.get("rt:route-test") != {"thesis": "A note.", "tags": ["x"], "grade": "B"}:
            bad.append(f"journal entry: {j.get('rt:route-test')!r}")
        if [g["id"] for g in store.trade_groups()] != ["g-route"]:
            bad.append(f"groups: {store.trade_groups()!r}")
        if "rt:route-test" not in store.trade_notes():
            bad.append(f"notes: {store.trade_notes()!r}")
        if not any(w["symbol"] == "ROUTE" for w in store.list_watchlist()):
            bad.append(f"watchlist: {store.list_watchlist()!r}")
        if store.tiles() != [{"symbol": "SPX", "exchange": "INDEX"}, {"symbol": "GC", "exchange": "COMEX"}]:
            bad.append(f"tiles: {store.tiles()!r}")
        if notify_py.settings().get("fills") is not True:
            bad.append(f"notify settings: {notify_py.settings()!r}")
        store.close_all()
    finally:
        proc.send_signal(signal.SIGTERM)
        proc.wait(timeout=10)
        shutil.rmtree(work, ignore_errors=True)

    for line in bad[:20]:
        print("  " + line)
    print(f"{len(bad)} differences across the routes and the gate")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
