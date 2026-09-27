-- What a source's own description of a security names it (a broker's
-- security record, read for a holding its positions show that no row names: a
-- coin its broker moved to a new ticker, units given): the name it goes by
-- where no record names it. A record's name, once one comes, stands before
-- it; a later description replaces it.
CREATE TABLE instruments_described (
    instrument_id TEXT PRIMARY KEY REFERENCES instruments(id),
    source TEXT NOT NULL,
    symbol TEXT NOT NULL,
    venue_mic TEXT,
    venue_name TEXT,
    name TEXT,
    day TEXT NOT NULL
) STRICT;
