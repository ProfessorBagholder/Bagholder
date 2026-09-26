-- No option contract's close is recorded any more (owner, 2026-09-24;
-- docs/plans/stage-3a-brief-06.md): the equity series is the broker's stated
-- value, and a held contract's price is read from its chain when it is shown.
DROP TABLE recorded_closes;
