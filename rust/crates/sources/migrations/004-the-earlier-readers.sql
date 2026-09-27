-- What the earlier readers keep (docs/plans/stage-6-cutover.md, 6a): news,
-- filings and their read text, exposures, gauges, short interest, universes,
-- and the charts' daily and intraday bars with when each span was read, as the
-- earlier store (bagholder.db) kept them. Everything here can be asked again
-- (docs/architecture.md §6), so it is the market cache's. The tables, their
-- columns and indexes are the earlier store's, unchanged in meaning: the
-- readers keep their SQL. Carried once from the earlier store at the first
-- start of this build (the server's `carry`), marked in `meta`.

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
CREATE INDEX news_published ON news (published_at);

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
CREATE INDEX filings_date ON filings (symbol, date DESC);

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

CREATE TABLE history_fetches (
    symbol TEXT PRIMARY KEY,
    start TEXT NOT NULL,
    fetched_at TEXT NOT NULL
);

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

CREATE TABLE bar_fetches (
    symbol TEXT NOT NULL,
    tf TEXT NOT NULL,
    start_ts INTEGER NOT NULL,
    fetched_at TEXT NOT NULL,
    PRIMARY KEY (symbol, tf)
);

-- What the readers remember by key: which form a symbol takes at TMX, which
-- source answered a chart or a listing's filings, a miss remembered for the
-- day, when a listing's news was last read, and the update check. The earlier
-- store's `meta`, holding only these.
CREATE TABLE meta (
    key TEXT PRIMARY KEY,
    value TEXT
);

-- A counter per table the market's context and the Releases tab read, moved by
-- the database itself when, and only when, a row's data changes (never for a
-- stamp of when it was read): the earlier store's generations for these tables.
CREATE TABLE gen (name TEXT PRIMARY KEY, n INTEGER NOT NULL DEFAULT 0);
INSERT INTO gen (name, n) VALUES ('exposures', 0), ('filings', 0), ('news', 0), ('universes', 0);
CREATE TRIGGER gen_exposures_ins AFTER INSERT ON exposures BEGIN UPDATE gen SET n = n + 1 WHERE name = 'exposures'; END;
CREATE TRIGGER gen_exposures_del AFTER DELETE ON exposures BEGIN UPDATE gen SET n = n + 1 WHERE name = 'exposures'; END;
CREATE TRIGGER gen_exposures_upd AFTER UPDATE ON exposures WHEN OLD."key" IS NOT NEW."key" OR OLD."sectors" IS NOT NEW."sectors" OR OLD."countries" IS NOT NEW."countries" OR OLD."coverage" IS NOT NEW."coverage" OR OLD."source" IS NOT NEW."source" OR OLD."as_of" IS NOT NEW."as_of" OR OLD."industry" IS NOT NEW."industry" OR OLD."error" IS NOT NEW."error" BEGIN UPDATE gen SET n = n + 1 WHERE name = 'exposures'; END;
CREATE TRIGGER gen_news_ins AFTER INSERT ON news BEGIN UPDATE gen SET n = n + 1 WHERE name = 'news'; END;
CREATE TRIGGER gen_news_del AFTER DELETE ON news BEGIN UPDATE gen SET n = n + 1 WHERE name = 'news'; END;
CREATE TRIGGER gen_news_upd AFTER UPDATE ON news WHEN OLD."id" IS NOT NEW."id" OR OLD."symbol" IS NOT NEW."symbol" OR OLD."exchange" IS NOT NEW."exchange" OR OLD."source" IS NOT NEW."source" OR OLD."headline" IS NOT NEW."headline" OR OLD."wire" IS NOT NEW."wire" OR OLD."url" IS NOT NEW."url" OR OLD."published_at" IS NOT NEW."published_at" OR OLD."kind" IS NOT NEW."kind" OR OLD."summary" IS NOT NEW."summary" BEGIN UPDATE gen SET n = n + 1 WHERE name = 'news'; END;
CREATE TRIGGER gen_universes_ins AFTER INSERT ON universes BEGIN UPDATE gen SET n = n + 1 WHERE name = 'universes'; END;
CREATE TRIGGER gen_universes_del AFTER DELETE ON universes BEGIN UPDATE gen SET n = n + 1 WHERE name = 'universes'; END;
CREATE TRIGGER gen_universes_upd AFTER UPDATE ON universes WHEN OLD."key" IS NOT NEW."key" OR OLD."symbol" IS NOT NEW."symbol" OR OLD."name" IS NOT NEW."name" OR OLD."value" IS NOT NEW."value" OR OLD."percent_change" IS NOT NEW."percent_change" OR OLD."sector" IS NOT NEW."sector" OR OLD."country" IS NOT NEW."country" BEGIN UPDATE gen SET n = n + 1 WHERE name = 'universes'; END;
CREATE TRIGGER gen_filings_ins AFTER INSERT ON filings BEGIN UPDATE gen SET n = n + 1 WHERE name = 'filings'; END;
CREATE TRIGGER gen_filings_del AFTER DELETE ON filings BEGIN UPDATE gen SET n = n + 1 WHERE name = 'filings'; END;
CREATE TRIGGER gen_filings_upd AFTER UPDATE ON filings WHEN OLD."symbol" IS NOT NEW."symbol" OR OLD."id" IS NOT NEW."id" OR OLD."source" IS NOT NEW."source" OR OLD."category" IS NOT NEW."category" OR OLD."profile_no" IS NOT NEW."profile_no" OR OLD."issuer" IS NOT NEW."issuer" OR OLD."type" IS NOT NEW."type" OR OLD."title" IS NOT NEW."title" OR OLD."date" IS NOT NEW."date" OR OLD."date_text" IS NOT NEW."date_text" OR OLD."size" IS NOT NEW."size" OR OLD."url" IS NOT NEW."url" OR OLD."subject" IS NOT NEW."subject" OR OLD."summary" IS NOT NEW."summary" OR OLD."enrich_version" IS NOT NEW."enrich_version" OR OLD."enrich_final" IS NOT NEW."enrich_final" BEGIN UPDATE gen SET n = n + 1 WHERE name = 'filings'; END;
