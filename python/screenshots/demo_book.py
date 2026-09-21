"""A made-up book for the README screenshots: four accounts, shares on both
sides of the border, covered calls, long options, dividends, crypto with
staking, and two and a half years of daily equity. Nothing in it is anyone's.

    python3 python/screenshots/demo_book.py --home /tmp/bh-demo      # a desktop data directory
    python3 python/screenshots/demo_book.py --pull /tmp/bh-demo-phone  # last-pull.json + journal.json for the apps

It carries what the Portfolio tab needs too: each account's net liquidation value, a margin balance and buying power.
The desktop directory is served with `BAGHOLDER_HOME=/tmp/bh-demo BAGHOLDER_PORT=8799 python3 bagholder.py`;
market data (FX, indexes, quotes, declared distributions) is fetched by the app itself. The phone files
are seeded as MOBILE.md describes. The screenshots in this folder were taken that way.
"""
import argparse
import datetime as dt
import json
import os
import random
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))   # python/
sys.path.insert(0, ROOT)

TODAY = "2026-09-08"

ACCOUNTS = {
    "TFSA": ("acct-tfsa", "CAD", "SELF_DIRECTED_TFSA"),
    "RRSP": ("acct-rrsp", "CAD", "SELF_DIRECTED_RRSP"),
    "Trading": ("acct-trading", "USD", "SELF_DIRECTED_NON_REGISTERED_MARGIN"),
    "Crypto": ("acct-crypto", "CAD", "SELF_DIRECTED_CRYPTO"),
}

# symbol -> (name, exchange, mic, currency)
LISTINGS = {
    "XEQT": ("iShares Core Equity ETF Portfolio", "TSX", "XTSE", "CAD"),
    "VFV": ("Vanguard S&P 500 Index ETF", "TSX", "XTSE", "CAD"),
    "ENB": ("Enbridge Inc.", "TSX", "XTSE", "CAD"),
    "TD": ("Toronto-Dominion Bank", "TSX", "XTSE", "CAD"),
    "SHOP": ("Shopify Inc.", "TSX", "XTSE", "CAD"),
    "BCE": ("BCE Inc.", "TSX", "XTSE", "CAD"),
    "CNQ": ("Canadian Natural Resources", "TSX", "XTSE", "CAD"),
    "AAPL": ("Apple Inc.", "NASDAQ", "XNAS", "USD"),
    "MSFT": ("Microsoft Corp.", "NASDAQ", "XNAS", "USD"),
    "NVDA": ("NVIDIA Corp.", "NASDAQ", "XNAS", "USD"),
    "INTC": ("Intel Corp.", "NASDAQ", "XNAS", "USD"),
    "SOFI": ("SoFi Technologies", "NASDAQ", "XNAS", "USD"),
    "RIVN": ("Rivian Automotive", "NASDAQ", "XNAS", "USD"),
    "HOOD": ("Robinhood Markets", "NASDAQ", "XNAS", "USD"),
    "COIN": ("Coinbase Global", "NASDAQ", "XNAS", "USD"),
    "UBER": ("Uber Technologies", "NYSE", "XNYS", "USD"),
    "TSLA": ("Tesla Inc.", "NASDAQ", "XNAS", "USD"),
    "PLTR": ("Palantir Technologies", "NASDAQ", "XNAS", "USD"),
    "AMD": ("Advanced Micro Devices", "NASDAQ", "XNAS", "USD"),
    "BTC": ("Bitcoin", "", "", "CAD"),
    "ETH": ("Ether", "", "", "CAD"),
    "SOL": ("Solana", "", "", "CAD"),
}

_n = [0]
_acts = []
_option_ids = {}


def sec_id(symbol):
    return "sec-" + symbol.lower().replace(" ", "-").replace(".", "_")


