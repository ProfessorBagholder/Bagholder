"""The Wealthsimple session, sync and orders refresh of the Rust server against
`bagholder.py`, end to end, on a mock Wealthsimple.

Both servers run on their own copy of a database with a fake session, pointed
at an in-process mock that answers OAuth and GraphQL by operation name with
synthetic data. Each is driven through the same phases (boot, sync, refresh,
orders refresh, a GraphQL error, a refused login); the databases, the session
file, the status and model payloads and the requests each made are compared.

    cargo build -p bagholder-server
    PYTHONPATH=~/.bagholder/pylibs python3 crates/server/synctest.py [source.db]

Nothing here talks to Wealthsimple. Exits nonzero on any difference.
"""
import json
import os
import re
import shutil
import signal
import sqlite3
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.request
from datetime import date, timedelta
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
BIN = os.environ.get("BAGHOLDER_BIN") or os.path.join(ROOT, "target", "debug", "bagholder")
SCRATCH = os.environ.get("SYNCTEST_DIR") or os.path.join(os.path.dirname(ROOT), "synctest")
SOURCE_DB = sys.argv[1] if len(sys.argv) > 1 else os.path.expanduser("~/.bagholder/bagholder.db")
PORTS = {"py": 8831, "rs": 8832}

IDENTITY = "identity-mock-0001"
CLIENT_ID = "0123456789abcdef0123"


# ---------------------------------------------------------------- mock data

def money(a, c="CAD"):
    return {"amount": str(a), "currency": c, "__typename": "Money"}


ACCOUNTS = [
    {"id": "mock-tfsa", "nickname": "Mock TFSA", "unifiedAccountType": "SELF_DIRECTED_TFSA", "currency": "CAD",
     "status": "open", "type": "ca_tfsa", "custodianAccounts": [{"id": "H-TFSA1", "branch": "TR"}],
     "accountFeatures": [{"name": "MARGIN_BOOST", "enabled": True, "functional": True,
                          "metadata": {"targetMarginAccountId": "H-MARG1"}}],
     "financials": {"currentCombined": {"id": "x1", "netLiquidationValue": money("12345.67")}}},
    {"id": "mock-margin", "nickname": "Mock Margin", "unifiedAccountType": "SELF_DIRECTED_MARGIN", "currency": "CAD",
     "status": "open", "type": "ca_non_registered", "custodianAccounts": [{"id": "H-MARG1", "branch": "TR"}],
     "accountFeatures": [],
     "financials": {"currentCombined": {"id": "x2", "netLiquidationValue": money("50000.5")}}},
    {"id": "mock-cash-usd", "nickname": "Mock Margin", "unifiedAccountType": "SELF_DIRECTED_NON_REGISTERED", "currency": "USD",
     "status": "open", "type": "ca_non_registered", "custodianAccounts": [{"id": "H-CASH1", "branch": "TR"}],
     "accountFeatures": [],
     "financials": {"currentCombined": {"id": "x3", "netLiquidationValue": money("777", "USD")}}},
]


def activity(aid, n, **kw):
    base = {"accountId": aid, "canonicalId": "mock-%s-%d" % (aid, n), "status": "POSTED", "currency": "CAD",
            "amountSign": "negative", "fees": "0", "occurredAt": "2026-09-%02dT14:3%d:00.000000+00:00" % (1 + n % 14, n % 10),
            "identityId": IDENTITY, "__typename": "ActivityFeedItem"}
    base.update(kw)
    return base


