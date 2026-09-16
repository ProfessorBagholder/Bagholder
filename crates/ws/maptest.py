"""The activity mapper, both ways.

Feed items in every shape Wealthsimple sends one: share and option fills of
each side, a multileg credit and debit, expiries and assignments, a name
change that renames a ticker, an in-kind distribution, deposits, withdrawals,
transfers, dividends, interest, stock-lending rows that must be dropped, rows
with no status, rows with a status that is not a fill, and rows whose amount
sign is the only thing saying which way they go.

    cargo build -p bagholder-ws && python3 crates/ws/maptest.py
"""
import itertools
import json
import os
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, ROOT)
import bagholder  # noqa: E402

BIN = os.path.join(ROOT, "target", "debug", "maptool")

ACCOUNTS = [
    {"id": "acct-cad", "nickname": "Trading", "unifiedAccountType": "CASH", "currency": "CAD",
     "linkedAccount": {"id": "acct-usd"}},
    {"id": "acct-usd", "nickname": "Trading", "unifiedAccountType": "CASH", "currency": "USD"},
    {"id": "acct-tfsa", "nickname": "TFSA", "unifiedAccountType": "TFSA", "currency": "CAD"},
    {"id": "acct-bare", "unifiedAccountType": "RRSP", "currency": "CAD"},
    {"id": "", "nickname": "no id"},
]


def item(**kw):
    base = {
        "canonicalId": "ws-1", "occurredAt": "2026-05-04T15:30:00Z", "accountId": "acct-cad",
        "status": "POSTED", "type": "DIY_BUY", "subType": "", "assetSymbol": "AAA",
        "assetQuantity": 100, "amount": 1000, "currency": "CAD", "fees": 0,
    }
    base.update(kw)
    return base


