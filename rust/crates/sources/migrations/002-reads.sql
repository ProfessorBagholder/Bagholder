-- Each read's span, outcome and time, per subject (an instrument's id, a
-- benchmark's key, a currency) and kind of data: what the due rules read
-- (docs/plans/stage-3a-sources.md, "Periodic reads"). The outcomes log is
-- trimmed and says how a source is; this says what has been asked. `first` and
-- `last` are the days the read settled: for an answer, from the first day asked
-- to the last day it held (a day after that is not known yet); for a source that
-- does not carry the subject, the span asked; for a failure, the span asked,
-- which waits out its source's rest. The newest read of each span is kept.
CREATE TABLE reads (
    subject TEXT NOT NULL,
    kind TEXT NOT NULL,
    source TEXT NOT NULL,
    first TEXT NOT NULL,
    last TEXT NOT NULL,
    outcome TEXT NOT NULL,
    at TEXT NOT NULL,
    PRIMARY KEY (subject, kind, source, first, last)
) STRICT;