def activities_for(aid):
    return [
        [
            activity(aid, 1, type="DIY_BUY", subType="MARKET_ORDER", assetSymbol="MOCKA", securityId="sec-s-mocka",
                     assetQuantity="10", amount="105.5"),
            activity(aid, 2, type="DIY_SELL", subType="LIMIT_ORDER", assetSymbol="MOCKA", securityId="sec-s-mocka",
                     assetQuantity="4", amount="48.25", amountSign="positive"),
            activity(aid, 3, type="DIVIDEND", subType=None, status=None, assetSymbol="MOCKA", securityId="sec-s-mocka",
                     amount="1.23", amountSign="positive"),
        ],
        [
            activity(aid, 4, type="OPTIONS_BUY", subType="BUY_TO_OPEN", assetSymbol="MOCKB", securityId="sec-o-mockb",
                     assetQuantity="2", amount="130", currency="USD", contractType="CALL", strikePrice="5",
                     expiryDate="2026-12-18"),
            activity(aid, 5, type="OPTIONS_SELL", subType="SELL_TO_CLOSE", assetSymbol="MOCKB", securityId="sec-o-mockb",
                     assetQuantity="1", amount="90", amountSign="positive", currency="USD", contractType="CALL",
                     strikePrice="5", expiryDate="2026-12-18"),
            activity(aid, 6, type="DEPOSIT", subType="EFT", amount="1000", amountSign="positive"),
            activity(aid, 7, type="DIY_BUY", subType="MARKET_ORDER", assetSymbol="MOCKA", status="CANCELLED",
                     securityId="sec-s-mocka", assetQuantity="1", amount="10"),
        ],
    ]


SECURITIES = {
    "sec-s-mocka": {"id": "sec-s-mocka", "currency": "CAD",
                    "stock": {"symbol": "MOCKA", "name": "Mock Alpha Corp", "primaryExchange": "TSX", "primaryMic": "XTSE"}},
    "sec-o-mockb": {"id": "sec-o-mockb", "currency": "USD",
                    "stock": {"symbol": "MOCKB", "name": "Mock Beta Call", "primaryExchange": "", "primaryMic": ""},
                    "optionDetails": {"underlyingSecurity": {"id": "sec-s-mockb"}}},
    "sec-s-mockb": {"id": "sec-s-mockb", "currency": "USD",
                    "stock": {"symbol": "MOCKB", "name": "Mock Beta Inc", "primaryExchange": "NASDAQ", "primaryMic": "XNAS"}},
}

FEED = [
    [{"id": "mock-ext-filled", "orderId": "order-f1", "canonicalAccountId": "mock-margin", "createdAtUtc": "2026-09-15T14:00:00Z",
      "status": "FILLED", "side": "BUY_QUANTITY", "executionType": "LIMIT", "submittedQuantity": "5", "limitPrice": "10.5",
      "stopPrice": None, "averageFillPrice": "10.4", "securityCurrency": "cad", "securityId": "sec-s-mocka", "symbol": "MOCKA",
      "security": {"id": "sec-s-mocka", "stock": {"symbol": "MOCKA", "name": "Mock Alpha Corp"}}},
     {"id": "mock-ext-cancelled", "orderId": "order-c1", "canonicalAccountId": "mock-tfsa", "createdAtUtc": "2026-09-15T15:00:00Z",
      "status": "CANCELLED", "side": "SELL_QUANTITY", "executionType": "STOP_LIMIT", "submittedQuantity": "3", "limitPrice": "9",
      "stopPrice": "9.1", "averageFillPrice": None, "securityCurrency": "CAD", "securityId": "sec-s-mocka", "symbol": "MOCKA",
      "security": {"id": "sec-s-mocka", "stock": {"symbol": "MOCKA", "name": "Mock Alpha Corp"}}}],
    [{"id": "mock-ext-pending", "orderId": "order-p1", "canonicalAccountId": "mock-margin", "createdAtUtc": "2026-09-16T13:00:00Z",
      "status": "SUBMITTED", "side": "BUY_QUANTITY", "executionType": "LIMIT", "submittedQuantity": "7", "limitPrice": "1.25",
      "stopPrice": None, "averageFillPrice": None, "securityCurrency": "USD", "securityId": "sec-o-mockb", "symbol": "MOCKB",
      "security": {"id": "sec-o-mockb", "stock": {"symbol": "MOCKB", "name": "Mock Beta Call"}}}],
]


def nav_page(variables, key):
    start, end = variables.get("startDate"), variables.get("endDate")
    cursor = variables.get("cursor")
    dates = [start] if not cursor else [end]
    has_next = (not cursor) and start != end
    edges = [{"node": {"date": d, "netLiquidationValue": money(1000 + int(d[:4]) % 100 + int(d[5:7]) * 10 + int(d[8:10])),
                       "netDeposits": money(900)}} for d in dates]
    hist = {"edges": edges, "pageInfo": {"hasNextPage": has_next, "endCursor": "nav-2" if has_next else None}}
    return {key: {"financials": {"historicalDaily": hist}}}