ITEMS = [
    item(),
    item(type="DIY_SELL", amount=1200),
    item(type="DIY_BUY", assetSymbol="EXCHANGE:BBB"),
    item(type="OPTIONS_SELL", subType="SELLTOOPEN", contractType="CALL", strikePrice=10,
         expiryDate="2026-08-21", assetQuantity=2, amount=600, currency="USD"),
    item(type="OPTIONS_BUY", subType="BUYTOCLOSE", contractType="P", strikePrice=3.5,
         expiryDate="2026-11-20T00:00:00Z", assetQuantity=1, amount=125, currency="USD"),
    item(type="OPTIONS_MULTILEG", subType="FILLED", assetQuantity=0, amount=225,
         amountSign="positive", contractType="CALL", strikePrice=12, expiryDate="2027-01-15"),
    item(type="OPTIONS_MULTILEG", subType="FILLED", assetQuantity=0, amount=225,
         amountSign="negative", contractType="CALL", strikePrice=12, expiryDate="2027-01-15"),
    item(type="OPTIONS_EXPIRY", subType="EXPIRED", assetQuantity=2, amount=0,
         contractType="CALL", strikePrice=10, expiryDate="2026-08-21"),
    item(type="OPTIONS_SHORT_EXPIRY", subType="", assetQuantity=2, amount=0,
         contractType="PUT", strikePrice=5, expiryDate="2026-08-21"),
    item(type="OPTIONS_ASSIGNMENT", subType="ASSIGNED", assetQuantity=3, amount=3000,
         contractType="CALL", strikePrice=10, expiryDate="2026-08-21"),
    item(type="OPTIONS_EXERCISE", subType="", assetQuantity=1, amount=0,
         contractType="PUT", strikePrice=5, expiryDate="2026-08-21"),
    item(type="DEPOSIT", assetSymbol="", assetQuantity=0, amount=5000),
    item(type="CONTRIBUTION", assetSymbol="", assetQuantity=0, amount=5000),
    item(type="WITHDRAWAL", assetSymbol="", assetQuantity=0, amount=500),
    item(type="INTERNAL_TRANSFER", subType="SOURCE", assetSymbol="", assetQuantity=0, amount=100),
    item(type="INTERNAL_TRANSFER", subType="DESTINATION", assetSymbol="", assetQuantity=0, amount=100),
    item(type="DIVIDEND", assetQuantity=0, amount=42.5, subType=""),
    item(type="DIVIDEND", assetQuantity=10, amount=0),                       # in kind: delivers shares
    item(type="INTEREST", subType="FPL_INTEREST", assetSymbol="", assetQuantity=0, amount=1.25),
    item(type="INTEREST_CHARGE", status="", assetSymbol="", assetQuantity=0, amount=9.99),
    item(type="FUNDS_CONVERSION", assetSymbol="", assetQuantity=0, amount=1000),
    item(type="FEE", assetSymbol="", assetQuantity=0, amount=10),
    item(type="REFUND", assetSymbol="", assetQuantity=0, amount=10),
    item(type="STOCK_DISTRIBUTION", assetQuantity=25, amount=0),
    item(type="STKDIS", assetQuantity=-25, amount=0, amountSign="negative"),
    item(type="CORPORATE_ACTION", subType="CODE_CHANGE", assetSymbol="OLD",
         counterAssetSymbol="NEW", assetQuantity=40, amount=0, status="PROCESSED"),
    item(type="CORPORATE_ACTION", subType="CODE_CHANGE", assetSymbol="OLD",
         assetQuantity=40, amount=0, status="PROCESSED"),                    # lone code change
    item(type="CORPORATE_ACTION", subType="CODE_CHANGE", assetSymbol="OLD",
         counterAssetSymbol="NEW", assetQuantity=40, amount=0, status="REJECTED"),
    item(type="SHARE_LENDING", assetQuantity=10, amount=0),
    item(type="STOCK_LOAN", assetQuantity=10, amount=0),
    item(type="DIY_BUY", status="CANCELLED"),
    item(type="DIY_BUY", status=""),
    item(type="DIY_BUY", occurredAt=""),
    item(type="SOMETHING_ELSE", subType="ODD", assetQuantity=0, amount=7, amountSign="negative"),
    item(type="SOMETHING_ELSE", subType="", assetSymbol="", assetQuantity=0, amount=7,
         amountSign="positive"),
    item(canonicalId="manual-3"),                                            # a homemade id is not the broker's
    item(canonicalId="a|b|c"),
    item(accountId="acct-usd", currency="USD"),
    item(accountId="acct-unknown"),
    item(type="DIY_BUY", assetQuantity=3, amount=0),                         # no cash: price falls out at 0
    item(type="DIY_BUY", assetQuantity=0, amount=100),
    item(type="OPTIONS_SELL", subType="SELLTOOPEN", contractType="CALL", strikePrice=10,
         expiryDate="bad-date", assetQuantity=2, amount=600),
    item(type="DIY_BUY", aftOriginatorName="Some Bank"),
    item(type="DIY_BUY", institutionName="Some Broker", aftOriginatorName=""),
    item(type="DIY_BUY", securityId="sec-1"),
    item(type="DIY_BUY", securityId="   "),
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
    payload = {"items": ITEMS, "accounts": ACCOUNTS}
    got = json.loads(subprocess.run([BIN], input=json.dumps(payload), capture_output=True,
                                    text=True, check=True).stdout)
    bad = []

    if norm(bagholder.nav_account_groups(ACCOUNTS)) != norm(got["navGroups"]):
        bad.append(f"navGroups py={bagholder.nav_account_groups(ACCOUNTS)} rs={got['navGroups']}")
    if norm(bagholder.fifo_pool_ids(ACCOUNTS)) != norm(got["fifoPools"]):
        bad.append(f"fifoPools py={bagholder.fifo_pool_ids(ACCOUNTS)} rs={got['fifoPools']}")

    rows_out = 0
    for i, (it, g) in enumerate(zip(ITEMS, got["rows"])):
        want = {
            "skip": bagholder.skip_activity(it),
            "corp": bagholder._is_corp_share_move(it),
            "codeChange": bagholder._is_code_change(it),
            "assetSymbol": bagholder._asset_symbol(it),
            "counterSymbol": bagholder._counter_symbol(it),
            "optionSymbol": bagholder.option_symbol(it),
            "signedCash": bagholder.signed_cash(it),
            "rows": bagholder.map_activity_rows(it, ACCOUNTS),
        }
        rows_out += len(want["rows"])
        for k in want:
            if norm(want[k]) != norm(g.get(k)):
                bad.append(f"item[{i}] ({it.get('type')}/{it.get('subType')}).{k}")
                bad.append(f"    py={json.dumps(norm(want[k]))[:220]}")
                bad.append(f"    rs={json.dumps(norm(g.get(k)))[:220]}")

    for line in bad[:30]:
        print("  " + line)
    print(f"{len(ITEMS)} feed items -> {rows_out} ledger rows, {len(bad)} differences")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
