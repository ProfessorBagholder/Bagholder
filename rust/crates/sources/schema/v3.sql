-- user_version 3
CREATE UNIQUE INDEX benchmark_closes_standing ON benchmark_closes (benchmark, day) WHERE first = 1;
CREATE UNIQUE INDEX benchmark_events_standing ON benchmark_events (benchmark, day, kind) WHERE first = 1;
CREATE UNIQUE INDEX daily_closes_standing ON daily_closes (instrument_id, day) WHERE first = 1;
CREATE INDEX outcomes_source ON outcomes (source, id);
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