class Mock:
    def __init__(self):
        self.lock = threading.Lock()
        self.reset()

    def reset(self):
        self.log = []
        self.phase = "boot"
        self.mode = "ok"
        self.token_n = 0

    def answer_graphql(self, op, v):
        if self.mode == "refused":
            return 401, {"error": "unauthorized"}
        if self.mode == "gql_error" and op == "FetchAllAccountFinancials":
            return 200, {"errors": [{"message": "Mock failure for testing", "path": ["identity"]}], "data": None}
        if op == "FetchAllAccountFinancials":
            if not v.get("cursor"):
                page, more = ACCOUNTS[:2], True
            else:
                page, more = ACCOUNTS[2:], False
            return 200, {"data": {"identity": {"id": IDENTITY, "accounts": {
                "edges": [{"cursor": a["id"], "node": a} for a in page],
                "pageInfo": {"hasNextPage": more, "endCursor": "acc-2" if more else None}}}}}
        if op == "FetchActivityFeedItems":
            aid = (v.get("condition") or {}).get("accountIds", [""])[0]
            pages = activities_for(aid)
            i = 1 if v.get("cursor") == "act-2" else 0
            more = i == 0
            return 200, {"data": {"activityFeedItems": {"edges": [{"node": n} for n in pages[i]],
                                                        "pageInfo": {"hasNextPage": more, "endCursor": "act-2" if more else None}}}}
        if op == "FetchAccountsWithBalance":
            out = []
            for aid in v.get("ids") or []:
                acc = next((a for a in ACCOUNTS if a["id"] == aid), None)
                if not acc:
                    continue
                ca = acc["custodianAccounts"][0]["id"]
                out.append({"id": aid, "custodianAccounts": [{"id": ca, "financials": {"balance": [
                    {"securityId": "sec-s-mocka", "quantity": "6"},
                    {"securityId": "sec-c-cad", "quantity": "1234.56"},
                ]}}]})
            return 200, {"data": {"accounts": out}}
        if op == "FetchAccountCurrentMarginBuyingPowerV2":
            return 200, {"data": {"account": {"id": v.get("accountId"), "financials": {"current": {"marginV3": {"trading": {
                "buyingPower": {"__typename": "BuyingPowerMetricAvailable", "total": money("4321.09")}}}}}}}}
        if op == "IdentityHistoricalFinancialsQuery":
            return 200, {"data": nav_page(v, "identity")}
        if op == "FetchAccountHistoricalFinancials":
            return 200, {"data": nav_page(v, "account")}
        if op == "FetchSecurities":
            return 200, {"data": {"securities": [SECURITIES.get(i) for i in v.get("ids") or []]}}
        if op == "FetchSecurity":
            return 200, {"data": {"security": SECURITIES.get(v.get("securityId"))}}
        if op == "OrderServiceExtendedOrderFeed":
            i = 1 if v.get("cursor") == "feed-2" else 0
            more = i == 0
            return 200, {"data": {"identity": {"id": IDENTITY, "orderServiceExtendedOrderFeed": {
                "edges": [{"cursor": n["id"], "node": n} for n in FEED[i]],
                "pageInfo": {"hasNextPage": more, "endCursor": "feed-2" if more else None}}}}}
        if op == "FetchSoOrdersExtendedOrder":
            ext = v.get("externalId")
            if ext == "mock-ext-pending":
                o = {"status": "PARTIALLY_FILLED", "filledQuantity": "3", "averageFilledPrice": "1.2", "submittedQuantity": "7",
                     "limitPrice": "1.3", "timeInForce": "day", "securityCurrency": "usd", "canonicalAccountId": "mock-margin",
                     "securityId": "sec-o-mockb", "orderType": "buy_quantity", "submittedAtUtc": "2026-09-16T13:00:01Z",
                     "expiredAtUtc": "2026-09-16T20:00:00Z", "firstFilledAtUtc": "2026-09-16T13:05:00Z", "lastFilledAtUtc": "2026-09-16T13:06:00Z"}
            else:
                o = {"status": "CANCELLED", "rejectionCause": "mock cancelled", "submittedQuantity": None}
            return 200, {"data": {"soOrdersExtendedOrder": o}}
        return 200, {"errors": [{"message": "unknown operation " + op}], "data": None}

    def answer_oauth(self, method, path, body):
        if path.endswith("/token/info"):
            if self.mode in ("refused", "info401"):
                return 401, {"error": "invalid_token"}
            return 200, {"resource_owner_id": "ro-1", "identity_canonical_id": IDENTITY, "email": "mock@example.test",
                         "application_uid": CLIENT_ID, "expires_in": 1700}
        if path.endswith("/token"):
            if self.mode == "refused":
                return 401, {"error": "invalid_grant", "error_description": "The provided authorization grant is invalid"}
            with self.lock:
                self.token_n += 1
                n = self.token_n
            return 200, {"access_token": "mock-access-%d" % n, "refresh_token": "mock-refresh-%d" % n,
                         "expires_in": 1800, "token_type": "Bearer", "identity_canonical_id": IDENTITY}
        return 404, {"error": "not_found"}


