-- Each tax slip the broker issued the person (brief 82, PR 1): the text of its file
-- as `pdftotext -layout` lays its pages out, kept whole as it came, read once and
-- kept, keyed by the broker's own id for the document. A slip the broker amends is a
-- document of its own: for an account, a kind and a year, the one made last is the
-- slip (`SPEC.md` tax-slips-3). What a slip states is read from its text each time
-- it is used, so a reader put right reads every kept slip again without asking the
-- broker; `created` is the broker's own date or instant for it, as it wrote it.
CREATE TABLE tax_slips (
    connection_id TEXT NOT NULL REFERENCES broker_connections(id),
    document TEXT NOT NULL,
    account_key TEXT NOT NULL,
    kind TEXT NOT NULL,
    year INTEGER NOT NULL,
    created TEXT NOT NULL,
    text TEXT NOT NULL,
    read_id TEXT NOT NULL REFERENCES broker_reads(id),
    PRIMARY KEY (connection_id, document)
) STRICT;
