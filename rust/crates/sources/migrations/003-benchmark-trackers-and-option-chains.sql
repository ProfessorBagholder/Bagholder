-- The benchmarks are read as total returns from ETFs that track each index
-- (owner, 2026-09-24; docs/plans/stage-3a-brief-06.md). The index levels kept
-- before (FRED, Yahoo's ^GSPC, TMX's ^TSX and ^TX60) are a different series:
-- they go, with the reads that made them due.
DROP INDEX benchmarks_standing;
DROP TABLE benchmarks;
DELETE FROM reads WHERE kind = 'benchmark';

-- Each tracker's closes as traded, a closed day written once as an
-- instrument's are: a later different value is kept beside it (`first` = 0)
-- and the first stands.
CREATE TABLE benchmark_closes (
    benchmark TEXT NOT NULL,
    day TEXT NOT NULL,
    close TEXT NOT NULL,
    source TEXT NOT NULL,
    first INTEGER NOT NULL CHECK (first IN (0, 1)),
    received_at TEXT NOT NULL,
    PRIMARY KEY (benchmark, day, source, close)
) STRICT;
CREATE UNIQUE INDEX benchmark_closes_standing ON benchmark_closes (benchmark, day) WHERE first = 1;

-- Each tracker's dividends by ex-date (per unit, as declared) and splits by the
-- day they took effect (`numerator` new units for every `denominator` held), as
-- its source states them, written once like its closes.
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
CREATE UNIQUE INDEX benchmark_events_standing ON benchmark_events (benchmark, day, kind) WHERE first = 1;

-- No option contract's close is read any more: a held contract's price is
-- read from its chain when it is shown. Their reads and outcomes go.
DELETE FROM reads WHERE kind = 'option-close';
DELETE FROM outcomes WHERE kind = 'option-close';

-- The chain last read for each underlying, as it stated itself: the session it
-- carries (the day of the underlying's last trade), when Cboe made it, and its
-- `Last-Modified`, which the next read sends back so Cboe answers only with a
-- newer chain.
CREATE TABLE option_chains (
    underlying TEXT PRIMARY KEY,
    session TEXT NOT NULL,
    made_at TEXT NOT NULL,
    last_modified TEXT,
    received_at TEXT NOT NULL
) STRICT;