MOCK = Mock()


class MockHandler(BaseHTTPRequestHandler):
    def log_message(self, *a):
        pass

    def _reply(self, code, obj):
        raw = json.dumps(obj).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(raw)))
        self.end_headers()
        self.wfile.write(raw)

    def _handle(self, method):
        n = int(self.headers.get("Content-Length") or 0)
        raw = self.rfile.read(n) if n else b""
        try:
            body = json.loads(raw) if raw else None
        except ValueError:
            body = None
        path = self.path.split("?", 1)[0]
        hdrs = {k.lower(): v for k, v in self.headers.items()
                if k.lower() in ("authorization", "x-ws-profile", "x-wealthsimple-client", "x-ws-api-version", "x-ws-locale",
                                 "x-platform-os", "x-ws-device-id", "x-ws-session-id", "origin", "referer", "content-type", "accept")}
        entry = {"phase": MOCK.phase, "method": method, "path": path, "headers": hdrs}
        if path.endswith("/graphql"):
            op = (body or {}).get("operationName")
            v = (body or {}).get("variables") or {}
            entry.update(op=op, variables=v, query_len=len((body or {}).get("query") or ""))
            code, out = MOCK.answer_graphql(op, v)
        else:
            entry.update(op=path, variables=body)
            code, out = MOCK.answer_oauth(method, path, body)
        entry["status"] = code
        with MOCK.lock:
            MOCK.log.append(entry)
        self._reply(code, out)

    def do_GET(self):
        self._handle("GET")

    def do_POST(self):
        self._handle("POST")


# ---------------------------------------------------------------- servers

PY_LAUNCHER = r'''
import sys, os
sys.path.insert(0, %(root)r)
import bagholder
base = os.environ["BAGHOLDER_WS_BASE"].rstrip("/")
bagholder.OAUTH = base + "/oauth"
bagholder.GRAPHQL = base + "/graphql"
bagholder.LOGIN_URL = base + "/app/login"
bagholder.main()
'''


def fake_session(expired=False):
    return {
        "access_token": "mock-access-0",
        "refresh_token": "mock-refresh-0",
        "expires_at": "2020-01-01T00:00:00.000Z" if expired else "2099-01-01T00:00:00.000Z",
        "identity_canonical_id": IDENTITY,
        "client_id": CLIENT_ID,
        "wssdi": "device-mock",
        "session_id": "session-mock",
        "user_agent": "SyncTest/1.0",
        "email": "mock@example.test",
    }


def copy_db(dst):
    src = sqlite3.connect("file:%s?mode=ro" % SOURCE_DB, uri=True)
    out = sqlite3.connect(dst)
    src.backup(out)
    out.close()
    src.close()


