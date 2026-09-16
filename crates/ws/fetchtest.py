"""The request shaping and response reading, both ways, without contacting
Wealthsimple: the activity window a pull asks for, the margin answer in each
of its shapes, which accounts count as margin accounts, and the Money reader.

    cargo build -p bagholder-ws && python3 crates/ws/fetchtest.py
"""
import json
import os
import subprocess
import sys
from datetime import datetime, timezone

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, ROOT)
import bagholder  # noqa: E402

BIN = os.path.join(ROOT, "target", "debug", "fetchtool")
NOW_UNIX = 1789000000
NOW = datetime.fromtimestamp(NOW_UNIX, timezone.utc)

CONDITIONS = [
    {"accountId": "acct-1"},
    {"accountId": "acct-1", "startDate": "2026-01-05"},
    {"accountId": "acct-1", "startDate": "2026-01-05T09:30:00.000Z"},
    {"accountId": "acct-1", "startDate": "   "},
    {"accountId": ""},
]

MARGINS = [
    {"account": {"financials": {"current": {"marginV3": {"trading": {"buyingPower": {
        "__typename": "BuyingPowerMetricAvailable", "total": {"amount": "5000.25", "currency": "CAD"}}}}}}}},
    {"account": {"financials": {"current": {"marginV3": {"trading": {"buyingPower": {
        "__typename": "BuyingPowerMetricAvailable", "total": {"amount": None}}}}}}}},
    {"account": {"financials": {"current": {"marginV3": {"trading": {"buyingPower": {
        "__typename": "BuyingPowerMetricUnavailable",
        "reason": {"__typename": "UnsupportedSecurities", "securities": [1, 2, 3]}}}}}}}},
    {"account": {"financials": {"current": {"marginV3": {"trading": {"buyingPower": {
        "__typename": "BuyingPowerMetricUnavailable", "reason": {}}}}}}}},
    {"account": {"financials": {"current": {"marginV3": {"trading": {}}}}}},
    {"account": {"financials": {}}},
    {},
]

ACCOUNTS = [
    {"id": "m1", "unifiedAccountType": "SELF_DIRECTED_MARGIN", "status": "open"},
    {"id": "m2", "unifiedAccountType": "SELF_DIRECTED_MARGIN", "status": "closed"},
    {"id": "c1", "unifiedAccountType": "SELF_DIRECTED_CASH", "status": "open"},
    {"id": "", "unifiedAccountType": "SELF_DIRECTED_MARGIN", "status": "open"},
    {"id": "m3", "unified_account_type": "margin", "status": ""},
]

MONEYS = [
    {"netLiquidationValue": {"amount": "1234.56", "currency": "USD"}},
    {"netLiquidationValue": {"amount": None}, "netLiquidationValueV2": {"amount": 99}},
    {"netDeposits": {"amount": 5, "currency": ""}},
    {"netLiquidationValue": "not a dict"},
    {},
]


def norm(v):
    if isinstance(v, bool):
        return v
    if isinstance(v, (int, float)):
        return round(float(v), 9) + 0.0
    if isinstance(v, dict):
        return {k: norm(x) for k, x in v.items()}
    if isinstance(v, (list, tuple)):
        return [norm(x) for x in v]
    return v


def main():
    payload = {"now": NOW_UNIX, "conditions": CONDITIONS, "margins": MARGINS,
               "accounts": ACCOUNTS, "moneys": MONEYS}
    got = json.loads(subprocess.run([BIN], input=json.dumps(payload), capture_output=True,
                                    text=True, check=True).stdout)
    bad = []
    for i, c in enumerate(CONDITIONS):
        want = bagholder.activity_fetch_condition(c["accountId"], c.get("startDate"), NOW)
        if norm(want) != norm(got["conditions"][i]):
            bad.append(f"condition[{i}]: py={want} rs={got['conditions'][i]}")
    for i, m in enumerate(MARGINS):
        want = bagholder.parse_margin(m)
        if norm(want) != norm(got["margins"][i]):
            bad.append(f"margin[{i}]: py={want} rs={got['margins'][i]}")
    want_ids = bagholder.margin_account_ids(ACCOUNTS)
    if norm(want_ids) != norm(got["marginIds"]):
        bad.append(f"marginIds: py={want_ids} rs={got['marginIds']}")
    for i, n in enumerate(MONEYS):
        want = list(bagholder._money_amount(n, "netLiquidationValue", "netLiquidationValueV2", "netDeposits"))
        if norm(want) != norm(got["moneys"][i]):
            bad.append(f"money[{i}]: py={want} rs={got['moneys'][i]}")
    for line in bad[:20]:
        print("  " + line)
    total = len(CONDITIONS) + len(MARGINS) + len(MONEYS) + 1
    print(f"{total} fetch cases, {len(bad)} differences")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