def row(kind, account, day, symbol, qty, px, cash, **more):
    _n[0] += 1
    aid, _cur, _typ = ACCOUNTS[account]
    base = {
        "id": "demo-%04d" % _n[0],
        "canonicalId": "demo-%04d" % _n[0],
        "occurredAt": day + "T15:30:00.000000-04:00",
        "transactionDate": day,
        "accountId": aid,
        "fifoId": aid,
        "accountType": account,
        "description": "",
        "direction": "",
        "symbol": symbol,
        "name": symbol.split(" ")[0] if symbol else "",
        "currency": "CAD",
        "aftType": "",
        "counterSymbol": "",
        "securityId": sec_id(symbol) if symbol else "",
        "quantity": qty,
        "unitPrice": px,
        "commission": 0.0,
        "netCashAmount": round(cash, 2),
    }
    kinds = {
        "buy": dict(category="trade", activityType="Trade", activitySubType="BUY", rawType="DIY_BUY", direction="debit"),
        "sell": dict(category="trade", activityType="Trade", activitySubType="SELL", rawType="DIY_SELL", direction="credit"),
        "sto": dict(category="trade", activityType="OPTIONS_SELL", activitySubType="SELLTOOPEN", rawType="OPTIONS_SELL", direction="credit"),
        "stc": dict(category="trade", activityType="OPTIONS_SELL", activitySubType="SELLTOCLOSE", rawType="OPTIONS_SELL", direction="credit"),
        "bto": dict(category="trade", activityType="OPTIONS_BUY", activitySubType="BUYTOOPEN", rawType="OPTIONS_BUY", direction="debit"),
        "btc": dict(category="trade", activityType="OPTIONS_BUY", activitySubType="BUYTOCLOSE", rawType="OPTIONS_BUY", direction="debit"),
        "div": dict(category="dividend", activityType="Dividend", activitySubType="dividend", rawType="DIVIDEND", direction="credit"),
        "dep": dict(category="deposit", activityType="Deposit", activitySubType="deposit", rawType="DEPOSIT", direction="credit", aftType="misc_payments"),
        "cbuy": dict(category="other", activityType="CRYPTO_BUY", activitySubType="MARKET_ORDER", rawType="CRYPTO_BUY", direction="credit"),
        "csell": dict(category="other", activityType="CRYPTO_SELL", activitySubType="MARKET_ORDER", rawType="CRYPTO_SELL", direction="credit"),
        "reward": dict(category="other", activityType="CRYPTO_STAKING_REWARD", activitySubType="other", rawType="CRYPTO_STAKING_REWARD", direction="credit"),
        "charge": dict(category="other", activityType="INTEREST_CHARGE", activitySubType="MARGIN_INTEREST", rawType="INTEREST_CHARGE", direction="debit"),
    }
    base.update(kinds[kind])
    base["description"] = {"div": "Dividend: " + symbol, "dep": "Deposit", "cbuy": "CRYPTO_BUY: " + symbol, "csell": "CRYPTO_SELL: " + symbol,
                           "reward": "CRYPTO_STAKING_REWARD: " + symbol}.get(kind, base["activityType"] + ": " + symbol)
    base.update(more)
    _acts.append(base)
    return base["id"]


def buy(account, day, symbol, qty, px, currency="CAD"):
    return row("buy", account, day, symbol, qty, px, -qty * px, currency=currency)


def sell(account, day, symbol, qty, px, currency="CAD"):
    return row("sell", account, day, symbol, -qty, px, qty * px, currency=currency)


def option(kind, account, day, symbol, qty, px):
    under = symbol.split(" ")[0]
    _option_ids[symbol] = under
    sign = 1 if kind in ("sto", "stc") else -1
    q = -qty if kind == "sto" else qty if kind in ("bto", "btc") else -qty
    if kind == "btc":
        q = qty
    return row(kind, account, day, symbol, q, px, sign * qty * px * 100, currency="USD")


def dividend(account, day, symbol, qty, per):
    return row("div", account, day, symbol, qty, per, qty * per)


def deposit(account, day, amount):
    return row("dep", account, day, "", 0, 0, amount)


def interest_charge(account, day, amount, currency):
    return row("charge", account, day, "", 0, 0, -amount, currency=currency)


def crypto(kind, day, symbol, qty, px):
    return row(kind, "Crypto", day, symbol, qty, px, qty * px)


