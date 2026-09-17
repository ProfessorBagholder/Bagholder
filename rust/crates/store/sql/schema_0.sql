CREATE TABLE IF NOT EXISTS meta (
    key TEXT PRIMARY KEY,
    value TEXT
);

CREATE TABLE IF NOT EXISTS activities (
    id TEXT PRIMARY KEY,
    canonical_id TEXT,
    occurred_at TEXT,
    transaction_date TEXT NOT NULL,
    settlement_date TEXT,
    account_id TEXT,
    book_id TEXT,
    fifo_id TEXT,
    account_type TEXT,
    activity_type TEXT,
    activity_sub_type TEXT,
    description TEXT,
    direction TEXT,
    symbol TEXT,
    name TEXT,
    currency TEXT,
    quantity REAL,
    unit_price REAL,
    commission REAL,
    net_cash_amount REAL,
    category TEXT,
    balance REAL,
    source TEXT,
    raw_type TEXT,
    aft_type TEXT,
    counter_symbol TEXT,
    security_id TEXT
);

CREATE TABLE IF NOT EXISTS securities (
    id TEXT PRIMARY KEY,
    symbol TEXT,
    name TEXT,
    primary_exchange TEXT,
    primary_mic TEXT,
    currency TEXT,
    underlying_id TEXT,
    fetched_at TEXT
);

CREATE UNIQUE INDEX IF NOT EXISTS activities_canonical_id_uq
    ON activities (canonical_id)
    WHERE canonical_id IS NOT NULL AND canonical_id != '';

CREATE TABLE IF NOT EXISTS accounts (
    id TEXT PRIMARY KEY,
    nickname TEXT,
    unified_account_type TEXT,
    currency TEXT,
    status TEXT,
    type TEXT,
    net_liquidation_value REAL
);

CREATE TABLE IF NOT EXISTS balances (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id TEXT,
    custodian_account_id TEXT,
    security_id TEXT,
    quantity REAL
);

CREATE TABLE IF NOT EXISTS nav_history (
    account_id TEXT NOT NULL DEFAULT '',
    date TEXT NOT NULL,
    equity REAL,
    currency TEXT,
    net_deposits REAL,
    PRIMARY KEY (account_id, date)
);

