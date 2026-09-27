-- user_version 5
CREATE UNIQUE INDEX benchmark_closes_standing ON benchmark_closes (benchmark, day) WHERE first = 1;
CREATE UNIQUE INDEX benchmark_events_standing ON benchmark_events (benchmark, day, kind) WHERE first = 1;
CREATE UNIQUE INDEX daily_closes_standing ON daily_closes (instrument_id, day) WHERE first = 1;
CREATE INDEX filings_date ON filings (symbol, date DESC);
CREATE INDEX news_published ON news (published_at);
CREATE INDEX outcomes_source ON outcomes (source, id);
CREATE TABLE bar_fetches (
    symbol TEXT NOT NULL,
    tf TEXT NOT NULL,
    start_ts INTEGER NOT NULL,
    fetched_at TEXT NOT NULL,
    PRIMARY KEY (symbol, tf)
);
CREATE TABLE benchmark_closes (
    benchmark TEXT NOT NULL,
    day TEXT NOT NULL,
    close TEXT NOT NULL,
    source TEXT NOT NULL,
    first INTEGER NOT NULL CHECK (first IN (0, 1)),
    received_at TEXT NOT NULL,
    PRIMARY KEY (benchmark, day, source, close)
) STRICT;
CREATE TABLE benchmark_events (
    benchmark TEXT NOT NULL,
    day TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('dividend', 'split')),
    amount TEXT NOT NULL,
    denominator TEXT,
    source TEXT NOT NULL,
    first INTEGER NOT NULL CHECK (first IN (0, 1)),
    received_at TEXT NOT NULL,
    CHECK ((kind = 'split') = (denominator IS NOT NULL)),
    PRIMARY KEY (benchmark, day, kind, source, amount)
) STRICT;
CREATE TABLE chains (
    instrument_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    source TEXT NOT NULL,
    form TEXT NOT NULL,
    won_at TEXT NOT NULL,
    PRIMARY KEY (instrument_id, kind)
) STRICT;
CREATE TABLE daily_closes (
    instrument_id TEXT NOT NULL,
    day TEXT NOT NULL,
    close TEXT NOT NULL,
    currency TEXT NOT NULL,
    source TEXT NOT NULL,
    first INTEGER NOT NULL CHECK (first IN (0, 1)),
    received_at TEXT NOT NULL,
    PRIMARY KEY (instrument_id, day, source, close)
) STRICT;
CREATE TABLE exposures (
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
CREATE TABLE filings (
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
    enrich_reads INTEGER,
    fetched_at TEXT,
    PRIMARY KEY (symbol, id)
);
CREATE TABLE gauges (
    name TEXT PRIMARY KEY,
    source TEXT,
    score REAL,
    rating TEXT,
    as_of TEXT,
    payload TEXT,
    read_version INTEGER,
    fetched_at TEXT
);
CREATE TABLE gen (name TEXT PRIMARY KEY, n INTEGER NOT NULL DEFAULT 0);
CREATE TABLE history_fetches (
    symbol TEXT PRIMARY KEY,
    start TEXT NOT NULL,
    fetched_at TEXT NOT NULL
);
CREATE TABLE meta (
    key TEXT PRIMARY KEY,
    value TEXT
);
CREATE TABLE news (
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
    summary TEXT,
    PRIMARY KEY (id, symbol, exchange)
);
CREATE TABLE option_chains (
    underlying TEXT PRIMARY KEY,
    session TEXT NOT NULL,
    made_at TEXT NOT NULL,
    last_modified TEXT,
    received_at TEXT NOT NULL
) STRICT;
CREATE TABLE outcomes (
    id INTEGER PRIMARY KEY,
    source TEXT NOT NULL,
    host TEXT NOT NULL,
    kind TEXT NOT NULL,
    instrument_id TEXT,
    outcome TEXT NOT NULL CHECK (outcome IN ('answered', 'not-carried', 'refused', 'unreachable', 'mismatch', 'meaning')),
    detail TEXT NOT NULL,
    shape_change TEXT,
    at TEXT NOT NULL
) STRICT;
CREATE TABLE price_bars (
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
CREATE TABLE price_history (
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
CREATE TABLE quoted_ex_dividends (
    instrument_id TEXT NOT NULL PRIMARY KEY,
    source TEXT NOT NULL,
    ex_date TEXT NOT NULL,
    received_at TEXT NOT NULL
) STRICT;
CREATE TABLE quotes (
    instrument_id TEXT NOT NULL,
    source TEXT NOT NULL,
    price TEXT NOT NULL,
    currency TEXT NOT NULL,
    change TEXT,
    change_pct TEXT,
    quoted_at TEXT NOT NULL,
    allowance_secs INTEGER NOT NULL CHECK (allowance_secs >= 0),
    received_at TEXT NOT NULL,
    PRIMARY KEY (instrument_id, source)
) STRICT;
CREATE TABLE reads (
    subject TEXT NOT NULL,
    kind TEXT NOT NULL,
    source TEXT NOT NULL,
    first TEXT NOT NULL,
    last TEXT NOT NULL,
    outcome TEXT NOT NULL,
    at TEXT NOT NULL,
    PRIMARY KEY (subject, kind, source, first, last)
) STRICT;
CREATE TABLE schema_migrations (
                        number INTEGER PRIMARY KEY,
                        name TEXT NOT NULL,
                        applied_at TEXT NOT NULL,
                        app_version TEXT NOT NULL
                     ) STRICT;
CREATE TABLE shorts (
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
CREATE TABLE universes (
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
CREATE TRIGGER gen_exposures_del AFTER DELETE ON exposures BEGIN UPDATE gen SET n = n + 1 WHERE name = 'exposures'; END;
CREATE TRIGGER gen_exposures_ins AFTER INSERT ON exposures BEGIN UPDATE gen SET n = n + 1 WHERE name = 'exposures'; END;
CREATE TRIGGER gen_exposures_upd AFTER UPDATE ON exposures WHEN OLD."key" IS NOT NEW."key" OR OLD."sectors" IS NOT NEW."sectors" OR OLD."countries" IS NOT NEW."countries" OR OLD."coverage" IS NOT NEW."coverage" OR OLD."source" IS NOT NEW."source" OR OLD."as_of" IS NOT NEW."as_of" OR OLD."industry" IS NOT NEW."industry" OR OLD."error" IS NOT NEW."error" BEGIN UPDATE gen SET n = n + 1 WHERE name = 'exposures'; END;
CREATE TRIGGER gen_filings_del AFTER DELETE ON filings BEGIN UPDATE gen SET n = n + 1 WHERE name = 'filings'; END;
CREATE TRIGGER gen_filings_ins AFTER INSERT ON filings BEGIN UPDATE gen SET n = n + 1 WHERE name = 'filings'; END;
CREATE TRIGGER gen_filings_upd AFTER UPDATE ON filings WHEN OLD."symbol" IS NOT NEW."symbol" OR OLD."id" IS NOT NEW."id" OR OLD."source" IS NOT NEW."source" OR OLD."category" IS NOT NEW."category" OR OLD."profile_no" IS NOT NEW."profile_no" OR OLD."issuer" IS NOT NEW."issuer" OR OLD."type" IS NOT NEW."type" OR OLD."title" IS NOT NEW."title" OR OLD."date" IS NOT NEW."date" OR OLD."date_text" IS NOT NEW."date_text" OR OLD."size" IS NOT NEW."size" OR OLD."url" IS NOT NEW."url" OR OLD."subject" IS NOT NEW."subject" OR OLD."summary" IS NOT NEW."summary" OR OLD."enrich_version" IS NOT NEW."enrich_version" OR OLD."enrich_final" IS NOT NEW."enrich_final" BEGIN UPDATE gen SET n = n + 1 WHERE name = 'filings'; END;
CREATE TRIGGER gen_news_del AFTER DELETE ON news BEGIN UPDATE gen SET n = n + 1 WHERE name = 'news'; END;
CREATE TRIGGER gen_news_ins AFTER INSERT ON news BEGIN UPDATE gen SET n = n + 1 WHERE name = 'news'; END;
CREATE TRIGGER gen_news_upd AFTER UPDATE ON news WHEN OLD."id" IS NOT NEW."id" OR OLD."symbol" IS NOT NEW."symbol" OR OLD."exchange" IS NOT NEW."exchange" OR OLD."source" IS NOT NEW."source" OR OLD."headline" IS NOT NEW."headline" OR OLD."wire" IS NOT NEW."wire" OR OLD."url" IS NOT NEW."url" OR OLD."published_at" IS NOT NEW."published_at" OR OLD."kind" IS NOT NEW."kind" OR OLD."summary" IS NOT NEW."summary" BEGIN UPDATE gen SET n = n + 1 WHERE name = 'news'; END;
CREATE TRIGGER gen_universes_del AFTER DELETE ON universes BEGIN UPDATE gen SET n = n + 1 WHERE name = 'universes'; END;
CREATE TRIGGER gen_universes_ins AFTER INSERT ON universes BEGIN UPDATE gen SET n = n + 1 WHERE name = 'universes'; END;
CREATE TRIGGER gen_universes_upd AFTER UPDATE ON universes WHEN OLD."key" IS NOT NEW."key" OR OLD."symbol" IS NOT NEW."symbol" OR OLD."name" IS NOT NEW."name" OR OLD."value" IS NOT NEW."value" OR OLD."percent_change" IS NOT NEW."percent_change" OR OLD."sector" IS NOT NEW."sector" OR OLD."country" IS NOT NEW."country" BEGIN UPDATE gen SET n = n + 1 WHERE name = 'universes'; END;