def start(kind, mock_base, fresh=False):
    home = os.path.join(SCRATCH, kind + ("-fresh" if fresh else ""))
    shutil.rmtree(home, ignore_errors=True)
    os.makedirs(home)
    if not fresh:
        copy_db(os.path.join(home, "bagholder.db"))
    with open(os.path.join(home, "session.json"), "w") as fh:
        json.dump(fake_session(expired=fresh), fh, indent=2)
    env = dict(os.environ, BAGHOLDER_CHILD="1", BAGHOLDER_NO_UPDATE="1", BAGHOLDER_NO_BROWSER="1",
               BAGHOLDER_DRY_ORDERS="1", BAGHOLDER_HOME=home, BAGHOLDER_PORT=str(PORTS[kind]),
               BAGHOLDER_WS_BASE=mock_base)
    if kind == "py":
        launcher = os.path.join(SCRATCH, "py_launcher.py")
        with open(launcher, "w") as fh:
            fh.write(PY_LAUNCHER % {"root": ROOT})
        env["PYTHONPATH"] = os.path.expanduser("~/.bagholder/pylibs")
        cmd = [sys.executable, launcher]
    else:
        cmd = [BIN]
    log = open(os.path.join(SCRATCH, kind + ".log"), "w")
    proc = subprocess.Popen(cmd, env=env, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT)
    return proc, home


def call(kind, path, body=None):
    port = PORTS[kind]
    data = json.dumps(body if body is not None else {}).encode() if body is not None or path.startswith("/api/") and False else None
    if body is not None:
        data = json.dumps(body).encode()
    req = urllib.request.Request("http://127.0.0.1:%d%s" % (port, path), data=data, method="POST" if data is not None else "GET")
    req.add_header("Host", "127.0.0.1:%d" % port)
    if data is not None:
        req.add_header("X-Bagholder", "1")
        req.add_header("Content-Type", "application/json")
    try:
        with urllib.request.urlopen(req, timeout=180) as r:
            return json.loads(r.read())
    except urllib.error.HTTPError as e:
        try:
            return {"_http": e.code, "body": json.loads(e.read() or b"{}")}
        except ValueError:
            return {"_http": e.code}


def wait_up(kind, proc):
    for _ in range(600):
        if proc.poll() is not None:
            raise SystemExit("%s exited early (%s); see %s.log" % (kind, proc.returncode, kind))
        try:
            return call(kind, "/api/status")
        except Exception:
            time.sleep(0.1)
    raise SystemExit(kind + " did not come up")


def settle(kind, quiet=2.0, limit=240):
    """Until syncing and listingsFilling are false and no mock request has
    arrived for `quiet` seconds."""
    t0 = time.time()
    while time.time() - t0 < limit:
        st = call(kind, "/api/status")
        with MOCK.lock:
            n = len(MOCK.log)
            last = MOCK.last_at if hasattr(MOCK, "last_at") else 0
        if not st.get("syncing") and not st.get("listingsFilling"):
            time.sleep(quiet)
            with MOCK.lock:
                n2 = len(MOCK.log)
            st2 = call(kind, "/api/status")
            if n2 == n and not st2.get("syncing") and not st2.get("listingsFilling"):
                return st2
        else:
            time.sleep(0.2)
    raise SystemExit(kind + " did not settle")


PHASES = []


