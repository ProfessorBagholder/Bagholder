-- One book account per set of broker ids the broker links as one (`linked`):
-- migration 37 split every account holding several ids, and so split an account
-- the broker gave a new id from its old one. The schema does not change; the
-- code step of this migration (`linked::step`) merges each linked set back onto
-- its oldest account, the split's inverse, and the next sync reads it whole.
SELECT 1;
