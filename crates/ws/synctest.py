"""The sync helpers that do not contact Wealthsimple, both ways: the public
wording of a failure, the token expiry, the stored shape of an account, the
Margin Boost target, and the window a daily pull asks for.

The redaction is the point of most of it. A failure reaches the page, so a
token must never be inside one.

    cargo build -p bagholder-ws && python3 crates/ws/synctest.py
"""
import json
import os
import shutil
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, ROOT)
import bagholder  # noqa: E402

BIN = os.path.join(ROOT, "target", "debug", "synctool")
NOW = 1789000000.0

ERRORS = [
    "",
    "plain failure",
    "HTTPSConnectionPool: CERTIFICATE_VERIFY_FAILED",
    "unable to get local issuer certificate",
    "Authorization: Bearer abc123SECRET",
    "bearer ABCDEF",
    "access_token=abc123",
    "refresh_token: zzz999",
    "a mention of Bearer and nothing after",
    "line one\nline two",
    "   lots    of    space   ",
    "x" * 300,
]

SESSIONS = [
    {"expires_at": "2026-09-16T14:00:00.000Z"},
    {"expires_at": "2026-09-16T14:00:00Z"},
    {"expires_at": 1789000500},
    {"expires_at": "1789000500"},
    {"expires_at": ""},
    {"expires_at": None},
    {},
    {"expires_at": "not a stamp"},
    {"expires_at": "2026-09-16T14:00:00+02:00"},
]

ACCOUNTS = [
    {"id": "a1", "nickname": "Trading", "unifiedAccountType": "SELF_DIRECTED_MARGIN",
     "currency": "CAD", "status": "open", "type": "ca_margin",
     "financials": {"currentCombined": {"netLiquidationValue": {"amount": 1234.5}}},
     "custodianAccounts": [{"id": "cust-1"}]},
    {"id": "a2", "nickname": "TFSA", "unifiedAccountType": "TFSA", "currency": "CAD",
     "status": "open", "type": "ca_tfsa",
     "accountFeatures": [{"name": "MARGIN_BOOST", "enabled": True,
                          "metadata": {"targetMarginAccountId": "cust-1"}}]},
    {"id": "a3", "accountFeatures": [{"name": "MARGIN_BOOST", "enabled": False,
                                      "metadata": {"targetMarginAccountId": "cust-1"}}]},
    {"id": "a4", "accountFeatures": [{"name": "MARGIN_BOOST", "enabled": True, "functional": False,
                                      "metadata": {"targetMarginAccountId": "cust-1"}}]},
    {"id": "a5", "accountFeatures": [{"name": "OTHER", "enabled": True}]},
    {"id": "a6", "accountFeatures": [{"name": "MARGIN_BOOST", "enabled": True,
                                      "metadata": {"targetMarginAccountId": "cust-unknown"}}]},
    {"id": "a7", "financials": {}},
]


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


def main():
    payload = {"now": NOW, "errors": ERRORS, "sessions": SESSIONS, "accounts": ACCOUNTS}
    got = json.loads(subprocess.run([BIN, "pure"], input=json.dumps(payload),
                                    capture_output=True, text=True, check=True).stdout)
    bad = []
    for i, e in enumerate(ERRORS):
        want = bagholder._public_sync_error(e)
        g = got["errors"][i]
        # the redaction must leave no secret behind, whatever the wording
        for secret in ("SECRET", "ABCDEF", "abc123", "zzz999"):
            if secret in g:
                bad.append(f"error[{i}]: rust leaked {secret!r}: {g!r}")
            if secret in want:
                bad.append(f"error[{i}]: python leaked {secret!r}: {want!r}")
        if want != g:
            bad.append(f"error[{i}] {e[:30]!r}: py={want!r} rs={g!r}")
    for i, s in enumerate(SESSIONS):
        want = {"expiresAt": bagholder._expires_at_unix(s),
                "needsRefresh": bagholder.token_refresh_needed(s, NOW)}
        if norm(want) != norm(got["sessions"][i]):
            bad.append(f"session[{i}] {s}: py={want} rs={got['sessions'][i]}")
    want_slim = bagholder.slim_accounts(ACCOUNTS)
    if norm(want_slim) != norm(got["slimAccounts"]):
        for i, (w, g) in enumerate(zip(want_slim, got["slimAccounts"])):
            if norm(w) != norm(g):
                bad.append(f"slimAccount[{i}]: py={w} rs={g}")
    want_targets = [bagholder.margin_boost_target(a) for a in ACCOUNTS]
    if norm(want_targets) != norm(got["boostTargets"]):
        bad.append(f"boostTargets: py={want_targets} rs={got['boostTargets']}")

    # the pull window, against a real store
    work = tempfile.mkdtemp(prefix="synctest-")
    import store
    store.set_home(work)
    store.ensure()
    empty = json.loads(subprocess.run([BIN, "bounds"],
                                      input=json.dumps({"db": os.path.join(work, "bagholder.db")}),
                                      capture_output=True, text=True, check=True).stdout)
    want_empty = bagholder.activity_sync_bounds()
    if empty["full"] is not want_empty["full_history"] or empty["start"] != want_empty["start_date"]:
        bad.append(f"bounds(empty): py={want_empty} rs={empty}")
    store.insert_activity({"id": "ws-1", "source": "wealthsimple", "canonicalId": "ws-1",
                           "transactionDate": "2026-05-04", "occurredAt": "2026-05-04T15:00:00Z",
                           "symbol": "AAA", "quantity": 1, "unitPrice": 1, "netCashAmount": -1,
                           "activitySubType": "BUY", "category": "trade"})
    store.insert_local({"id": "csv-1", "source": "csv", "transactionDate": "2026-09-01",
                        "occurredAt": "2026-09-01", "symbol": "BBB", "quantity": 1,
                        "unitPrice": 1, "netCashAmount": -1, "activitySubType": "BUY",
                        "category": "trade"})
    filled = json.loads(subprocess.run([BIN, "bounds"],
                                       input=json.dumps({"db": os.path.join(work, "bagholder.db")}),
                                       capture_output=True, text=True, check=True).stdout)
    want_filled = bagholder.activity_sync_bounds()
    if filled["full"] is not want_filled["full_history"] or filled["start"] != want_filled["start_date"]:
        bad.append(f"bounds(filled): py={want_filled} rs={filled}")
    store.close_all()
    shutil.rmtree(work, ignore_errors=True)

    for line in bad[:20]:
        print("  " + line)
    n = len(ERRORS) + len(SESSIONS) + len(ACCOUNTS) * 2 + 2
    print(f"{n} sync cases, {len(bad)} differences")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