def run(kind, mock_base, fresh=False):
    MOCK.reset()
    out = {"responses": {}, "status": {}}
    if fresh:
        # an empty store and an expired login whose token/info is refused: boot refreshes, sync pulls everything
        MOCK.mode = "info401"
    proc, home = start(kind, mock_base, fresh)
    try:
        wait_up(kind, proc)
        out["status"]["boot"] = settle(kind)
        MOCK.mode = "ok"

        MOCK.phase = "sync"
        out["responses"]["sync"] = call(kind, "/api/sync", {})
        time.sleep(0.5)
        out["status"]["sync"] = settle(kind)

        MOCK.phase = "refresh"
        out["responses"]["refresh"] = call(kind, "/api/refresh", {})
        out["status"]["refresh"] = settle(kind, quiet=0.5)

        MOCK.phase = "orders1"
        out["responses"]["orders1"] = call(kind, "/api/orders/refresh", {})
        MOCK.phase = "orders2"
        out["responses"]["orders2"] = call(kind, "/api/orders/refresh", {})
        out["status"]["orders"] = settle(kind, quiet=0.5)
        out["model"] = call(kind, "/api/model")
        with open(os.path.join(home, "session.json")) as fh:
            out["session_ok"] = json.load(fh)

        MOCK.phase = "gql_error"
        MOCK.mode = "gql_error"
        out["responses"]["gql_error_sync"] = call(kind, "/api/sync", {})
        time.sleep(0.5)
        out["status"]["gql_error"] = settle(kind, quiet=0.5)
        out["responses"]["gql_error_orders"] = call(kind, "/api/orders/refresh", {})

        MOCK.phase = "refused"
        MOCK.mode = "refused"
        out["responses"]["refused_refresh"] = call(kind, "/api/refresh", {})
        out["status"]["refused_refresh"] = settle(kind, quiet=0.5)
        out["responses"]["refused_sync"] = call(kind, "/api/sync", {})
        time.sleep(0.5)
        out["status"]["refused_sync"] = settle(kind, quiet=0.5)
        out["responses"]["refused_orders"] = call(kind, "/api/orders/refresh", {})
        out["responses"]["refused_refresh_again"] = call(kind, "/api/refresh", {})
        out["status"]["end"] = settle(kind, quiet=0.5)
    finally:
        MOCK.phase = "stop"
        proc.send_signal(signal.SIGTERM)
        try:
            proc.wait(timeout=20)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.wait()
    with open(os.path.join(home, "session.json")) as fh:
        out["session_end"] = json.load(fh)
    out["requests"] = list(MOCK.log)
    out["db"] = dump_db(os.path.join(home, "bagholder.db"))
    return out


# ---------------------------------------------------------------- comparison

TABLES = ["activities", "accounts", "balances", "nav_history", "securities", "margin", "orders", "brackets",
          "notifications", "grouped_trades"]
META_KEYS = {"synced_at", "last_activity_pull", "balances_read_at", "security_id_backfill_done"}

TS = re.compile(r"\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}(:\d{2}(\.\d+)?)?(Z|[+-]\d{2}:?\d{2})?")
UUID = re.compile(r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}")
TODAY = date.today()
DAYS = {(TODAY + timedelta(days=d)).isoformat(): "<day%+d>" % d for d in (-1, 0, 1, 2)}


def norm_text(s):
    if not isinstance(s, str):
        return s
    s = TS.sub("<ts>", s)
    s = UUID.sub("<uuid>", s)
    for d, tag in DAYS.items():
        s = s.replace(d, tag)
    return s


def norm(v):
    if isinstance(v, bool) or v is None:
        return v
    if isinstance(v, (int, float)):
        return round(float(v), 6) + 0.0
    if isinstance(v, str):
        return norm_text(v)
    if isinstance(v, dict):
        return {k: norm(x) for k, x in v.items()}
    if isinstance(v, (list, tuple)):
        return [norm(x) for x in v]
    return v


def dump_db(path):
    c = sqlite3.connect(path)
    c.row_factory = sqlite3.Row
    out = {}
    have = {r[0] for r in c.execute("SELECT name FROM sqlite_master WHERE type='table'")}
    for t in TABLES:
        if t not in have:
            out[t] = None
            continue
        cols = [r[1] for r in c.execute("PRAGMA table_info(%s)" % t)]
        rows = []
        for r in c.execute("SELECT * FROM %s" % t):
            d = {k: r[k] for k in cols if k not in ("rowid",)}
            if t == "notifications" and d.get("kind") not in ("connection", "orders", "order"):
                continue
            rows.append(d)
        out[t] = rows
    out["meta"] = {r["key"]: r["value"] for r in c.execute("SELECT key, value FROM meta") if r["key"] in META_KEYS}
    c.close()
    return out


def row_key(t, d):
    # surrogate keys: autoincrement ids move with how often a table was rewritten
    drop = {"id"} if t in ("notifications", "balances", "margin") else set()
    if t == "activities" and _s(d.get("source")) != "wealthsimple":
        drop = {"id"}
    return json.dumps({k: v for k, v in norm(d).items() if k not in drop}, sort_keys=True, default=str)


