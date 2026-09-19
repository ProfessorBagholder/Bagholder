"""Write the shared model cases: activity rows in, the figures the spec says
they produce out, as JSON every implementation (Python, Swift, Kotlin) runs
through its own model. The Python model is the reference: run this after an
intended model change, review the diff of tests/cases, commit both.

    python3 python/tests/make_cases.py
"""
import json
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
# the shared cases live at the repository root, read by every implementation
CASES_DIR = os.path.join(os.path.dirname(ROOT), "tests", "cases")
sys.path.insert(0, ROOT)
import model  # noqa: E402
from test_model import act, buy, sell  # noqa: E402

TRADE_KEYS = ("id", "symbol", "kind", "currency", "side", "qty", "mult", "entry", "exit", "entryDate", "exitDate", "holdDays", "pnl", "pnlCad", "pnlPct", "status", "fees", "account", "exchange", "grade", "tags")
KPI_KEYS = ("count", "wins", "losses", "breakeven", "winRate", "realized", "expectancy", "profitFactor", "avgHold", "avgWin", "avgLoss", "grossWin", "grossLoss")
POSITION_KEYS = ("id", "symbol", "kind", "currency", "account", "exchange", "qty", "avg", "cost", "held", "alloc", "short", "dayChange", "grade")
PORTFOLIO_KEYS = ("marketValue", "costBasis", "unrealized", "unrealizedPct", "positionCount", "accountCount", "nav", "navAccounts", "marginUsed", "marginUsedBy", "marginUsedPct", "availableMargin", "availableMarginUnavailable", "hasMargin", "cash", "cashPct", "dayChange", "dayChangePct")
ALLOCATION_KEYS = ("id", "symbol", "account", "value", "share")
YEAR_KEYS = ("year", "r", "days", "from", "to", "flow", "endV", "spR")
MONTH_KEYS = ("key", "label", "value", "count")
SYMBOL_KEYS = ("symbol", "pnl", "n", "legs", "winRate", "avgHold")
QUEUE_KEYS = ("id", "symbol", "date", "pnl", "missing")
HOLDING_KEYS = ("symbol", "qty", "per", "freq", "freqVerified", "annual", "yoc", "ytd", "ttm", "all", "nextExDate", "nextPayDate", "exPast", "payPast")
TILE_KEYS = ("label", "total", "perMonth", "count", "yield", "projected", "earned", "book", "marginUsed", "interestPerMonth", "interestMonths")


def snapshot(acts, securities=None, nav=None, nav_by_account=None, accounts=None, balances=None, margin=None):
    return {"activities": acts, "accounts": accounts or [], "balances": balances or [], "margin": margin or [], "navHistory": nav or [], "navByAccount": nav_by_account or {}, "syncedAt": "", "tradeGroups": [], "notes": {}, "securities": securities or []}


def dividend(id, symbol, qty, per, day, account="Cashflow"):
    return act(id=id, category="dividend", activityType="Dividend", rawType="DIVIDEND", quantity=qty, unitPrice=per, netCashAmount=round(qty * per, 2), transactionDate=day, symbol=symbol, currency="CAD", accountType=account)


def sto(id, symbol, qty, px, day, **extra):
    o = dict(id=id, category="trade", activityType="OPTIONS_SELL", activitySubType="SELLTOOPEN", rawType="OPTIONS_SELL", quantity=-qty, unitPrice=px, netCashAmount=qty * px * 100, transactionDate=day, symbol=symbol)
    o.update(extra)
    return act(**o)


def btc(id, symbol, qty, px, day, sub="BUYTOCLOSE", **extra):
    o = dict(id=id, category="trade", activityType="OPTIONS_BUY", activitySubType=sub, rawType="OPTIONS_BUY", quantity=qty, unitPrice=px, netCashAmount=-qty * px * 100, transactionDate=day, symbol=symbol)
    o.update(extra)
    return act(**o)


def multileg(id, symbol, cash, day):
    """A Wealthsimple multileg fill as posted: quantity 0, only the cash."""
    return act(id=id, activityType="OPTIONS_MULTILEG", activitySubType="FILLED", rawType="OPTIONS_MULTILEG", quantity=0, netCashAmount=cash, transactionDate=day, symbol=symbol)


def crypto(id, kind, symbol, qty, px, day, account="Crypto"):
    raw = {"buy": "CRYPTO_BUY", "sell": "CRYPTO_SELL", "reward": "CRYPTO_STAKING_REWARD"}[kind]
    return act(id=id, activityType=raw, activitySubType="MARKET_ORDER" if kind != "reward" else "other", rawType=raw, quantity=qty, unitPrice=px, netCashAmount=qty * px, transactionDate=day, symbol=symbol, currency="CAD", accountType=account)


def crypto_transfer(id, symbol, qty, value, day, out=False, account="Crypto"):
    return act(id=id, activityType="CRYPTO_TRANSFER", activitySubType="TRANSFER_OUT" if out else "TRANSFER_IN", rawType="CRYPTO_TRANSFER",
               direction="DEBIT" if out else "CREDIT", quantity=qty, unitPrice=value / qty, netCashAmount=-value if out else value,
               transactionDate=day, symbol=symbol, currency="CAD", accountType=account)


def nav(day, equity, deposits=None):
    return {"date": day, "equity": equity, "netDeposits": deposits}


