-- user_version 2
CREATE UNIQUE INDEX benchmarks_standing ON benchmarks (benchmark, day) WHERE first = 1;
CREATE UNIQUE INDEX daily_closes_standing ON daily_closes (instrument_id, day) WHERE first = 1;
CREATE INDEX outcomes_source ON outcomes (source, id);
CREATE TABLE benchmarks (
    benchmark TEXT NOT NULL,
    day TEXT NOT NULL,
    level TEXT NOT NULL,
    source TEXT NOT NULL,
    first INTEGER NOT NULL CHECK (first IN (0, 1)),
    received_at TEXT NOT NULL,
    PRIMARY KEY (benchmark, day, source, level)
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