def build():
    j = {}
    # deposits
    deposit("TFSA", "2024-01-03", 30000); deposit("RRSP", "2024-01-03", 30000); deposit("Trading", "2024-02-01", 20000)
    deposit("Crypto", "2024-02-20", 15000); deposit("TFSA", "2024-07-02", 20000); deposit("RRSP", "2025-01-06", 20000); deposit("TFSA", "2025-09-02", 15000)

    # TFSA: Canadian core and a few swings
    buy("TFSA", "2024-01-08", "XEQT", 300, 28.10); buy("TFSA", "2024-07-08", "XEQT", 200, 31.40); buy("TFSA", "2025-09-03", "XEQT", 250, 36.20)
    enb = buy("TFSA", "2024-02-12", "ENB", 400, 46.30)
    buy("TFSA", "2024-03-11", "TD", 150, 80.20)
    shop = buy("TFSA", "2024-04-15", "SHOP", 60, 95.40); sell("TFSA", "2025-02-20", "SHOP", 60, 165.30)
    bce = buy("TFSA", "2024-05-06", "BCE", 200, 45.10); sell("TFSA", "2025-03-12", "BCE", 200, 32.40)
    cnq = buy("TFSA", "2025-04-08", "CNQ", 150, 40.20); sell("TFSA", "2025-08-19", "CNQ", 150, 45.10)
    j["rt:" + shop] = {"grade": "A", "tags": ["momentum", "earnings"], "thesis": "Merchant growth re-accelerating after the logistics sale; held through two earnings beats and sold into the run."}
    j["rt:" + bce] = {"grade": "D", "tags": ["yield-trap"], "thesis": "Bought for the dividend; the payout was cut and the thesis was gone. Should have sold on the cut, not four months later."}
    j["rt:" + cnq] = {"grade": "B", "tags": ["energy", "swing"], "thesis": "Oil oversold into tariff noise; took the bounce and left."}
    j["rt:" + enb] = {"grade": "", "tags": ["income"], "thesis": "Core income holding. Add on weakness below $45."}

    # RRSP: index core plus US names
    buy("RRSP", "2024-01-10", "VFV", 250, 118.50); buy("RRSP", "2025-01-08", "VFV", 120, 145.20)
    aapl = buy("RRSP", "2024-02-05", "AAPL", 40, 186.50, "USD"); sell("RRSP", "2024-12-10", "AAPL", 40, 246.80, "USD")
    buy("RRSP", "2024-06-03", "MSFT", 20, 410.20, "USD")
    nvda = buy("RRSP", "2024-08-07", "NVDA", 100, 98.90, "USD"); sell("RRSP", "2025-01-27", "NVDA", 60, 118.40, "USD")
    intc = buy("RRSP", "2024-04-29", "INTC", 200, 31.20, "USD"); sell("RRSP", "2024-11-05", "INTC", 200, 22.90, "USD")
    j["rt:" + aapl] = {"grade": "B", "tags": ["core", "trim"], "thesis": "Services margin story intact; trimmed the whole lot at a stretched multiple to fund the index core."}
    j["rt:" + intc] = {"grade": "C", "tags": ["turnaround"], "thesis": "Foundry turnaround was a story, not a number. Cut after the dividend suspension."}

    # Trading (USD): covered calls on AAPL, long options, US swings
    buy("Trading", "2024-03-04", "AAPL", 100, 172.30, "USD")
    cc1 = option("sto", "Trading", "2024-03-18", "AAPL 19APR24 190.00 CALL", 1, 2.10); option("btc", "Trading", "2024-04-12", "AAPL 19APR24 190.00 CALL", 1, 0.45)
    cc2 = option("sto", "Trading", "2024-05-20", "AAPL 21JUN24 200.00 CALL", 1, 2.85); option("btc", "Trading", "2024-06-14", "AAPL 21JUN24 200.00 CALL", 1, 9.20)
    cc3 = option("sto", "Trading", "2024-10-21", "AAPL 15NOV24 240.00 CALL", 1, 3.40); option("btc", "Trading", "2024-11-13", "AAPL 15NOV24 240.00 CALL", 1, 0.30)
    cc4 = option("sto", "Trading", "2025-01-27", "AAPL 21FEB25 260.00 CALL", 1, 2.60); option("btc", "Trading", "2025-02-19", "AAPL 21FEB25 260.00 CALL", 1, 0.20)
    option("sto", "Trading", "2026-08-24", "AAPL 16OCT26 260.00 CALL", 1, 4.10)
    tsla = option("bto", "Trading", "2024-07-15", "TSLA 20SEP24 200.00 PUT", 2, 6.30); option("stc", "Trading", "2024-08-21", "TSLA 20SEP24 200.00 PUT", 2, 2.10)
    nvc = option("bto", "Trading", "2024-09-16", "NVDA 17JAN25 120.00 CALL", 2, 4.80); option("stc", "Trading", "2024-12-02", "NVDA 17JAN25 120.00 CALL", 2, 16.40)
    pltr = option("bto", "Trading", "2024-10-14", "PLTR 21MAR25 40.00 CALL", 3, 2.15); option("stc", "Trading", "2025-02-10", "PLTR 21MAR25 40.00 CALL", 3, 34.50)
    amd = option("bto", "Trading", "2025-03-10", "AMD 20JUN25 130.00 CALL", 2, 5.40); option("stc", "Trading", "2025-06-13", "AMD 20JUN25 130.00 CALL", 2, 0.55)
    sofi = buy("Trading", "2024-06-24", "SOFI", 300, 7.15, "USD"); sell("Trading", "2025-01-21", "SOFI", 300, 15.80, "USD")
    rivn = buy("Trading", "2024-09-09", "RIVN", 200, 14.20, "USD"); sell("Trading", "2025-04-07", "RIVN", 200, 11.60, "USD")
    hood = buy("Trading", "2026-01-12", "HOOD", 100, 42.10, "USD"); sell("Trading", "2026-03-25", "HOOD", 100, 55.30, "USD")
    coin = buy("Trading", "2026-02-09", "COIN", 30, 250.40, "USD"); sell("Trading", "2026-05-14", "COIN", 30, 198.20, "USD")
    uber = buy("Trading", "2026-04-20", "UBER", 80, 78.30, "USD"); sell("Trading", "2026-07-28", "UBER", 80, 88.10, "USD")
    buy("Trading", "2026-06-15", "NVDA", 25, 165.40, "USD")
    j["rt:" + cc1] = {"grade": "A", "tags": ["covered-call"], "thesis": "Monthly call against the 100 shares; closed at 80% of max profit as planned."}
    j["rt:" + cc2] = {"grade": "C", "tags": ["covered-call", "capped"], "thesis": "Sold the 200 strike two weeks before WWDC. Bought back for a loss rather than lose the shares."}
    j["rt:" + cc3] = {"grade": "A", "tags": ["covered-call"], "thesis": "Post-earnings IV crush; closed early."}
    j["rt:" + cc4] = {"grade": "A", "tags": ["covered-call"], "thesis": "Same setup as November."}
    j["rt:" + tsla] = {"grade": "D", "tags": ["hedge", "theta"], "thesis": "Bought puts after the run-up expecting a fade. Fade came late; theta ate it."}
    j["rt:" + nvc] = {"grade": "B", "tags": ["earnings", "long-call"], "thesis": "Blackwell ramp priced too low into Q3; sold into the December high."}
    j["rt:" + pltr] = {"grade": "A", "tags": ["long-call", "momentum"], "thesis": "Commercial revenue inflection; sized small, let it run through two earnings."}
    j["rt:" + amd] = {"grade": "C", "tags": ["long-call"], "thesis": "Bet on an MI350 re-rate. Export controls in April killed the timing."}
    j["rt:" + sofi] = {"grade": "B", "tags": ["fintech", "swing"], "thesis": "Bank charter economics finally showing in NIM; rode it to the January high."}
    j["rt:" + rivn] = {"grade": "", "tags": [], "thesis": ""}
    j["rt:" + hood] = {"grade": "", "tags": [], "thesis": ""}
    j["rt:" + coin] = {"grade": "", "tags": [], "thesis": ""}
    j["rt:" + uber] = {"grade": "", "tags": [], "thesis": ""}

    # Crypto
    crypto("cbuy", "2024-02-26", "BTC", 0.25, 58200); crypto("cbuy", "2024-08-05", "BTC", 0.15, 82500); crypto("csell", "2024-12-16", "BTC", 0.20, 132400)
    eth = crypto("cbuy", "2024-03-11", "ETH", 3, 4150); crypto("csell", "2025-04-02", "ETH", 3, 3480)
    crypto("cbuy", "2024-11-11", "SOL", 40, 195)
    d = dt.date(2024, 12, 5)
    while d.isoformat() <= TODAY:
        crypto("reward", d.isoformat(), "SOL", 0.22, 200 + (d.month * 7) % 60)
        d = (d.replace(day=1) + dt.timedelta(days=32)).replace(day=5)
    j["rt:" + eth] = {"grade": "C", "tags": ["crypto"], "thesis": "Bought the ETF-approval news; sold a year later below cost."}

    # Dividends and distributions
    for y, per in ((2024, 0.915), (2025, 0.9425), (2026, 0.975)):
        for m in (3, 6, 9, 12):
            day = "%d-%02d-01" % (y, m)
            if "2024-02-12" < day <= TODAY:
                dividend("TFSA", day, "ENB", 400, per)
    for y, per in ((2024, 1.02), (2025, 1.05), (2026, 1.05)):
        for m in (1, 4, 7, 10):
            day = "%d-%02d-30" % (y, m) if m != 1 else "%d-01-31" % y
            if "2024-03-11" < day <= TODAY:
                dividend("TFSA", day, "TD", 150, per)
    for day in ("2024-07-15", "2024-10-15", "2025-01-15"):
        dividend("TFSA", day, "BCE", 200, 0.9975)
    for y in (2024, 2025, 2026):
        for m, per in ((3, 0.17), (6, 0.19), (9, 0.18), (12, 0.20)):
            day = "%d-%02d-28" % (y, m)
            if "2024-01-08" < day <= TODAY:
                qty = 300 + (200 if day >= "2024-07-08" else 0) + (250 if day >= "2025-09-03" else 0)
                dividend("TFSA", day, "XEQT", qty, per)
            if "2024-01-10" < day <= TODAY:
                dividend("RRSP", day, "VFV", 250 + (120 if day >= "2025-01-08" else 0), per * 2)
    # Margin interest on the Trading account, billed on the first of the month in USD
    for i, amount in enumerate((64.10, 71.85, 88.20, 93.40, 97.15, 102.60, 109.35, 118.90)):
        day = "2026-%02d-01" % (2 + i)
        if day <= TODAY:
            interest_charge("Trading", day, amount, "USD")
    _acts.sort(key=lambda a: (a["transactionDate"], a["id"]))
    return j


