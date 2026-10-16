-- Market data kept per listing and filings per filer, never per ticker (plan
-- `market-data-keyed-by-listing`; brief 40). What one listing's readers keep is
-- keyed by its `ListingKey` (`SYMBOL@VENUE`: a Canadian venue's code, `US`,
-- `CRYPTO`, `INDEX` or `OCC`). Those rows were kept by a bare ticker that two
-- listings could share, so they cannot be told apart and are read again ("deleting
-- it loses only time", docs/architecture.md §6). Filings are carried: a document
-- never changes and its read text cost a fetch and a model's read.

-- charts: daily and intraday bars, and how far each was read
DROP TABLE price_history;
CREATE TABLE price_history (
    listing TEXT NOT NULL,
    date TEXT NOT NULL,
    open REAL,
    high REAL,
    low REAL,
    close REAL NOT NULL,
    volume REAL,
    source TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (listing, date)
);
DROP TABLE history_fetches;
CREATE TABLE history_fetches (
    listing TEXT PRIMARY KEY,
    start TEXT NOT NULL,
    fetched_at TEXT NOT NULL
);
DROP TABLE price_bars;
CREATE TABLE price_bars (
    listing TEXT NOT NULL,
    tf TEXT NOT NULL,
    ts INTEGER NOT NULL,
    open REAL,
    high REAL,
    low REAL,
    close REAL NOT NULL,
    volume REAL,
    source TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (listing, tf, ts)
);
DROP TABLE bar_fetches;
CREATE TABLE bar_fetches (
    listing TEXT NOT NULL,
    tf TEXT NOT NULL,
    start_ts INTEGER NOT NULL,
    fetched_at TEXT NOT NULL,
    PRIMARY KEY (listing, tf)
);

-- short interest, per listing
DROP TABLE shorts;
CREATE TABLE shorts (
    listing TEXT PRIMARY KEY,
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
    fetched_at TEXT
);

-- news, per listing
DROP TABLE news;
CREATE TABLE news (
    id TEXT NOT NULL,
    listing TEXT NOT NULL,
    -- the venue in the words the listing was read under, for display alone
    exchange TEXT NOT NULL DEFAULT '',
    source TEXT,
    headline TEXT,
    wire TEXT,
    url TEXT,
    published_at TEXT,
    fetched_at TEXT,
    kind TEXT,
    summary TEXT,
    PRIMARY KEY (id, listing)
);
CREATE INDEX news_published ON news (published_at);
CREATE TRIGGER gen_news_ins AFTER INSERT ON news BEGIN UPDATE gen SET n = n + 1 WHERE name = 'news'; END;
CREATE TRIGGER gen_news_del AFTER DELETE ON news BEGIN UPDATE gen SET n = n + 1 WHERE name = 'news'; END;
CREATE TRIGGER gen_news_upd AFTER UPDATE ON news WHEN OLD."id" IS NOT NEW."id" OR OLD."listing" IS NOT NEW."listing" OR OLD."exchange" IS NOT NEW."exchange" OR OLD."source" IS NOT NEW."source" OR OLD."headline" IS NOT NEW."headline" OR OLD."wire" IS NOT NEW."wire" OR OLD."url" IS NOT NEW."url" OR OLD."published_at" IS NOT NEW."published_at" OR OLD."kind" IS NOT NEW."kind" OR OLD."summary" IS NOT NEW."summary" BEGIN UPDATE gen SET n = n + 1 WHERE name = 'news'; END;

-- a share's and a fund's breakdowns were kept by ticker (`share:<SYM>:<form>`,
-- `fund:<SYM>`); they are read again under the listing's key
DELETE FROM exposures WHERE key LIKE 'share:%' OR key LIKE 'fund:%';

-- filings: one row per document, under its source's own id, with the filer it
-- belongs to (an SEC CIK, or a SEDAR+ profile number). A row's filer is read from
-- the row itself: a SEDAR+ row's profile, else its ticker's stored SEDAR+ profile;
-- an SEC row's CIK from its address. A document stored under two tickers becomes
-- one row, keeping the most complete read. A row naming no filer is dropped.
CREATE TABLE filings_v6 (
    source TEXT NOT NULL,
    id TEXT NOT NULL,
    filer TEXT NOT NULL,
    category TEXT,
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
    PRIMARY KEY (source, id)
);
CREATE TABLE filings_carried (
    symbol TEXT NOT NULL,
    source TEXT NOT NULL,
    filer TEXT NOT NULL,
    PRIMARY KEY (symbol, source, filer)
);
CREATE TEMP TABLE filed AS
SELECT f.*, CASE
        WHEN f.source = 'SEDAR+' AND COALESCE(f.profile_no, '') <> '' THEN f.profile_no
        WHEN f.source = 'SEDAR+' THEN (SELECT NULLIF(m.value, '') FROM meta m WHERE m.key = 'sedar_profile:' || f.symbol)
        WHEN f.source = 'SEC' AND instr(f.url, '/edgar/data/') > 0
            THEN CAST(CAST(substr(substr(f.url, instr(f.url, '/edgar/data/') + 12), 1, instr(substr(f.url, instr(f.url, '/edgar/data/') + 12), '/') - 1) AS INTEGER) AS TEXT)
        WHEN f.source = 'SEC' AND instr(f.url, 'CIK=') > 0
            THEN CAST(CAST(substr(f.url, instr(f.url, 'CIK=') + 4) AS INTEGER) AS TEXT)
    END AS filer
FROM filings f;
INSERT INTO filings_carried (symbol, source, filer)
SELECT DISTINCT symbol, source, filer FROM filed WHERE filer IS NOT NULL AND filer NOT IN ('', '0');
INSERT INTO filings_v6 (source, id, filer, category, issuer, type, title, date, date_text, size, url, subject, summary, enriched_at, enrich_version, enrich_final, enrich_reads, fetched_at)
SELECT source, id, filer, category, issuer, type, title, date, date_text, size, url, subject, summary, enriched_at, enrich_version, enrich_final, enrich_reads, fetched_at
FROM (
    SELECT *, ROW_NUMBER() OVER (
        PARTITION BY source, id
        ORDER BY COALESCE(enrich_final, 0) DESC, COALESCE(enrich_version, 0) DESC, enriched_at IS NULL, enriched_at DESC, fetched_at DESC
    ) AS rank
    FROM filed
    WHERE filer IS NOT NULL AND filer NOT IN ('', '0')
)
WHERE rank = 1;
-- what the carry dropped, for the record: rows that named no filer
INSERT OR REPLACE INTO meta (key, value)
SELECT 'carry:filings_without_a_filer', CAST(COUNT(*) AS TEXT) FROM filed WHERE filer IS NULL OR filer IN ('', '0');
DROP TABLE filed;
DROP TABLE filings;
ALTER TABLE filings_v6 RENAME TO filings;
CREATE INDEX filings_filer ON filings (source, filer, date DESC);
CREATE TRIGGER gen_filings_ins AFTER INSERT ON filings BEGIN UPDATE gen SET n = n + 1 WHERE name = 'filings'; END;
CREATE TRIGGER gen_filings_del AFTER DELETE ON filings BEGIN UPDATE gen SET n = n + 1 WHERE name = 'filings'; END;
CREATE TRIGGER gen_filings_upd AFTER UPDATE ON filings WHEN OLD."source" IS NOT NEW."source" OR OLD."id" IS NOT NEW."id" OR OLD."filer" IS NOT NEW."filer" OR OLD."category" IS NOT NEW."category" OR OLD."issuer" IS NOT NEW."issuer" OR OLD."type" IS NOT NEW."type" OR OLD."title" IS NOT NEW."title" OR OLD."date" IS NOT NEW."date" OR OLD."date_text" IS NOT NEW."date_text" OR OLD."size" IS NOT NEW."size" OR OLD."url" IS NOT NEW."url" OR OLD."subject" IS NOT NEW."subject" OR OLD."summary" IS NOT NEW."summary" OR OLD."enrich_version" IS NOT NEW."enrich_version" OR OLD."enrich_final" IS NOT NEW."enrich_final" BEGIN UPDATE gen SET n = n + 1 WHERE name = 'filings'; END;

-- which filers a listing's filings come from, and how each was settled: the SEC's
-- ticker-and-exchange list (`sec-ticker`), SEDAR+'s exact name (`sedar-name`), or
-- the TSX's interlisted list pairing it with another listing (`interlisted`)
CREATE TABLE listing_filers (
    listing TEXT NOT NULL,
    source TEXT NOT NULL,
    filer TEXT NOT NULL,
    settled_by TEXT NOT NULL CHECK (settled_by IN ('sec-ticker', 'sedar-name', 'interlisted')),
    checked_at TEXT NOT NULL,
    PRIMARY KEY (listing, source, filer)
);
-- a listing whose filer could not be settled, and why, as it was last asked
CREATE TABLE listing_filer_misses (
    listing TEXT NOT NULL,
    source TEXT NOT NULL,
    why TEXT NOT NULL,
    checked_at TEXT NOT NULL,
    PRIMARY KEY (listing, source)
);
-- when each filer's filings were last read
CREATE TABLE filer_fetches (
    source TEXT NOT NULL,
    filer TEXT NOT NULL,
    fetched_at TEXT NOT NULL,
    PRIMARY KEY (source, filer)
);
-- the TSX's interlisted list as last read: each Canadian listing and the US
-- listing it is paired with
CREATE TABLE interlisted (
    listing TEXT PRIMARY KEY,
    us_listing TEXT NOT NULL,
    name TEXT NOT NULL,
    as_of TEXT NOT NULL,
    read_at TEXT NOT NULL
);

-- every meta key that named a ticker, and so a listing it could not tell apart
DELETE FROM meta WHERE key LIKE 'bars_source:%' OR key LIKE 'bars_miss:%' OR key LIKE 'tmx_form:%'
    OR key LIKE 'news_fetched:%' OR key LIKE 'news_source_fetched:%'
    OR key LIKE 'filings_fetched:%' OR key LIKE 'filings_sources:%' OR key LIKE 'sedar_profile:%';