def _s(v):
    return "" if v is None else str(v)


def diff(a, b, path=""):
    out = []
    if isinstance(a, dict) and isinstance(b, dict):
        for k in list(dict.fromkeys(list(a) + list(b))):
            if k not in a:
                out.append("%s/%s only rs" % (path, k))
            elif k not in b:
                out.append("%s/%s only py" % (path, k))
            else:
                out += diff(a[k], b[k], path + "/" + k)
    elif isinstance(a, list) and isinstance(b, list):
        if len(a) != len(b):
            out.append("%s len py=%d rs=%d" % (path, len(a), len(b)))
        for i, (x, y) in enumerate(zip(a, b)):
            out += diff(x, y, "%s[%d]" % (path, i))
    elif a != b:
        out.append("%s py=%s rs=%s" % (path, json.dumps(a, default=str)[:160], json.dumps(b, default=str)[:160]))
    return out


IGNORE_STATUS = {"dataVersion", "startedAt", "summaryReady", "latestVersion", "updateAvailable", "updateUrl", "canUpdate", "notify"}


def compare(py, rs, fresh=False):
    problems = {}

    def add(section, lines):
        if lines:
            problems.setdefault(section, []).extend(lines)

    # databases
    for t in TABLES:
        a, b = py["db"][t], rs["db"][t]
        if a is None or b is None:
            if a != b:
                add("db." + t, ["table present py=%s rs=%s" % (a is not None, b is not None)])
            continue
        ka = sorted(row_key(t, r) for r in a)
        kb = sorted(row_key(t, r) for r in b)
        if ka != kb:
            sa, sb = set(ka), set(kb)
            lines = ["rows py=%d rs=%d" % (len(a), len(b))]
            lines += ["only py: " + x[:400] for x in sorted(sa - sb)[:6]]
            lines += ["only rs: " + x[:400] for x in sorted(sb - sa)[:6]]
            if sa == sb:
                lines.append("same rows, different multiplicity")
            add("db." + t, lines)
    add("db.meta", diff(norm(py["db"]["meta"]), norm(rs["db"]["meta"]), "meta"))

    # session files
    for k in ("session_ok", "session_end"):
        add(k, diff(norm(py[k]), norm(rs[k]), k))

    # route responses and status
    for k in py["responses"]:
        add("response." + k, diff(norm(py["responses"][k]), norm(rs["responses"].get(k)), k))
    for k in py["status"]:
        a = {x: y for x, y in (py["status"][k] or {}).items() if x not in IGNORE_STATUS}
        b = {x: y for x, y in (rs["status"].get(k) or {}).items() if x not in IGNORE_STATUS}
        add("status." + k, diff(norm(a), norm(b), k))

    # model: the book-derived parts only (market figures move with the clock)
    keys = ("accounts", "activityCount", "syncedAt", "filters", "positionsSummary", "tradeCount", "unmatched", "cashflow", "equity") \
        if fresh else ("accounts", "activityCount", "syncedAt", "filters")
    def rows_by_content(m):
        # rows tied on date are ordered by their uuid, which is random per run
        m = json.loads(json.dumps(m or {}))
        cf = m.get("cashflow")
        if isinstance(cf, dict) and isinstance(cf.get("rows"), list):
            cf["rows"].sort(key=lambda r: json.dumps(norm(r), sort_keys=True))
        return m
    py = dict(py, model=rows_by_content(py["model"]))
    rs = dict(rs, model=rows_by_content(rs["model"]))
    for key in keys:
        if key in (py["model"] or {}) or key in (rs["model"] or {}):
            add("model." + key, diff(norm((py["model"] or {}).get(key)), norm((rs["model"] or {}).get(key)), key)[:10])

    # requests: per phase, the set of (method, op, variables, headers) and the order of distinct calls
    def req_key(e):
        return json.dumps({"method": e["method"], "op": e["op"], "variables": norm(e.get("variables")),
                           "headers": norm(e["headers"]), "status": e["status"]}, sort_keys=True)

    # listing lookups run on whichever thread first finds an unnamed security (the
    # portfolio read at start races the boot listing fill), so they are compared as
    # the ids asked for across the whole run rather than per phase
    def looked_up(reqs):
        return sorted(i for e in reqs if e["op"] in ("FetchSecurities", "FetchSecurity") and e["phase"] in ("boot", "sync")
                      for i in ((e.get("variables") or {}).get("ids") or [(e.get("variables") or {}).get("securityId")]))
    if looked_up(py["requests"]) != looked_up(rs["requests"]):
        add("requests.securities", ["py=%s rs=%s" % (looked_up(py["requests"]), looked_up(rs["requests"]))])
    lookup = lambda e: e["op"] in ("FetchSecurities", "FetchSecurity") and e["phase"] in ("boot", "sync")
    py = dict(py, requests=[e for e in py["requests"] if not lookup(e)])
    rs = dict(rs, requests=[e for e in rs["requests"] if not lookup(e)])
    phases = list(dict.fromkeys([e["phase"] for e in py["requests"]] + [e["phase"] for e in rs["requests"]]))
    for ph in phases:
        ra = [req_key(e) for e in py["requests"] if e["phase"] == ph]
        rb = [req_key(e) for e in rs["requests"] if e["phase"] == ph]
        sa, sb = set(ra), set(rb)
        lines = []
        if sa != sb:
            lines += ["only py: " + x[:500] for x in sorted(sa - sb)[:8]]
            lines += ["only rs: " + x[:500] for x in sorted(sb - sa)[:8]]
        # the sequence of distinct calls, in first-seen order
        oa = list(dict.fromkeys(ra))
        ob = list(dict.fromkeys(rb))
        if sa == sb and oa != ob:
            lines.append("same calls, different order")
            lines += ["  py: " + json.loads(x)["op"] for x in oa[:40]]
            lines += ["  rs: " + json.loads(x)["op"] for x in ob[:40]]
        if sorted(ra) != sorted(rb) and sa == sb:
            ca = {x: ra.count(x) for x in sa}
            cb = {x: rb.count(x) for x in sb}
            lines += ["count %s py=%d rs=%d" % (json.loads(x)["op"], ca[x], cb[x]) for x in sa if ca[x] != cb[x]][:8]
        add("requests." + ph, lines)
    return problems