# What Wealthsimple states per account, as of the snapshot: net liquidation value, cash
# (negative on margin), and buying power for the margin account.
NAV_BY_ACCOUNT = {"TFSA": 90714.35, "RRSP": 97220.10, "Trading": 18157.40, "Crypto": 27787.60}   # positions at the snapshot plus cash, less margin
CASH_SECURITIES = [{"id": "sec-c-cad", "symbol": "CAD", "name": "Canadian dollar", "primaryExchange": "", "primaryMic": "", "currency": "CAD", "underlyingId": ""},
                   {"id": "sec-c-usd", "symbol": "USD", "name": "US dollar", "primaryExchange": "", "primaryMic": "", "currency": "USD", "underlyingId": ""}]
BALANCES = [{"accountId": "acct-tfsa", "securityId": "sec-c-cad", "quantity": 4210.35},
            {"accountId": "acct-rrsp", "securityId": "sec-c-cad", "quantity": 1875.00},
            {"accountId": "acct-trading", "securityId": "sec-c-usd", "quantity": -18240.60},
            {"accountId": "acct-crypto", "securityId": "sec-c-cad", "quantity": 312.40}]
MARGIN = [{"accountId": "acct-trading", "buyingPower": 12680.45, "currency": "CAD", "unavailable": ""}]


