-- Which instruments are one by a source's own succession (docs/architecture.md
-- §5, "Matching across sources"): an id a broker states retired by a
-- corporate action and the id its listing trades under now. Every broker id
-- keeps its own instrument; this says which instrument each is read as, and is
-- decided again from the records and standings after every write
-- (`Book::settle_successions`), so a later row that states the succession
-- otherwise parts them again.
CREATE TABLE instrument_joins (
    instrument_id TEXT PRIMARY KEY REFERENCES instruments(id),
    into_id TEXT NOT NULL REFERENCES instruments(id),
    CHECK (instrument_id <> into_id)
) STRICT;
CREATE INDEX instrument_joins_into ON instrument_joins (into_id);