NAV = [
    nav("2023-12-15", 50.0, 50.0),          # pre-history: a few dollars parked before the real start
    nav("2024-01-15", 100000.0, 100000.0),
    nav("2024-03-01", 110000.0, 100000.0),
    nav("2024-06-03", 95000.0, 100000.0),
    nav("2024-09-03", 140000.0, 130000.0),  # a 30,000 deposit on the day
    nav("2024-12-31", 150000.0, 130000.0),
    nav("2025-03-03", 170000.0, 130000.0),
    nav("2025-06-02", 150000.0, 130000.0),
    nav("2025-12-31", 200000.0, 130000.0),
    nav("2026-03-02", 230000.0, 130000.0),
    nav("2026-06-01", 190000.0, 120000.0),  # a 10,000 withdrawal
    nav("2026-09-04", 215000.0, 120000.0),
]
SP500 = {"2023-12-29": 4700.0, "2024-06-28": 5400.0, "2024-12-31": 5900.0, "2025-06-30": 6200.0, "2025-12-31": 6800.0, "2026-09-04": 7200.0}
TSX = {"2023-12-29": 20900.0, "2024-06-28": 21800.0, "2024-12-31": 24700.0, "2025-06-30": 26800.0, "2025-12-31": 28100.0, "2026-09-04": 29500.0}

# a small book across two accounts for the filter cases: shares, an option chain, a loser, a
# position in each account, one exchange record per share, and a journal on three trades
FILTER_ACTS = [
    buy("b1", "AAA", 100, 10.0, "2026-01-05", securityId="sec-aaa"), sell("s1", "AAA", 100, 12.0, "2026-01-20", securityId="sec-aaa"),
    buy("b2", "AAA", 50, 12.0, "2026-02-02", securityId="sec-aaa"), sell("s2", "AAA", 50, 11.0, "2026-02-10", securityId="sec-aaa"),
    buy("b3", "BBB", 200, 5.0, "2025-11-03", accountType="TFSA", securityId="sec-bbb"), sell("s3", "BBB", 200, 6.0, "2026-03-16", accountType="TFSA", securityId="sec-bbb"),
    sto("sto", "ZZZ 21AUG26 10.00 CALL", 2, 3, "2026-04-01"), btc("cover", "ZZZ 21AUG26 10.00 CALL", 2, 1, "2026-05-15"),
    buy("b4", "CCC", 10, 100.0, "2026-06-01", currency="USD"), sell("s4", "CCC", 10, 90.0, "2026-06-12", currency="USD"),
    buy("b5", "DDD", 300, 2.0, "2026-07-01", securityId="sec-ddd"),
    buy("b6", "EEE", 40, 25.0, "2026-08-01", accountType="TFSA"),
    dividend("d1", "DDD", 300, 0.05, "2026-08-15", account="Trading"),
]
FILTER_SECS = [{"id": "sec-aaa", "symbol": "AAA", "name": "Triple A Corp", "primaryExchange": "TSX"},
               {"id": "sec-bbb", "symbol": "BBB", "name": "Bee Inc", "primaryExchange": "NASDAQ"},
               {"id": "sec-ddd", "symbol": "DDD", "name": "Dee Fund", "primaryExchange": "TSX"}]
FILTER_JOURNAL = {"rt:b1": {"grade": "A", "thesis": "Breakout after earnings.", "tags": ["earnings", "breakout"]},
                  "rt:b2": {"grade": "F", "thesis": "", "tags": []},
                  "rt:sto": {"grade": "B", "thesis": "Covered call on a flat name.", "tags": ["income"]},
                  "rt:b5": {"grade": "", "thesis": "Holding for the distribution.", "tags": ["income"]}}
FILTER_MARKET = {"fx": {"2026-06-01": 1.37, "2026-06-12": 1.36}, "benchmark": SP500, "benchmarks": {"TSX": TSX}}


