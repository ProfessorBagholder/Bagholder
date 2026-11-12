-- A security's tax form for a year, as its issuer filed it with CDS (brief 82,
-- PR 2): read by this copy alone and kept here alone, never shared and never in
-- the repository. One row a listing and year, whatever the read found: the form
-- read (its CUSIP, its reference, each record date's return of capital and
-- non-cash distribution per unit), the form set aside (its boxes do not add to
-- what was paid), or none found. `read_at` decides when it is read again.
CREATE TABLE fund_tax_forms (
    listing TEXT NOT NULL,
    year INTEGER NOT NULL,
    outcome TEXT NOT NULL CHECK (outcome IN ('read', 'set-aside', 'none')),
    cusip TEXT NOT NULL DEFAULT '',
    form TEXT NOT NULL DEFAULT '',
    figures TEXT NOT NULL DEFAULT '[]',
    why TEXT NOT NULL DEFAULT '',
    read_at TEXT NOT NULL,
    PRIMARY KEY (listing, year)
);
