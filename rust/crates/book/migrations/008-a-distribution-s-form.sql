-- Whether a declared distribution's source states how it is paid
-- (docs/plans/stage-3c-switch.md, §3a): an exchange's record states an amount
-- per unit and not whether it is paid in cash or in units, and its rows are
-- kept as that (`unstated`), the form found from the record. Every row stored
-- before is its source's statement as the reader read it (`stated`).
ALTER TABLE declared_distributions ADD COLUMN form TEXT NOT NULL DEFAULT 'stated' CHECK (form IN ('stated', 'unstated'));
