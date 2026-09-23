-- The facts a figure is computed from, kept because none of them can be fetched
-- again as it was when it was used (docs/architecture.md §6,
-- docs/plans/stage-2-engine.md "Facts and adjustments in the book"), and a
-- trade's anchor naming the instrument it opened.

-- One transaction can open two round trips (an assignment closes the contract's
-- and opens the underlying's), so an anchor is a transaction and an instrument.
ALTER TABLE trades ADD COLUMN anchor_instrument TEXT REFERENCES instruments(id);
UPDATE trades SET anchor_instrument = (
    SELECT t.instrument_id FROM transactions t WHERE t.record_id = trades.anchor_record AND t.leg = trades.anchor_leg
) WHERE anchor_record IS NOT NULL;
DROP INDEX trades_anchor;
CREATE UNIQUE INDEX trades_anchor ON trades (anchor_record, anchor_leg, anchor_instrument) WHERE anchor_record IS NOT NULL;

-- Every observation the Bank of Canada published, as received: CAD per unit of
-- the currency. The first stored for a day is the rate; a later different value
-- is kept beside it and reported, and never replaces it.
CREATE TABLE fx_rates (
    currency TEXT NOT NULL CHECK (length(currency) = 3),
    day TEXT NOT NULL,
    rate TEXT NOT NULL,
    source TEXT NOT NULL,
    received_at TEXT NOT NULL,
    PRIMARY KEY (currency, day, rate)
) STRICT;

-- The span of days each completed read of a series covered: a weekday inside
-- one with no rate was not published, one outside every read is a rate not read.
CREATE TABLE fx_reads (
    currency TEXT NOT NULL CHECK (length(currency) = 3),
    first_day TEXT NOT NULL,
    last_day TEXT NOT NULL,
    source TEXT NOT NULL,
    received_at TEXT NOT NULL,
    CHECK (first_day <= last_day)
) STRICT;
CREATE INDEX fx_reads_currency ON fx_reads (currency, first_day);

-- The currencies the Bank publishes a rate for, as its list of series says.
CREATE TABLE fx_series (
    currency TEXT PRIMARY KEY CHECK (length(currency) = 3),
    source TEXT NOT NULL,
    received_at TEXT NOT NULL
) STRICT;

-- The Bank's own holiday schedule.
CREATE TABLE bank_holidays (
    day TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    source TEXT NOT NULL,
    received_at TEXT NOT NULL
) STRICT;

-- Each read of a fund's declared distributions, whole: a distribution the fund
-- withdrew is absent from a later read.
CREATE TABLE declared_reads (
    id INTEGER PRIMARY KEY,
    instrument_id TEXT NOT NULL REFERENCES instruments(id),
    source TEXT NOT NULL,
    read_at TEXT NOT NULL
) STRICT;
CREATE INDEX declared_reads_instrument ON declared_reads (instrument_id, read_at);
CREATE TABLE declared_distributions (
    read_id INTEGER NOT NULL REFERENCES declared_reads(id),
    ex_date TEXT NOT NULL,
    record_date TEXT,
    pay_date TEXT,
    amount TEXT NOT NULL,
    currency TEXT NOT NULL CHECK (length(currency) = 3),
    kind TEXT NOT NULL CHECK (kind IN ('regular', 'special', 'non-cash'))
) STRICT;
CREATE INDEX declared_distributions_read ON declared_distributions (read_id);

-- Payments per year as a source states it.
CREATE TABLE stated_frequencies (
    instrument_id TEXT NOT NULL REFERENCES instruments(id),
    per_year INTEGER NOT NULL CHECK (per_year > 0),
    source TEXT NOT NULL,
    stated_at TEXT,
    received_at TEXT NOT NULL,
    PRIMARY KEY (instrument_id, received_at)
) STRICT;

-- The daily closes no source can give again later (an option contract's): the
-- first stored for a day stands.
CREATE TABLE recorded_closes (
    instrument_id TEXT NOT NULL REFERENCES instruments(id),
    day TEXT NOT NULL,
    close TEXT NOT NULL,
    currency TEXT NOT NULL CHECK (length(currency) = 3),
    source TEXT NOT NULL,
    received_at TEXT NOT NULL,
    PRIMARY KEY (instrument_id, day)
) STRICT;

-- What a corporate event did, or what the person states about a transaction,
-- derived from a record like its transactions (the issuer's notice, the
-- exchange's bulletin, the person's own entry). It explains one transaction,
-- which a supersede moves as it moves a trade's anchor.
CREATE TABLE adjustments (
    record_id TEXT NOT NULL REFERENCES source_records(id),
    leg TEXT NOT NULL,
    applies_record TEXT NOT NULL,
    applies_leg TEXT NOT NULL,
    PRIMARY KEY (record_id, leg)
) STRICT;
CREATE INDEX adjustments_applies ON adjustments (applies_record, applies_leg);
CREATE TABLE adjustment_legs (
    record_id TEXT NOT NULL,
    leg TEXT NOT NULL,
    position INTEGER NOT NULL,
    from_instrument TEXT REFERENCES instruments(id),
    to_instrument TEXT REFERENCES instruments(id),
    units_per_unit TEXT,
    cost_share TEXT,
    cash_per_unit TEXT,
    cash_currency TEXT,
    cost TEXT,
    cost_currency TEXT,
    acquired TEXT,
    PRIMARY KEY (record_id, leg, position),
    FOREIGN KEY (record_id, leg) REFERENCES adjustments(record_id, leg),
    CHECK ((cash_per_unit IS NULL) = (cash_currency IS NULL)),
    CHECK ((cost IS NULL) = (cost_currency IS NULL)),
    CHECK (from_instrument IS NOT NULL OR to_instrument IS NOT NULL)
) STRICT;
