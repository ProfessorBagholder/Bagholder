-- What the person's own page states, kept for when no page is open
-- (docs/plans/stage-3c-switch.md, §2, "The zone"): the time zone of the browser
-- in use, the latest one stated. Each setting is one row, with who stated it and
-- when.
CREATE TABLE settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    source TEXT NOT NULL,
    set_at TEXT NOT NULL
) STRICT;
