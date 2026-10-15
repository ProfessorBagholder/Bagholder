-- An instrument's names kept as rows (`docs/plans/stage-3-a-change-on-a-deep-history.md`,
-- A): one row per unbroken run of live records' sightings under one symbol and
-- venue, under the instrument the runs are read as, keyed by the run's first
-- sighting. `Book::names` reads them where nothing is due; the fold over the
-- sightings stays the reference. No foreign keys: `Book::clear` deletes
-- sightings and instruments in its own order, and the triggers and the settle at
-- the end of its transaction keep the runs right for whatever it leaves.
CREATE TABLE instrument_names (
    instrument_id TEXT NOT NULL,
    first_day TEXT NOT NULL,
    first_symbol TEXT NOT NULL,
    first_sighting INTEGER NOT NULL,
    last_day TEXT NOT NULL,
    last_symbol TEXT NOT NULL,
    last_sighting INTEGER NOT NULL,
    symbol TEXT NOT NULL,
    venue_mic TEXT,
    venue_name TEXT,
    name TEXT,
    source TEXT NOT NULL,
    PRIMARY KEY (instrument_id, first_day, first_symbol, first_sighting)
) STRICT;

-- What a run is built from changed since it was last settled: a sighting, under
-- the instrument it is kept under (a member's own id, not its head), or, with no
-- sighting, an instrument whose runs are folded whole (a succession joined or
-- parted). Written by the triggers below, emptied by `Book::settle_names`.
CREATE TABLE names_due (
    instrument_id TEXT NOT NULL,
    day TEXT,
    symbol TEXT,
    sighting INTEGER
) STRICT;
CREATE INDEX names_due_instrument ON names_due (instrument_id);

-- a run's latest venue name and name, read by a seek from its last sighting back
CREATE INDEX instrument_sightings_venue_named ON instrument_sightings (instrument_id, day, symbol) WHERE venue_name IS NOT NULL;
CREATE INDEX instrument_sightings_named ON instrument_sightings (instrument_id, day, symbol) WHERE name IS NOT NULL;

-- the row an upsert replaces, marked before it goes: a REPLACE fires no delete
-- trigger on a connection without `recursive_triggers`, which a writer outside
-- the book may be
CREATE TRIGGER instrument_sightings_names_replaced BEFORE INSERT ON instrument_sightings BEGIN
    INSERT INTO names_due (instrument_id, day, symbol, sighting)
    SELECT instrument_id, day, symbol, rowid FROM instrument_sightings WHERE record_id = NEW.record_id AND leg = NEW.leg AND instrument_id = NEW.instrument_id;
END;

-- the row an UPDATE OR REPLACE onto another sighting's key deletes, marked before
-- it goes, for the same reason
CREATE TRIGGER instrument_sightings_names_rekeyed BEFORE UPDATE OF record_id, leg, instrument_id ON instrument_sightings BEGIN
    INSERT INTO names_due (instrument_id, day, symbol, sighting)
    SELECT instrument_id, day, symbol, rowid FROM instrument_sightings WHERE record_id = NEW.record_id AND leg = NEW.leg AND instrument_id = NEW.instrument_id AND rowid <> OLD.rowid;
END;

CREATE TRIGGER instrument_sightings_names_insert AFTER INSERT ON instrument_sightings BEGIN
    INSERT INTO names_due (instrument_id, day, symbol, sighting) VALUES (NEW.instrument_id, NEW.day, NEW.symbol, NEW.rowid);
END;

CREATE TRIGGER instrument_sightings_names_update AFTER UPDATE ON instrument_sightings BEGIN
    INSERT INTO names_due (instrument_id, day, symbol, sighting) VALUES (OLD.instrument_id, OLD.day, OLD.symbol, OLD.rowid);
    INSERT INTO names_due (instrument_id, day, symbol, sighting) VALUES (NEW.instrument_id, NEW.day, NEW.symbol, NEW.rowid);
END;

CREATE TRIGGER instrument_sightings_names_delete AFTER DELETE ON instrument_sightings BEGIN
    INSERT INTO names_due (instrument_id, day, symbol, sighting) VALUES (OLD.instrument_id, OLD.day, OLD.symbol, OLD.rowid);
END;

-- a record made live, revised or removed: each of its sightings
CREATE TRIGGER source_records_names_update AFTER UPDATE OF state ON source_records WHEN OLD.state IS NOT NEW.state BEGIN
    INSERT INTO names_due (instrument_id, day, symbol, sighting) SELECT instrument_id, day, symbol, rowid FROM instrument_sightings WHERE record_id = NEW.id;
END;

CREATE TRIGGER instrument_joins_names_insert AFTER INSERT ON instrument_joins BEGIN
    INSERT INTO names_due (instrument_id) VALUES (NEW.instrument_id), (NEW.into_id);
END;

CREATE TRIGGER instrument_joins_names_update AFTER UPDATE ON instrument_joins BEGIN
    INSERT INTO names_due (instrument_id) VALUES (OLD.instrument_id), (OLD.into_id), (NEW.instrument_id), (NEW.into_id);
END;

CREATE TRIGGER instrument_joins_names_delete AFTER DELETE ON instrument_joins BEGIN
    INSERT INTO names_due (instrument_id) VALUES (OLD.instrument_id), (OLD.into_id);
END;

-- the runs are read by the engine's ledger: the record's change log names the
-- instrument whose runs moved (migration 028)
CREATE TRIGGER instrument_names_changed_insert AFTER INSERT ON instrument_names BEGIN
    INSERT INTO ledger_changes (kind, key) SELECT 'instrument', k FROM (SELECT NEW.instrument_id AS k) WHERE k IS NOT NULL;
END;

CREATE TRIGGER instrument_names_changed_update AFTER UPDATE ON instrument_names BEGIN
    INSERT INTO ledger_changes (kind, key) SELECT 'instrument', k FROM (SELECT OLD.instrument_id AS k) WHERE k IS NOT NULL;
    INSERT INTO ledger_changes (kind, key) SELECT 'instrument', k FROM (SELECT NEW.instrument_id AS k) WHERE k IS NOT NULL;
END;

CREATE TRIGGER instrument_names_changed_delete AFTER DELETE ON instrument_names BEGIN
    INSERT INTO ledger_changes (kind, key) SELECT 'instrument', k FROM (SELECT OLD.instrument_id AS k) WHERE k IS NOT NULL;
END;

-- every instrument folded whole once, when the book next opens
INSERT INTO names_due (instrument_id) SELECT id FROM instruments;
