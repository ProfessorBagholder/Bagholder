-- Reference for brief 75 §2: the book's migration 37, modelled on Clear data's
-- order of deletes (rust/crates/book/src/clear.rs:152-233). Children before
-- source_records; the runner checks every reference before it commits.
CREATE TEMP TABLE gone AS SELECT id FROM source_records WHERE source = 'bagholder-import';

-- a trade anchored on one is orphaned with its note kept, as Clear data does
UPDATE trades SET anchor_record = NULL, anchor_leg = NULL, anchor_instrument = NULL,
       orphaned_reason = 'the record it opened on was removed'
 WHERE anchor_record IN (SELECT id FROM temp.gone);

-- an adjustment of another record that explained one of these transactions
INSERT INTO record_problems (record_id, code, detail)
SELECT a.record_id, 'adjustment-target-gone',
       'the transaction it explains, ' || a.applies_record || '/' || a.applies_leg || ', was removed'
  FROM adjustments a
 WHERE a.applies_record IN (SELECT id FROM temp.gone)
   AND a.record_id NOT IN (SELECT id FROM temp.gone);

DELETE FROM transfer_links WHERE out_record IN (SELECT id FROM temp.gone) OR in_record IN (SELECT id FROM temp.gone);
DELETE FROM link_records WHERE record_id IN (SELECT id FROM temp.gone);
-- a link left with nothing on either side supersedes nothing; one a CSV record
-- still shares a side of stays (links.rs:140-157)
DELETE FROM link_records WHERE link_id IN (
  SELECT l.id FROM links l
   WHERE NOT EXISTS (SELECT 1 FROM link_records f WHERE f.link_id = l.id AND f.side = 'from')
      OR NOT EXISTS (SELECT 1 FROM link_records t WHERE t.link_id = l.id AND t.side = 'to'));
DELETE FROM links WHERE id NOT IN (SELECT link_id FROM link_records);
DELETE FROM instrument_sightings WHERE record_id IN (SELECT id FROM temp.gone);
DELETE FROM adjustment_legs WHERE record_id IN (SELECT id FROM temp.gone);
DELETE FROM adjustments WHERE record_id IN (SELECT id FROM temp.gone);
DELETE FROM record_problems WHERE record_id IN (SELECT id FROM temp.gone);
DELETE FROM record_refs WHERE record_id IN (SELECT id FROM temp.gone);
DELETE FROM record_revisions WHERE record_id IN (SELECT id FROM temp.gone);
DELETE FROM transactions WHERE record_id IN (SELECT id FROM temp.gone);
DELETE FROM record_holds WHERE record_id IN (SELECT id FROM temp.gone);
DELETE FROM record_orders WHERE record_id IN (SELECT id FROM temp.gone);
DELETE FROM statement_holds WHERE record_id IN (SELECT id FROM temp.gone);
DELETE FROM source_records WHERE id IN (SELECT id FROM temp.gone);
DROP TABLE temp.gone;

-- an orphaned trade that carries no note and sits in no group holds nothing
DELETE FROM trades WHERE anchor_record IS NULL
   AND id NOT IN (SELECT trade_id FROM journal WHERE trade_id IS NOT NULL)
   AND id NOT IN (SELECT trade_id FROM trade_group_members);

-- the columns that existed only for the earlier app
DROP INDEX trades_legacy_key;
ALTER TABLE trades DROP COLUMN legacy_key;
DROP INDEX trade_groups_legacy_key;
ALTER TABLE trade_groups DROP COLUMN legacy_key;

-- the reference the pull wrote on the broker's own records only so an import
-- row could be found and replaced (pull.rs:698); nothing reads it afterwards
DELETE FROM record_refs WHERE scheme = 'broker-record:wealthsimple';

-- the marks of the one-time carries and repairs
DELETE FROM settings WHERE key IN ('carried.notices', 'following.carried', 'carried.told-by-listing', 'repair.carried-notes-reattached');

-- every account is read whole at the next sync, and the header does not restart
-- on what the last pull's statements said of accounts as they were
DELETE FROM activity_reads;
DELETE FROM settings WHERE key = 'statements.said';

-- then the code step of this migration (brief 75 §4): every book account holding
-- more than one broker id is split to one account per id, with the mappings in
-- hand, inside this same transaction