CASES = {
    # two share round trips in CAD: a win, then a loss, one account
    "shares_two_round_trips": {
        "today": "2026-03-01",
        "activities": [
            buy("b1", "AAA", 100, 10.0, "2026-01-05"), sell("s1", "AAA", 100, 12.0, "2026-01-20"),
            buy("b2", "AAA", 50, 12.0, "2026-02-02"), sell("s2", "AAA", 50, 11.0, "2026-02-10"),
        ],
        "market": {"fx": {}, "benchmark": {}},
    },
    # a short option sold to open and bought to close, USD, contract multiplier 100
    "option_short_then_cover": {
        "today": "2026-03-01",
        "activities": [
            act(id="sto", category="trade", activityType="OPTIONS_SELL", activitySubType="SELLTOOPEN", rawType="OPTIONS_SELL", quantity=-2, unitPrice=3, netCashAmount=600, transactionDate="2026-01-01", symbol="ZZZ 21AUG26 10.00 CALL"),
            act(id="btc", category="trade", activityType="OPTIONS_BUY", activitySubType="BUYTOCLOSE", rawType="OPTIONS_BUY", quantity=2, unitPrice=1, netCashAmount=-200, transactionDate="2026-02-01", symbol="ZZZ 21AUG26 10.00 CALL"),
        ],
        "market": {"fx": {"2026-01-01": 1.40, "2026-02-01": 1.35}, "benchmark": {}},
    },
    # an open position with two lots, no sale yet
    "shares_open_position_two_lots": {
        "today": "2026-03-01",
        "activities": [buy("b1", "BBB", 100, 5.0, "2026-01-05"), buy("b2", "BBB", 100, 7.0, "2026-02-05")],
        "market": {"fx": {}, "benchmark": {}},
    },
    # the Portfolio tiles: CAD aggregates over every account, cash accounts included, margin used
    # from the negative cash per currency, available margin from Wealthsimple's buying power,
    # and the day's change on a position from its quote
    "portfolio_tiles_over_all_accounts": {
        "today": "2026-02-01",
        "activities": [
            buy("b1", "AAA", 10, 10, "2026-01-05"),
            buy("b2", "BBB", 5, 20, "2026-01-06", accountType="Kids", accountId="acct-2", currency="USD"),
        ],
        "securities": [
            {"id": "sec-c-cad", "symbol": "CAD", "currency": "CAD"},
            {"id": "sec-c-usd", "symbol": "USD", "currency": "USD"},
        ],
        "accounts": [
            {"id": "acct-1", "nickname": "Trading", "currency": "CAD", "netLiquidationValue": 1500.0, "unifiedAccountType": "SELF_DIRECTED_NON_REGISTERED_MARGIN"},
            {"id": "acct-2", "nickname": "Kids", "currency": "CAD", "netLiquidationValue": 400.0, "unifiedAccountType": "SELF_DIRECTED_JOINT_NON_REGISTERED_MARGIN"},
            {"id": "acct-3", "nickname": "Cash", "currency": "CAD", "netLiquidationValue": 25.0, "unifiedAccountType": "CASH"},
            {"id": "acct-4", "nickname": "Old", "currency": "CAD", "netLiquidationValue": 999.0, "status": "closed", "unifiedAccountType": "SELF_DIRECTED_NON_REGISTERED_MARGIN"},
            {"id": "acct-5", "nickname": "TFSA", "currency": "CAD", "netLiquidationValue": 0.0, "unifiedAccountType": "SELF_DIRECTED_TFSA"},
        ],
        "balances": [
            {"accountId": "acct-1", "securityId": "sec-c-cad", "quantity": -300.0},
            {"accountId": "acct-1", "securityId": "sec-c-usd", "quantity": -10.0},
            {"accountId": "acct-2", "securityId": "sec-c-cad", "quantity": 50.0},
        ],
        "margin": [
            {"accountId": "acct-1", "buyingPower": 700.0, "currency": "CAD", "unavailable": ""},
            {"accountId": "acct-2", "buyingPower": None, "currency": "CAD", "unavailable": "UnavailableSecurities (1 securities)"},
            {"accountId": "acct-5", "buyingPower": 5638.24, "currency": "CAD", "unavailable": ""},
        ],
        "journal": {"rt:b1": {"grade": "B", "thesis": "hold", "tags": ["core"]}},
        "market": {"fx": {"2026-02-01": 1.5}, "benchmark": {}, "quotes": {"AAA": {"price": 12.0, "priceChange": 0.5, "percentChange": 4.35}, "BBB": {"price": 30.0}}},
    },
    # the same book with one account on: the tiles narrow to it, and the account whose margin is unavailable says so
    "portfolio_tiles_one_account": {
        "today": "2026-02-01",
        "activities": [
            buy("b1", "AAA", 10, 10, "2026-01-05"),
            buy("b2", "BBB", 5, 20, "2026-01-06", accountType="Kids", accountId="acct-2", currency="USD"),
        ],
        "securities": [{"id": "sec-c-cad", "symbol": "CAD", "currency": "CAD"}, {"id": "sec-c-usd", "symbol": "USD", "currency": "USD"}],
        "accounts": [
            {"id": "acct-1", "nickname": "Trading", "currency": "CAD", "netLiquidationValue": 1500.0, "unifiedAccountType": "SELF_DIRECTED_NON_REGISTERED_MARGIN"},
            {"id": "acct-2", "nickname": "Kids", "currency": "CAD", "netLiquidationValue": 400.0, "unifiedAccountType": "SELF_DIRECTED_JOINT_NON_REGISTERED_MARGIN"},
        ],
        "balances": [{"accountId": "acct-1", "securityId": "sec-c-cad", "quantity": -300.0}, {"accountId": "acct-2", "securityId": "sec-c-cad", "quantity": 50.0}],
        "margin": [{"accountId": "acct-2", "buyingPower": None, "currency": "CAD", "unavailable": "UnavailableSecurities (1 securities)"}],
        "filters": {"lists": {"account": ["Kids"]}},
        "market": {"fx": {"2026-02-01": 1.5}, "benchmark": {}, "quotes": {"BBB": {"price": 30.0, "priceChange": -1.0, "percentChange": -3.2}}},
    },
    # a book without a margin account: Cash and Day change stand in for the margin tiles, Last 12 months for the Margin used tile
    "portfolio_tiles_without_margin": {
        "today": "2026-02-01",
        "activities": [
            buy("b1", "AAA", 10, 10, "2025-01-05"),
            buy("b2", "BBB", 5, 20, "2025-01-06", accountType="Kids", accountId="acct-2", currency="USD"),
            dividend("d0", "AAA", 10, 1.0, "2024-12-01"),
            dividend("d1", "AAA", 10, 1.0, "2025-06-01"),
            dividend("d2", "AAA", 10, 1.5, "2025-12-01"),
        ],
        "securities": [{"id": "sec-c-cad", "symbol": "CAD", "currency": "CAD"}, {"id": "sec-c-usd", "symbol": "USD", "currency": "USD"}],
        "accounts": [
            {"id": "acct-1", "nickname": "Trading", "currency": "CAD", "netLiquidationValue": 1500.0, "unifiedAccountType": "SELF_DIRECTED_TFSA"},
            {"id": "acct-2", "nickname": "Kids", "currency": "CAD", "netLiquidationValue": 500.0, "unifiedAccountType": "SELF_DIRECTED_RESP"},
        ],
        "balances": [
            {"accountId": "acct-1", "securityId": "sec-c-cad", "quantity": 300.0},
            {"accountId": "acct-2", "securityId": "sec-c-usd", "quantity": 10.0},
        ],
        "margin": [{"accountId": "acct-1", "buyingPower": 5638.24, "currency": "CAD", "unavailable": ""}],
        "market": {"fx": {"2026-02-01": 1.5}, "benchmark": {}, "quotes": {"AAA": {"price": 12.0, "priceChange": 0.5, "percentChange": 4.35}, "BBB": {"price": 30.0, "priceChange": -1.0, "percentChange": -3.2}}},
    },
    # the Margin used tile: the Portfolio figure, with margin interest averaged over the months that carried a charge
    "cashflow_margin_used_tile": {
        "today": "2026-09-07",
        "activities": [
            buy("b1", "AAA", 10, 10, "2026-01-05"),
            dividend("d1", "AAA", 10, 0.5, "2026-08-06"),
            act(id="i1", activityType="INTEREST_CHARGE", activitySubType="MARGIN_INTEREST", rawType="INTEREST_CHARGE", category="other",
                netCashAmount=-100, transactionDate="2026-07-01", symbol="", currency="CAD"),
            act(id="i2", activityType="INTEREST_CHARGE", activitySubType="MARGIN_INTEREST", rawType="INTEREST_CHARGE", category="other",
                netCashAmount=-20, transactionDate="2026-08-01", symbol="", currency="USD"),
            act(id="i3", activityType="INTEREST_CHARGE", activitySubType="MARGIN_INTEREST", rawType="INTEREST_CHARGE", category="other",
                netCashAmount=-10, transactionDate="2026-08-04", symbol="", currency="CAD"),
        ],
        "securities": [{"id": "sec-c-cad", "symbol": "CAD", "currency": "CAD"}, {"id": "sec-c-usd", "symbol": "USD", "currency": "USD"}],
        "accounts": [{"id": "acct-1", "nickname": "Trading", "currency": "CAD", "netLiquidationValue": 1500.0, "unifiedAccountType": "SELF_DIRECTED_NON_REGISTERED_MARGIN"}],
        "balances": [{"accountId": "acct-1", "securityId": "sec-c-cad", "quantity": -300.0}],
        "market": {"fx": {"2026-08-01": 1.5, "2026-09-07": 1.5}, "benchmark": {}},
    },
    # an income holding with a declared distribution record: rate, projection, ex-div and pay day
    "cashflow_holding_with_declared_record": {
        "today": "2026-09-07",
        "activities": [
            buy("b1", "RDDY", 4000, 11.64, "2026-05-01", accountType="Cashflow"),
            dividend("d1", "RDDY", 4000, 0.15, "2026-08-06"),
            dividend("d2", "RDDY", 4000, 0.15, "2026-09-04"),
        ],
        "market": {"fx": {}, "benchmark": {},
                   "distributions": {"RDDY": [{"exDate": "2026-07-31", "payDate": "2026-08-06", "amount": 0.15, "currency": "CAD"},
                                              {"exDate": "2026-08-31", "payDate": "2026-09-04", "amount": 0.15, "currency": "CAD"},
                                              {"exDate": "2026-09-30", "payDate": "2026-10-06", "amount": 0.15, "currency": "CAD"}]}},
    },
    # a short call covered and re-sold on the same day is a roll: the cover's P&L
    # folds into the far contract's basis and only the far contract is a trade
    "option_roll_same_day_folds": {
        "today": "2027-01-01",
        "activities": [
            sto("aug-sto", "ZZZ 21AUG26 10.00 CALL", 1, 3, "2026-01-01"),
            btc("aug-cover", "ZZZ 21AUG26 10.00 CALL", 1, 1, "2026-08-15"),
            sto("jan-sto", "ZZZ 15JAN27 12.00 CALL", 1, 2, "2026-08-15"),
            btc("jan-cover", "ZZZ 15JAN27 12.00 CALL", 1, 0.5, "2026-12-01"),
        ],
        "market": {"fx": {"2026-01-01": 1.40, "2026-08-15": 1.38, "2026-12-01": 1.36}, "benchmark": {}},
    },
    # a multileg roll posts only the closing leg, with quantity 0: 16 short Jan27
    # calls rolled to Jan28, 6 more sold, all 22 bought back; nothing stays open
    "option_multileg_roll_carries_the_leg": {
        "today": "2026-09-01",
        "activities": [
            sto("sto", "LUNR 15JAN27 12.00 CALL", 16, 6.2225, "2025-10-01"),
            multileg("ml", "LUNR 15JAN27 12.00 CALL", -2160, "2025-11-14"),
            sto("sto2", "LUNR 21JAN28 12.00 CALL", 6, 6.75, "2025-12-10"),
            btc("btc", "LUNR 21JAN28 12.00 CALL", 22, 13.3, "2026-06-26", sub="BUYTOOPEN"),
        ],
        "market": {"fx": {}, "benchmark": {}},
    },
    # two multileg debits on one contract, a day apart, close 16 shorts (1, then the
    # 15 left) and carry them to the far contract; a second contract on the same
    # underlying is its own trade; 6 more sold on the far contract; all 22 bought
    # back in three fills. One chain trade, and the other contract's trade.
    "option_two_multilegs_then_chain": {
        "today": "2026-09-08",
        "activities": [
            sto("sto16", "LUNR 15JAN27 12.00 CALL", 16, 6.2225, "2025-07-24"),
            sto("sto2", "LUNR 15JAN27 10.00 CALL", 2, 6.9025, "2025-07-24"),
            btc("btc2", "LUNR 15JAN27 10.00 CALL", 2, 4.55, "2025-11-06", sub="BUYTOOPEN"),
            multileg("ml1", "LUNR 15JAN27 12.00 CALL", -128, "2025-11-13"),
            multileg("ml2", "LUNR 15JAN27 12.00 CALL", -2025, "2025-11-14"),
            sto("sto6", "LUNR 21JAN28 12.00 CALL", 6, 6.75, "2025-12-10"),
            btc("b7", "LUNR 21JAN28 12.00 CALL", 7, 13.3, "2026-06-26", sub="BUYTOOPEN"),
            btc("b10", "LUNR 21JAN28 12.00 CALL", 10, 13.3, "2026-06-26", sub="BUYTOOPEN"),
            btc("b5", "LUNR 21JAN28 12.00 CALL", 5, 13.45, "2026-06-26", sub="BUYTOOPEN"),
        ],
        "market": {"fx": {}, "benchmark": {}},
    },
    # short Dec puts rolled forward: the roll's only posted leg names a contract never
    # opened; the June buy-back of 26 closes the 11 known shorts, the carried leg and
    # the 9 old puts (nearest expiry first), and nothing stays open
    "option_rolled_chain_buy_back_closes_older_contracts": {
        "today": "2026-09-08",
        "activities": [
            sto("s1", "BBAI 26DEC25 5.50 PUT", 3, 0.12, "2025-12-05"),
            sto("s2", "BBAI 02JAN26 5.50 PUT", 5, 0.2, "2025-12-11"),
            sto("s4", "BBAI 19DEC25 6.00 PUT", 6, 0.2, "2025-12-12"),
            sto("s3", "BBAI 26DEC25 6.00 PUT", 1, 0.4, "2025-12-15"),
            multileg("ml1", "BBAI 19DEC25 6.00 PUT", -18, "2025-12-15"),
            multileg("ml2", "BBAI 18JUN26 5.00 PUT", -1830, "2025-12-18"),
            sto("s5", "BBAI 21JAN28 5.00 PUT", 11, 2.4, "2026-02-27"),
            btc("btc", "BBAI 21JAN28 5.00 PUT", 26, 2.74, "2026-06-29", sub="BUYTOOPEN"),
        ],
        "market": {"fx": {}, "benchmark": {}},
    },
    # two multileg debits with quantity 0 close a short in two pieces (1, then 15)
    "option_two_multilegs_close_a_short": {
        "today": "2026-09-08",
        "activities": [
            sto("sto", "LUNR 15JAN27 12.00 CALL", 16, 6.2225, "2026-01-10"),
            multileg("ml1", "LUNR 15JAN27 12.00 CALL", -128, "2026-03-01"),
            multileg("ml2", "LUNR 15JAN27 12.00 CALL", -2025, "2026-03-01"),
        ],
        "market": {"fx": {}, "benchmark": {}},
    },
    # credit multilegs on a short are a roll, one of them posted as SELLTOCLOSE
    "option_credit_multilegs_on_a_short": {
        "today": "2026-09-08",
        "activities": [
            sto("sto", "BBAI 21JAN28 10.00 CALL", 3, 1.2, "2026-01-05"),
            act(id="cr1", category="trade", activityType="OPTIONS_SELL", activitySubType="SELLTOCLOSE", rawType="OPTIONS_MULTILEG", quantity=0, netCashAmount=14, transactionDate="2026-02-01", symbol="BBAI 21JAN28 10.00 CALL"),
            multileg("cr2", "BBAI 21JAN28 10.00 CALL", 56, "2026-02-01"),
        ],
        "market": {"fx": {}, "benchmark": {}},
    },
    # a chain that folds twice: the first cover folds into the second contract, which is
    # then itself covered and folds, with that adjusted P&L, into the third
    "option_roll_chain_folded_twice": {
        "today": "2026-09-08",
        "activities": [
            sto("s1", "QQQ 20MAR26 5.00 PUT", 1, 0.5, "2025-12-01"),
            btc("c1", "QQQ 20MAR26 5.00 PUT", 1, 1.5, "2025-12-15"),
            sto("s2", "QQQ 17APR26 5.00 PUT", 1, 2.0, "2025-12-15"),
            btc("c2", "QQQ 17APR26 5.00 PUT", 1, 3.0, "2025-12-18"),
            sto("s3", "QQQ 15MAY26 5.00 PUT", 1, 4.0, "2025-12-18"),
            btc("c3", "QQQ 15MAY26 5.00 PUT", 1, 1.0, "2026-02-02"),
        ],
        "market": {"fx": {}, "benchmark": {}},
    },
    # a credit roll up: two multileg credits on the 10 call move 5 shorts to
    # the 12 call, then the 12 calls are bought back
    "option_credit_roll_up": {
        "today": "2026-09-01",
        "activities": [
            sto("sto", "BBAI 21JAN28 10.00 CALL", 5, 3.0, "2025-11-12"),
            multileg("cr1", "BBAI 21JAN28 10.00 CALL", 14, "2026-06-09"),
            multileg("cr2", "BBAI 21JAN28 10.00 CALL", 56, "2026-06-17"),
            btc("btc", "BBAI 21JAN28 12.00 CALL", 5, 0.85, "2026-06-26", sub="BUYTOOPEN"),
        ],
        "market": {"fx": {}, "benchmark": {}},
    },
    # a posted short expiry closes the short at zero and keeps the premium
    "option_short_expiry_posted": {
        "today": "2027-02-01",
        "activities": [
            sto("sto", "ABC 15JAN27 10.00 CALL", 5, 2, "2026-01-10"),
            act(id="exp", activityType="OPTIONS_SHORT_EXPIRY", activitySubType="EXPIRED", rawType="OPTIONS_SHORT_EXPIRY", quantity=5, transactionDate="2027-01-15", symbol="ABC 15JAN27 10.00 CALL"),
        ],
        "market": {"fx": {}, "benchmark": {}},
    },
    # Wealthsimple posted no expiry row: a lot still open after its expiry date
    # closes at zero on that date; a contract not yet expired stays open
    "option_expiry_assumed": {
        "today": "2026-03-01",
        "activities": [
            sto("sto", "BBAI 02JAN26 5.50 PUT", 2, 0.3, "2025-12-05"),
            btc("bto", "ZZZ 17JUL26 10.00 CALL", 1, 1.0, "2026-01-05", sub="BUYTOOPEN"),
        ],
        "market": {"fx": {}, "benchmark": {}},
    },
    # a long option that expired worthless, posted as a long expiry
    "option_long_expiry_posted": {
        "today": "2025-09-01",
        "activities": [
            btc("bto", "LUNR 22AUG25 8.00 CALL", 2, 0.4, "2025-07-01", sub="BUYTOOPEN"),
            act(id="exp", category="option_event", activityType="EXPIR", activitySubType="BUY", rawType="OPTIONS_EXPIRY", quantity=2, transactionDate="2025-08-22", symbol="LUNR 22AUG25 8.00 CALL"),
        ],
        "market": {"fx": {}, "benchmark": {}},
    },
    # an assigned covered call: the option keeps its premium and the shares are
    # sold at the strike; the share leg is derived, Wealthsimple posts only the option
    "option_assignment_call_delivers_shares": {
        "today": "2026-09-06",
        "activities": [
            buy("b1", "ASTS", 300, 25.0, "2025-01-10", currency="USD", securityId="sec-s-asts"),
            sto("sto", "ASTS 07MAR25 31.00 CALL", 3, 1.5, "2025-02-10", securityId="sec-o-asts"),
            act(id="asg", category="option_event", activityType="ASSIGN", activitySubType="BUYTOCLOSE", rawType="OPTIONS_ASSIGN", quantity=3, unitPrice=0, netCashAmount=9300, transactionDate="2025-03-07", symbol="ASTS 07MAR25 31.00 CALL", securityId="sec-o-asts"),
        ],
        "securities": [{"id": "sec-o-asts", "symbol": "ASTS", "underlyingId": "sec-s-asts"}, {"id": "sec-s-asts", "symbol": "ASTS", "name": "AST SpaceMobile", "primaryExchange": "NASDAQ"}],
        "market": {"fx": {"2025-01-10": 1.44, "2025-02-10": 1.43, "2025-03-07": 1.43}, "benchmark": {}},
    },
    # an assigned short put buys the shares at the strike: a new open position
    "option_assignment_put_buys_shares": {
        "today": "2026-01-01",
        "activities": [
            sto("sto", "BBAI 05DEC25 5.00 PUT", 1, 0.5, "2025-11-10"),
            act(id="asg", category="option_event", activityType="ASSIGN", activitySubType="BUYTOCLOSE", rawType="OPTIONS_ASSIGN", quantity=1, unitPrice=0, netCashAmount=-500, transactionDate="2025-12-05", symbol="BBAI 05DEC25 5.00 PUT"),
        ],
        "market": {"fx": {}, "benchmark": {}},
    },
    # crypto bought, a staking reward (a lot at zero cost), then everything sold
    # coins sent out of the account leave at cost: no slice, no P&L; the sale that
    # follows closes what is left, first-in first-out (1 ETH at 100, 1 at 120)
    "crypto_transfer_out_leaves_at_cost": {
        "today": "2026-03-01",
        "activities": [
            crypto("cb", "buy", "ETH", 2, 100, "2026-01-01"),
            crypto_transfer("ti", "ETH", 1, 120, "2026-01-05"),
            crypto_transfer("to", "ETH", 1, 200, "2026-01-10", out=True),
            crypto("cs", "sell", "ETH", 2, 150, "2026-02-01"),
        ],
        "market": {"fx": {}, "benchmark": {}},
    },
    "crypto_buy_reward_sell": {
        "today": "2026-03-01",
        "activities": [
            crypto("cb", "buy", "ETH", 2, 100, "2026-01-01"),
            crypto("rw", "reward", "ETH", 1, 0, "2026-01-05"),
            crypto("cs", "sell", "ETH", 3, 150, "2026-02-01"),
        ],
        "market": {"fx": {}, "benchmark": {}},
    },
    # a coin deposited (transferred in) then sold is not a scoreable trade: no buy
    # was made here, so it stands alone and is left out of the performance figures
    "crypto_deposited_coin_not_scored": {
        "today": "2026-03-01",
        "activities": [
            crypto("cb", "buy", "BTC", 1, 100, "2026-01-01"),
            crypto_transfer("dep", "BTC", 1, 100, "2026-01-05"),
            crypto("cs", "sell", "BTC", 2, 150, "2026-02-01"),
        ],
        "market": {"fx": {}, "benchmark": {}},
    },
    # a monthly payer between ex-date and pay day: the distribution still to be
    # paid is the one shown, its ex-date passed, its pay day not
    "cashflow_between_ex_date_and_pay_day": {
        "today": "2026-09-07",
        "activities": [
            buy("b1", "EASY", 1000, 20.0, "2026-05-01", accountType="Cashflow"),
            dividend("d1", "EASY", 1000, 0.20, "2026-07-08"),
            dividend("d2", "EASY", 1000, 0.20, "2026-08-08"),
        ],
        "market": {"fx": {}, "benchmark": {},
                   "distributions": {"EASY": [{"exDate": "2026-06-30", "payDate": "2026-07-08", "amount": 0.20, "currency": "CAD"},
                                              {"exDate": "2026-07-31", "payDate": "2026-08-08", "amount": 0.20, "currency": "CAD"},
                                              {"exDate": "2026-08-31", "payDate": "2026-09-08", "amount": 0.21, "currency": "CAD"}]}},
    },
    # the equity series with a pre-history balance, a deposit and a withdrawal: yearly
    # returns net of flows, the index over the same spans, annualized, drawdown
    "nav_yearly_returns_and_drawdown": {
        "today": "2026-09-07",
        "activities": [buy("b1", "AAA", 100, 10.0, "2026-01-05"), sell("s1", "AAA", 100, 12.0, "2026-01-20")],
        "nav": NAV,
        "market": {"fx": {}, "benchmark": SP500, "benchmarks": {"TSX": TSX}},
    },
    # the same series compared against the S&P/TSX instead
    "nav_against_the_tsx": {
        "today": "2026-09-07",
        "activities": [buy("b1", "AAA", 100, 10.0, "2026-01-05"), sell("s1", "AAA", 100, 12.0, "2026-01-20")],
        "nav": NAV,
        "market": {"fx": {}, "benchmark": SP500, "benchmarks": {"TSX": TSX}},
        "filters": {"benchmark": "TSX"},
    },
    # no filter: every trade and position, the journal on the trades, the dashboard cards
    "dashboard_journal_monthly_by_symbol_queue": {
        "today": "2026-09-07",
        "activities": FILTER_ACTS, "securities": FILTER_SECS, "journal": FILTER_JOURNAL, "nav": NAV,
        "nav_by_account": {"Trading": NAV[:6]},
        "market": FILTER_MARKET,
    },
    # one account and this year: trades scoped by close date, positions by account, the
    # equity series of that account, Cashflow scoped to the account and the dates
    "filters_account_and_ytd": {
        "today": "2026-09-07",
        "activities": FILTER_ACTS, "securities": FILTER_SECS, "journal": FILTER_JOURNAL, "nav": NAV,
        "nav_by_account": {"Trading": NAV[:6]},
        "market": FILTER_MARKET,
        "filters": {"lists": {"account": ["Trading"]}, "preset": "ytd"},
    },
    # a symbol filter matches an option by its underlying; Result keeps the winners
    "filters_symbol_by_underlying_and_result": {
        "today": "2026-09-07",
        "activities": FILTER_ACTS, "securities": FILTER_SECS, "journal": FILTER_JOURNAL, "nav": NAV,
        "market": FILTER_MARKET,
        "filters": {"lists": {"symbol": ["ZZZ", "AAA"], "result": ["Winners"]}},
    },
    # a year, a tag, a grade and a range together; Cashflow ignores the ones it cannot apply
    "filters_year_tag_grade_and_range": {
        "today": "2026-09-07",
        "activities": FILTER_ACTS, "securities": FILTER_SECS, "journal": FILTER_JOURNAL, "nav": NAV,
        "market": FILTER_MARKET,
        "filters": {"years": ["2026"], "lists": {"tag": ["earnings"], "grade": ["A", "Ungraded"]}, "ranges": {"hold": {"op": ">", "v": 5}}},
    },
    # kind and a date range; then exchange from the security record; then free text
    "filters_kind_and_date_range": {
        "today": "2026-09-07",
        "activities": FILTER_ACTS, "securities": FILTER_SECS, "journal": FILTER_JOURNAL, "nav": NAV,
        "market": FILTER_MARKET,
        "filters": {"lists": {"kind": ["Options", "Shares"]}, "from": "2026-02-01", "to": "2026-05-31"},
    },
    "filters_exchange_and_search": {
        "today": "2026-09-07",
        "activities": FILTER_ACTS, "securities": FILTER_SECS, "journal": FILTER_JOURNAL, "nav": NAV,
        "market": FILTER_MARKET,
        "filters": {"lists": {"exchange": ["TSX"]}, "search": "a"},
    },
    # no declared record: the rate and frequency come from the payments received
    # (quarterly, read from the gaps), the ex-date from the quote, the pay day
    # from the last payment
    "cashflow_holding_from_payments_only": {
        "today": "2026-09-07",
        "activities": [
            buy("b1", "QQQQ", 200, 50.0, "2025-10-01", accountType="Cashflow"),
            dividend("d1", "QQQQ", 200, 0.30, "2025-12-15"),
            dividend("d2", "QQQQ", 200, 0.30, "2026-03-16"),
            dividend("d3", "QQQQ", 200, 0.32, "2026-06-15"),
        ],
        "market": {"fx": {}, "benchmark": {}, "distributions": {}, "quotes": {"QQQQ": {"exDividendDate": "2026-09-15"}}},
    },
}