def listings():
    out = []
    for sym, (name, ex, mic, cur) in LISTINGS.items():
        out.append({"id": sec_id(sym), "symbol": sym, "name": name, "primaryExchange": ex, "primaryMic": mic, "currency": cur, "underlyingId": ""})
    for osym, under in _option_ids.items():
        out.append({"id": sec_id(osym), "symbol": osym, "name": osym, "primaryExchange": "OPRA", "primaryMic": "OPRA", "currency": "USD", "underlyingId": sec_id(under)})
    return out


def nav_series():
    """Business days from 2024-01-02: deposits as a step, equity as deposits times a drifting, noisy path."""
    rng = random.Random(9)
    steps = [("2024-01-03", 60000), ("2024-02-01", 80000), ("2024-02-20", 95000), ("2024-07-02", 115000), ("2025-01-06", 135000), ("2025-09-02", 150000)]
    out = []
    d = dt.date(2024, 1, 2)
    growth = 1.0
    while d.isoformat() <= TODAY:
        if d.weekday() < 5:
            deposits = 0
            for day, total in steps:
                if day <= d.isoformat():
                    deposits = total
            growth *= 1 + rng.gauss(0.00055, 0.009)
            if d.isoformat() in ("2024-08-05", "2025-04-03", "2025-04-04"):
                growth *= 0.955
            equity = round(deposits * growth, 2) if deposits else 0.0
            out.append({"date": d.isoformat(), "equity": equity, "netDeposits": float(deposits), "currency": "CAD"})
        d += dt.timedelta(days=1)
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--home", help="write a desktop data directory (bagholder.db) here")
    ap.add_argument("--pull", help="write last-pull.json and journal.json for the phone apps here")
    args = ap.parse_args()
    journal = build()
    lst = listings()
    nav = nav_series()
    if args.pull:
        os.makedirs(args.pull, exist_ok=True)
        with open(os.path.join(args.pull, "last-pull.json"), "w") as f:
            metrics = {"realizedPnlCad": 0.0, "tradeCount": 0, "winCount": 0, "lossCount": 0, "evenCount": 0, "grossProfit": 0.0, "grossLoss": 0.0, "winRate": 0.0,
                       "profitFactor": 0.0, "avgWin": 0.0, "avgLoss": 0.0, "expectancy": 0.0, "maxWinPnl": 0.0, "maxWinSymbol": "", "maxLossPnl": 0.0, "maxLossSymbol": "", "avgHoldDays": 0.0}
            # the iOS decoder wants every field of the pull, the derived ones included; the apps compute them from the rows
            accounts = [{"id": aid, "nickname": nick, "unifiedAccountType": typ, "currency": cur, "status": "open", "type": "self_directed", "netLiquidationValue": NAV_BY_ACCOUNT[nick]} for nick, (aid, cur, typ) in ACCOUNTS.items()]
            json.dump({"activities": _acts, "listings": lst + CASH_SECURITIES, "nav": nav, "navByAccount": {}, "syncedAt": TODAY + "T20:05:00Z",
                       "accounts": accounts, "balances": BALANCES, "margin": MARGIN,
                       "closed": [], "metrics": metrics, "monthly": [], "years": [], "avgAnnualized": "", "avgAnnualizedSubtitle": ""}, f)
        with open(os.path.join(args.pull, "journal.json"), "w") as f:
            json.dump({k: v for k, v in journal.items() if v["grade"] or v["tags"] or v["thesis"]}, f, indent=1, sort_keys=True)
        print("phone seed:", len(_acts), "activities,", len(lst), "listings,", len(nav), "nav days")
    if args.home:
        os.makedirs(args.home, exist_ok=True)
        os.environ["BAGHOLDER_HOME"] = args.home
        import store
        store.set_home(args.home)
        store.ensure()
        store.apply_wealthsimple_mapped(_acts)
        store.upsert_securities(lst)
        store.replace_accounts([{"id": aid, "nickname": nick, "unifiedAccountType": typ, "currency": cur, "status": "open", "type": "self_directed", "netLiquidationValue": NAV_BY_ACCOUNT[nick]} for nick, (aid, cur, typ) in ACCOUNTS.items()])
        store.upsert_securities(CASH_SECURITIES)
        store.replace_balances(BALANCES)
        store.replace_margin(MARGIN)
        store.replace_nav(nav)
        store.set_meta("synced_at", TODAY + "T20:05:00Z")
        for k, v in journal.items():
            if v["grade"] or v["tags"] or v["thesis"]:
                store.save_journal_entry(k, v)
        print("desktop:", store.activity_count(), "activities in", os.path.join(args.home, "bagholder.db"))


if __name__ == "__main__":
    main()
