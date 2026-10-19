-- A document its regulator no longer lists (brief 69): SEDAR+ answered the
-- issuer's list, scoped to the day it was filed, without it. A fact, kept with
-- when it was found, and shown on its row; never a failure that stands.
ALTER TABLE filings ADD COLUMN unlisted_at TEXT;
DROP TRIGGER gen_filings_upd;
CREATE TRIGGER gen_filings_upd AFTER UPDATE ON filings WHEN OLD."source" IS NOT NEW."source" OR OLD."id" IS NOT NEW."id" OR OLD."filer" IS NOT NEW."filer" OR OLD."category" IS NOT NEW."category" OR OLD."issuer" IS NOT NEW."issuer" OR OLD."type" IS NOT NEW."type" OR OLD."title" IS NOT NEW."title" OR OLD."date" IS NOT NEW."date" OR OLD."date_text" IS NOT NEW."date_text" OR OLD."size" IS NOT NEW."size" OR OLD."url" IS NOT NEW."url" OR OLD."subject" IS NOT NEW."subject" OR OLD."summary" IS NOT NEW."summary" OR OLD."enrich_version" IS NOT NEW."enrich_version" OR OLD."enrich_final" IS NOT NEW."enrich_final" OR OLD."unlisted_at" IS NOT NEW."unlisted_at" BEGIN UPDATE gen SET n = n + 1 WHERE name = 'filings'; END;