def rounded(v):
    if isinstance(v, float):
        return round(v, 6)
    if isinstance(v, dict):
        return {k: rounded(x) for k, x in v.items()}
    if isinstance(v, list):
        return [rounded(x) for x in v]
    return v


def pick(d, keys):
    return {k: d[k] for k in keys if k in d}


def expect_from(snap, market, today, filters, journal=None):
    """What every implementation must produce for one case: the view for the filters."""
    base = model.build_base(snap, market, journal or {}, today=today)
    view = model.build_view(base, filters)
    trades = sorted(view["trades"], key=lambda t: (t["entryDate"], t["exitDate"], t["symbol"]))
    cf = view["cashflow"]
    out = {
        "kpi": pick(view["kpi"], KPI_KEYS),
        "trades": [dict(pick(t, TRADE_KEYS), fills=[f["sub"] for f in sorted(t["fills"], key=lambda f: f["when"])]) for t in trades],
        "positions": [dict(pick(p, POSITION_KEYS), fills=[f["sub"] for f in sorted(p["fills"], key=lambda f: f["when"])]) for p in sorted(view["positions"], key=lambda p: (p["symbol"], p["account"]))],
        "positionsSummary": view["positionsSummary"],
        "portfolio": dict(pick(view["portfolio"], PORTFOLIO_KEYS), allocation=[pick(a, ALLOCATION_KEYS) for a in view["portfolio"]["allocation"]]),
        "equity": {"label": view["equity"]["label"], "series": [{"d": p["d"], "v": p["v"]} for p in view["equity"]["series"]],
                   "drawdown": view["equity"]["drawdown"], "annualized": view["equity"]["annualized"]},
        "years": [pick(y, YEAR_KEYS) for y in view["years"]],
        "benchmark": view["benchmark"],
        "monthly": [pick(m, MONTH_KEYS) for m in view["monthly"]],
        "bySymbol": [pick(r, SYMBOL_KEYS) for r in view["bySymbol"]],
        "grades": {"buckets": [pick(b, ("grade", "n", "pnl")) for b in view["grades"]["buckets"]], "ungraded": view["grades"]["ungraded"], "graded": view["grades"]["graded"]},
        "queue": [pick(q, QUEUE_KEYS) for q in view["queue"]],
        "options": {k: view["options"][k] for k in ("accounts", "symbols", "tags", "exchanges", "kinds", "years")},
        "cashflowHoldings": [pick(h, HOLDING_KEYS) for h in sorted(cf["holdings"], key=lambda h: h["symbol"])],
        "cashflowTiles": [pick(t, TILE_KEYS) for t in cf["tiles"]],
        "cashflowMonths": [pick(m, MONTH_KEYS) for m in cf["months"]],
        "cashflowTotal": cf["total"], "cashflowCount": cf["count"], "cashflowSkipped": cf["skippedFilters"],
    }
    return rounded(out)


def case_snapshot(case):
    return snapshot(case["activities"], case.get("securities"), case.get("nav"), case.get("nav_by_account"), case.get("accounts"), case.get("balances"), case.get("margin"))


def expect(case):
    return expect_from(case_snapshot(case), case["market"], case["today"], case.get("filters") or {}, case.get("journal"))


def main():
    for name, case in CASES.items():
        doc = {"today": case["today"], "snapshot": case_snapshot(case), "market": case["market"], "filters": case.get("filters") or {},
               "journal": case.get("journal") or {}, "expect": expect(case)}
        path = os.path.join(CASES_DIR, name + ".json")
        with open(path, "w") as f:
            json.dump(doc, f, indent=2, sort_keys=True)
            f.write("\n")
        print("wrote", os.path.relpath(path, os.path.dirname(ROOT)))


if __name__ == "__main__":
    main()
