-- What the cache kept only for the earlier app (brief 75 §3): each ticker's
-- filers, read once to carry the book's told marks, and the mark of the carry.
DROP TABLE filings_carried;
DELETE FROM meta WHERE key = 'carried.earlier-store';
