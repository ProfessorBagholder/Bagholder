-- The market cache (docs/plans/stage-3a-sources.md, "The market cache"): what
-- the sources answered that can be asked again. Nothing here is a fact a figure
-- was computed from (those are the book's), and deleting the file loses only
-- time. Keyed by the book's instrument ids. Decimals are canonical text, times
-- RFC 3339 instants, days YYYY-MM-DD.

-- The latest quote per instrument and source. `quoted_at` is when the source
-- says the price was current; `allowance_secs` is how old it may already have
-- been when that is not stated exactly (Coinbase's spot price).
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

-- A listing's or a coin's close per session day, as traded. A day is written
-- once: the first value stands (`first` = 1) and is what the figures read; a
-- later different value is kept beside it (`first` = 0), and recorded as a
-- meaning outcome.
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
CREATE UNIQUE INDEX daily_closes_standing ON daily_closes (instrument_id, day) WHERE first = 1;

-- An index's level per closed day, written once the same way.
CREATE TABLE benchmarks (
    benchmark TEXT NOT NULL,
    day TEXT NOT NULL,
    level TEXT NOT NULL,
    source TEXT NOT NULL,
    first INTEGER NOT NULL CHECK (first IN (0, 1)),
    received_at TEXT NOT NULL,
    PRIMARY KEY (benchmark, day, source, level)
) STRICT;
CREATE UNIQUE INDEX benchmarks_standing ON benchmarks (benchmark, day) WHERE first = 1;

-- The source and form that last answered for an instrument and kind of data,
-- asked first next time.
CREATE TABLE chains (
    instrument_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    source TEXT NOT NULL,
    form TEXT NOT NULL,
    won_at TEXT NOT NULL,
    PRIMARY KEY (instrument_id, kind)
) STRICT;

-- Every request's outcome. Per source, the newest thousand and the newest of
-- each outcome kind are kept, so a busy source never erases a quiet one's health.
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
CREATE INDEX outcomes_source ON outcomes (source, id);