def main():
    if not os.path.exists(BIN):
        raise SystemExit("no binary at %s; cargo build -p bagholder-server" % BIN)
    os.makedirs(SCRATCH, exist_ok=True)
    mock = ThreadingHTTPServer(("127.0.0.1", 0), MockHandler)
    threading.Thread(target=mock.serve_forever, daemon=True).start()
    base = "http://127.0.0.1:%d" % mock.server_address[1]
    only = os.environ.get("SYNCTEST_ONLY")
    total, sections, summary = 0, 0, []
    for scenario, fresh in (("copy", False), ("fresh", True)):
        results = {}
        for kind in ("py", "rs"):
            cached = os.path.join(SCRATCH, "%s-%s.json" % (kind, scenario))
            if only and only != kind and os.path.exists(cached):
                with open(cached) as fh:
                    results[kind] = json.load(fh)
                continue
            print("running", scenario, kind, flush=True)
            results[kind] = run(kind, base, fresh)
            with open(cached, "w") as fh:
                json.dump(results[kind], fh, indent=1, default=str)
        problems = compare(results["py"], results["rs"], fresh)
        for section, lines in problems.items():
            print("DIFF [%s] %s" % (scenario, section))
            for line in lines[:30]:
                print("   ", line)
        total += sum(len(v) for v in problems.values())
        sections += len(problems)
        summary.append("%s: %s, requests py=%d rs=%d" % (scenario, "clean" if not problems else "%d sections differ" % len(problems),
                                                         len(results["py"]["requests"]), len(results["rs"]["requests"])))
    mock.shutdown()
    if total:
        print("synctest: FAIL, %d differences in %d sections (%s)" % (total, sections, "; ".join(summary)))
        sys.exit(1)
    print("synctest: PASS, db tables, session, status, model, responses and requests match (%s)" % "; ".join(summary))


if __name__ == "__main__":
    main()
