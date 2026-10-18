-- The venue of the listing an order or bracket trades, as the broker states it
-- (brief 43): a fill of the app's own order states it, as the broker's row for the
-- order does. Orders written before it state none, and their fills read as on the
-- venue last stated (`names`).
ALTER TABLE orders ADD COLUMN venue_mic TEXT;
ALTER TABLE orders ADD COLUMN venue_name TEXT;
ALTER TABLE brackets ADD COLUMN venue_mic TEXT;
ALTER TABLE brackets ADD COLUMN venue_name TEXT;

-- a sighting that states no venue now reads as on the one last stated, never as on
-- none: every instrument folded whole once, when the book next opens
INSERT INTO names_due (instrument_id) SELECT id FROM instruments;
