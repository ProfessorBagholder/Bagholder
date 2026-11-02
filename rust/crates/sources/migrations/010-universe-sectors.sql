-- Each S&P/TSX 60 constituent's sector as TMX last said it, and when
-- (`docs/plans/heatmaps-always-ready.md`): the day's quotes come for every
-- constituent in one request that carries no sector, so a sector is asked singly
-- only where none is kept here younger than a month. Read by the Canada read
-- alone, no figure: no generation counter.
CREATE TABLE universe_sectors (
    symbol TEXT PRIMARY KEY,
    sector TEXT NOT NULL,
    read_at TEXT NOT NULL
);
-- the sectors the last reads already stated, as of those reads
INSERT INTO universe_sectors (symbol, sector, read_at)
    SELECT symbol, sector, fetched_at FROM universes
    WHERE key = 'ca' AND sector IS NOT NULL AND sector <> '' AND sector <> 'Not classified' AND fetched_at IS NOT NULL;