CREATE TABLE IF NOT EXISTS grouped_trades (
    id TEXT PRIMARY KEY,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS fx_rates (
    pair TEXT NOT NULL,
    date TEXT NOT NULL,
    rate REAL NOT NULL,
    PRIMARY KEY (pair, date)
);

CREATE TABLE IF NOT EXISTS benchmark_prices (
    symbol TEXT NOT NULL,
    date TEXT NOT NULL,
    close REAL NOT NULL,
    PRIMARY KEY (symbol, date)
);

CREATE TABLE IF NOT EXISTS distributions (
    symbol TEXT NOT NULL,
    ex_date TEXT NOT NULL,
    pay_date TEXT,
    amount REAL NOT NULL,
    currency TEXT,
    source TEXT NOT NULL DEFAULT 'tmx',
    PRIMARY KEY (symbol, ex_date, source)
);

CREATE TABLE IF NOT EXISTS quotes (
    symbol TEXT PRIMARY KEY,
    price REAL,
    dividend_amount REAL,
    dividend_frequency TEXT,
    ex_dividend_date TEXT,
    source TEXT,
    fetched_at TEXT
);

CREATE TABLE IF NOT EXISTS margin (
    account_id TEXT PRIMARY KEY,
    buying_power REAL,
    currency TEXT,
    unavailable TEXT,
    fetched_at TEXT
);

CREATE TABLE IF NOT EXISTS distribution_fetches (
    symbol TEXT PRIMARY KEY,
    fetched_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS price_history (
    symbol TEXT NOT NULL,
    date TEXT NOT NULL,
    open REAL,
    high REAL,
    low REAL,
    close REAL NOT NULL,
    volume REAL,
    source TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (symbol, date)
);

CREATE TABLE IF NOT EXISTS history_fetches (
    symbol TEXT PRIMARY KEY,
    start TEXT NOT NULL,
    fetched_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS price_bars (
    symbol TEXT NOT NULL,
    tf TEXT NOT NULL,
    ts INTEGER NOT NULL,
    open REAL,
    high REAL,
    low REAL,
    close REAL NOT NULL,
    volume REAL,
    source TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (symbol, tf, ts)
);

CREATE TABLE IF NOT EXISTS bar_fetches (
    symbol TEXT NOT NULL,
    tf TEXT NOT NULL,
    start_ts INTEGER NOT NULL,
    fetched_at TEXT NOT NULL,
    PRIMARY KEY (symbol, tf)
);

CREATE TABLE IF NOT EXISTS brackets (
    id TEXT PRIMARY KEY,
    order_id TEXT NOT NULL,
    created_at TEXT NOT NULL,
    account_id TEXT NOT NULL,
    security_id TEXT NOT NULL,
    symbol TEXT,
    currency TEXT,
    quantity REAL,
    tif TEXT,
    sl_kind TEXT,
    sl_price REAL,
    sl_trail REAL,
    sl_trail_unit TEXT,
    sl_order_id TEXT,
    sl_native INTEGER,
    sl_mode TEXT,
    high_water REAL,
    tp_price REAL,
    tp_order_id TEXT,
    status TEXT NOT NULL,
    outcome TEXT,
    error TEXT,
    attempts INTEGER,
    moved_at TEXT,
    armed_at TEXT,
    seen_held INTEGER,
    missed_at TEXT,
    updated_at TEXT
);

CREATE TABLE IF NOT EXISTS exposures (
    key TEXT PRIMARY KEY,
    sectors TEXT,
    countries TEXT,
    coverage REAL,
    source TEXT,
    as_of TEXT,
    industry TEXT,
    error TEXT,
    fetched_at TEXT
);

CREATE TABLE IF NOT EXISTS watchlist (
    symbol TEXT NOT NULL,
    exchange TEXT NOT NULL DEFAULT '',
    name TEXT,
    currency TEXT,
    security_id TEXT,
    added_at TEXT,
    PRIMARY KEY (symbol, exchange)
);

CREATE TABLE IF NOT EXISTS news (
    id TEXT NOT NULL,
    symbol TEXT NOT NULL,
    exchange TEXT NOT NULL DEFAULT '',
    source TEXT,
    headline TEXT,
    wire TEXT,
    url TEXT,
    published_at TEXT,
    fetched_at TEXT,
    kind TEXT,
    PRIMARY KEY (id, symbol, exchange)
);
CREATE INDEX IF NOT EXISTS news_published ON news (published_at);

CREATE TABLE IF NOT EXISTS universes (
    key TEXT NOT NULL,
    symbol TEXT NOT NULL,
    name TEXT,
    value REAL,
    percent_change REAL,
    sector TEXT,
    country TEXT,
    fetched_at TEXT,
    PRIMARY KEY (key, symbol)
);

CREATE TABLE IF NOT EXISTS orders (
    id TEXT PRIMARY KEY,
    created_at TEXT NOT NULL,
    account_id TEXT NOT NULL,
    account TEXT,
    security_id TEXT NOT NULL,
    symbol TEXT,
    currency TEXT,
    side TEXT NOT NULL,
    type TEXT NOT NULL,
    quantity REAL NOT NULL,
    limit_price REAL,
    stop_price REAL,
    tif TEXT NOT NULL,
    stop_loss TEXT,
    take_profit TEXT,
    status TEXT NOT NULL,
    ws_order_id TEXT,
    error TEXT,
    request TEXT,
    updated_at TEXT
);

CREATE TABLE IF NOT EXISTS filings (
    symbol TEXT NOT NULL,
    id TEXT NOT NULL,
    source TEXT NOT NULL DEFAULT '',
    category TEXT,
    profile_no TEXT,
    issuer TEXT,
    type TEXT,
    title TEXT,
    date TEXT,
    date_text TEXT,
    size TEXT,
    url TEXT,
    subject TEXT,
    summary TEXT,
    enriched_at TEXT,
    enrich_version INTEGER,
    enrich_final INTEGER,
    fetched_at TEXT,
    PRIMARY KEY (symbol, id)
);
CREATE INDEX IF NOT EXISTS filings_date ON filings (symbol, date DESC);

CREATE TABLE IF NOT EXISTS gauges (
    name TEXT PRIMARY KEY,
    source TEXT,
    score REAL,
    rating TEXT,
    as_of TEXT,
    payload TEXT,
    read_version INTEGER,
    fetched_at TEXT
);
CREATE TABLE IF NOT EXISTS shorts (
    symbol TEXT NOT NULL,
    exchange TEXT NOT NULL DEFAULT '',
    market TEXT,
    as_of TEXT,
    shares REAL,
    previous REAL,
    previous_of TEXT,
    change REAL,
    float_shares REAL,
    of_float REAL,
    average_volume REAL,
    days_to_cover REAL,
    volume_of TEXT,
    volume_span TEXT,
    short_volume REAL,
    total_volume REAL,
    volume_pct REAL,
    name TEXT,
    series TEXT,
    read_version INTEGER,
    fetched_at TEXT,
    PRIMARY KEY (symbol, exchange)
);
