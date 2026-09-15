"""Market instruments the watchlist can follow beside listings: the indices, futures,
rates and currency pairs people watch, each with the code Yahoo's chart endpoint quotes
it under. Nothing here is traded from the app; the directory exists so ⌘K finds `WTI`,
`NDX` or `VIX` and the watchlist can quote them."""
from __future__ import annotations

KIND_LABEL = {"Index": "Indices", "Future": "Futures", "Commodity": "Commodities", "Rate": "Rates", "Currency": "Currencies"}

# symbol, name, kind, venue shown, currency, Yahoo code, aliases (matched whole, case-insensitively)
INSTRUMENTS = [
    ("SPX", "S&P 500", "Index", "Index", "USD", "^GSPC", ("S&P", "S&P500", "SP500", "GSPC")),
    ("NDX", "Nasdaq 100", "Index", "Index", "USD", "^NDX", ("NASDAQ100", "NASDAQ 100")),
    ("IXIC", "Nasdaq Composite", "Index", "Index", "USD", "^IXIC", ("NASDAQ", "COMP")),
    ("DJI", "Dow Jones Industrial Average", "Index", "Index", "USD", "^DJI", ("DJIA", "DOW", "DOW JONES")),
    ("RUT", "Russell 2000", "Index", "Index", "USD", "^RUT", ("RUSSELL", "RUSSELL 2000")),
    ("VIX", "CBOE Volatility Index", "Index", "Index", "USD", "^VIX", ("VOLATILITY",)),
    ("TSX", "S&P/TSX Composite", "Index", "Index", "CAD", "^GSPTSE", ("GSPTSE", "TSX COMPOSITE", "S&P/TSX")),
    ("FTSE", "FTSE 100", "Index", "Index", "GBP", "^FTSE", ("FTSE 100",)),
    ("DAX", "DAX", "Index", "Index", "EUR", "^GDAXI", ("GDAXI",)),
    ("N225", "Nikkei 225", "Index", "Index", "JPY", "^N225", ("NIKKEI", "NIKKEI 225")),
    ("HSI", "Hang Seng", "Index", "Index", "HKD", "^HSI", ("HANG SENG",)),
    ("STOXX50E", "Euro Stoxx 50", "Index", "Index", "EUR", "^STOXX50E", ("STOXX", "EURO STOXX")),
    ("DXY", "US Dollar Index", "Index", "Index", "USD", "DX-Y.NYB", ("DOLLAR INDEX",)),
    # the equity index futures trade nearly around the clock: the read on the market after hours
    ("ES", "S&P 500 E-mini futures", "Future", "CME", "USD", "ES=F", ("ES=F", "S&P FUTURES", "S&P 500 FUTURES", "SPX FUTURES", "ES FUTURES", "FUTURES")),
    ("NQ", "Nasdaq 100 E-mini futures", "Future", "CME", "USD", "NQ=F", ("NQ=F", "NASDAQ FUTURES", "NASDAQ 100 FUTURES", "NQ FUTURES")),
    ("YM", "Dow E-mini futures", "Future", "CBOT", "USD", "YM=F", ("YM=F", "DOW FUTURES", "YM FUTURES")),
    ("RTY", "Russell 2000 E-mini futures", "Future", "CME", "USD", "RTY=F", ("RTY=F", "RUSSELL FUTURES", "RTY FUTURES")),
    ("CL", "Crude Oil (WTI)", "Commodity", "NYMEX", "USD", "CL=F", ("WTI", "CRUDE", "OIL", "CRUDE OIL")),
    ("BZ", "Brent Crude Oil", "Commodity", "ICE", "USD", "BZ=F", ("BRENT",)),
    ("NG", "Natural Gas", "Commodity", "NYMEX", "USD", "NG=F", ("NATGAS", "NATURAL GAS", "GAS")),
    ("GC", "Gold", "Commodity", "COMEX", "USD", "GC=F", ("GOLD",)),
    ("SI", "Silver", "Commodity", "COMEX", "USD", "SI=F", ("SILVER",)),
    ("HG", "Copper", "Commodity", "COMEX", "USD", "HG=F", ("COPPER",)),
    ("PL", "Platinum", "Commodity", "NYMEX", "USD", "PL=F", ("PLATINUM",)),
    ("ZC", "Corn", "Commodity", "CBOT", "USD", "ZC=F", ("CORN",)),
    ("ZW", "Wheat", "Commodity", "CBOT", "USD", "ZW=F", ("WHEAT",)),
    ("TNX", "US 10-Year Treasury Yield", "Rate", "Index", "USD", "^TNX", ("10Y", "10-YEAR", "10 YEAR", "TREASURY", "YIELD")),
    ("USDCAD", "US Dollar / Canadian Dollar", "Currency", "FX", "CAD", "CAD=X", ("USD/CAD", "CAD", "LOONIE")),
    ("EURUSD", "Euro / US Dollar", "Currency", "FX", "USD", "EURUSD=X", ("EUR/USD", "EURO")),
    ("GBPUSD", "British Pound / US Dollar", "Currency", "FX", "USD", "GBPUSD=X", ("GBP/USD", "POUND")),
    ("USDJPY", "US Dollar / Japanese Yen", "Currency", "FX", "JPY", "JPY=X", ("USD/JPY", "YEN")),
    ("BTCUSD", "Bitcoin / US Dollar", "Currency", "FX", "USD", "BTC-USD", ("BITCOIN", "BTC")),
]


# what a market tile calls the instrument: the symbol unless people know it by a name
LABELS = {"CL": "WTI", "BZ": "BRENT", "NG": "NATGAS", "GC": "GOLD", "SI": "SILVER", "HG": "COPPER", "PL": "PLATINUM", "ZC": "CORN", "ZW": "WHEAT",
          "TNX": "10Y", "USDCAD": "USD/CAD", "EURUSD": "EUR/USD", "GBPUSD": "GBP/USD", "USDJPY": "USD/JPY", "BTCUSD": "BITCOIN"}


def label(symbol):
    sym = str(symbol or "").strip().upper()
    return LABELS.get(sym, sym)


def _rows():
    return [{"symbol": s, "name": n, "kind": k, "exchange": v, "currency": c, "yahoo": y, "aliases": a} for s, n, k, v, c, y, a in INSTRUMENTS]


def find(symbol, exchange=""):
    """The instrument a watched row is, by its symbol and venue, or None for a listing."""
    sym, ex = str(symbol or "").strip().upper(), str(exchange or "").strip().upper()
    for r in _rows():
        if r["symbol"] == sym and r["exchange"].upper() == ex:
            return r
    return None


def search(text):
    """Instruments matching the text: an exact symbol or alias first, then a symbol, name or
    alias starting with it, then one with a word starting with it. A single letter matches
    only an exact symbol, so `V` finds Visa's listings and not every index with a V in it."""
    q = str(text or "").strip().upper()
    if not q:
        return []
    out = []
    for r in _rows():
        names = [r["symbol"]] + [a.upper() for a in r["aliases"]]
        words = [w for x in names + [r["name"].upper()] for w in x.replace("/", " ").split()]
        if q in names:
            rank = 0
        elif len(q) < 2:
            rank = -1
        elif any(x.startswith(q) for x in names + [r["name"].upper()]):
            rank = 1
        elif any(w.startswith(q) for w in words):
            rank = 2
        else:
            rank = -1
        if rank >= 0:
            out.append((rank, {"symbol": r["symbol"], "name": r["name"], "exchange": r["exchange"], "currency": r["currency"], "kind": r["kind"], "rank": rank}))
    out.sort(key=lambda x: x[0])
    return [r for _, r in out]
