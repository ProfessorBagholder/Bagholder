-- What the person follows (docs/architecture.md §6; docs/plans/stage-5-interface-and-running.md,
-- A2): the watched listings and the Markets tab's tile row, each an instrument of
-- the book, found as §5 finds any instrument.

-- a listing followed without being held, newest first
CREATE TABLE watched (
    instrument_id TEXT PRIMARY KEY REFERENCES instruments(id),
    added_at TEXT NOT NULL
) STRICT;

-- the tile row, in the person's order; whether it was ever chosen is the
-- setting `tiles.chosen` (never chosen is the default row, chosen empty is empty)
CREATE TABLE tiles (
    instrument_id TEXT PRIMARY KEY REFERENCES instruments(id),
    position INTEGER NOT NULL UNIQUE
) STRICT;

-- what the person picked an instrument as, for one no record names: its symbol
-- on its venue and its name (a record's sightings name the others)
CREATE TABLE listings_named (
    instrument_id TEXT PRIMARY KEY REFERENCES instruments(id),
    symbol TEXT NOT NULL,
    venue_mic TEXT,
    venue_name TEXT,
    name TEXT,
    named_at TEXT NOT NULL
) STRICT;
