-- What a broker states of each of its own security ids (docs/architecture.md
-- §5, "Matching across sources"): still traded under, retired by a corporate
-- action, or delisted, with the listing it named as it said so. Written as
-- records are derived and as a statement names a security no record does; a
-- book from before this is filled when its records are derived again.
CREATE TABLE security_standings (
    scheme TEXT NOT NULL,
    value TEXT NOT NULL,
    standing TEXT NOT NULL CHECK (standing IN ('live', 'retired-by-event', 'delisted')),
    kind TEXT NOT NULL,
    currency TEXT NOT NULL,
    symbol TEXT,
    venue_mic TEXT,
    PRIMARY KEY (scheme, value, standing)
) STRICT;
CREATE INDEX security_standings_listing ON security_standings (standing, symbol, venue_mic);
