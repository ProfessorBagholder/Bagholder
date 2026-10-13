-- The record's changes, kept by the book itself (docs/plans/stage-3-the-record-by-change.md, A):
-- every write to a table the engine's ledger or its adjustments are read from appends the
-- keys whose read it changes, in the writer's own transaction, on whichever connection.
-- The server applies what is past the last sequence it applied, by key, and prunes what it
-- has applied at a start. AUTOINCREMENT: a sequence never handed out twice, even after the
-- log is pruned to empty, so nothing is ever numbered below what was already applied.
-- A succession (instrument_joins) names every record, trade, contract and adjustment it
-- re-points, as the book reads them through the join.
CREATE TABLE ledger_changes (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    kind TEXT NOT NULL CHECK (kind IN ('record', 'instrument', 'account', 'trade', 'group', 'adjustment')),
    key TEXT NOT NULL
) STRICT;
CREATE INDEX option_terms_underlying ON option_terms (underlying_id);
CREATE INDEX trades_anchor_instrument ON trades (anchor_instrument) WHERE anchor_instrument IS NOT NULL;
-- a record's links read by key from either side; an account by a broker's id for it
CREATE INDEX transfer_links_in ON transfer_links (in_record);
CREATE INDEX account_refs_value ON account_refs (value);
-- an instrument's newest sighting, by day and symbol: what it is called now
CREATE INDEX instrument_sightings_newest ON instrument_sightings (instrument_id, day, symbol);


CREATE TRIGGER accounts_changed_insert AFTER INSERT ON accounts BEGIN
    INSERT INTO ledger_changes (kind, key) SELECT 'account', k FROM (SELECT NEW.id AS k) WHERE k IS NOT NULL;
END;

CREATE TRIGGER accounts_changed_update AFTER UPDATE ON accounts BEGIN
    INSERT INTO ledger_changes (kind, key) SELECT 'account', k FROM (SELECT OLD.id AS k) WHERE k IS NOT NULL;
    INSERT INTO ledger_changes (kind, key) SELECT 'account', k FROM (SELECT NEW.id AS k) WHERE k IS NOT NULL;
END;

CREATE TRIGGER accounts_changed_delete AFTER DELETE ON accounts BEGIN
    INSERT INTO ledger_changes (kind, key) SELECT 'account', k FROM (SELECT OLD.id AS k) WHERE k IS NOT NULL;
END;

CREATE TRIGGER broker_connections_changed_insert AFTER INSERT ON broker_connections BEGIN
    INSERT INTO ledger_changes (kind, key) SELECT 'account', k FROM (SELECT id AS k FROM accounts WHERE connection_id = NEW.id) WHERE k IS NOT NULL;
END;

CREATE TRIGGER broker_connections_changed_update AFTER UPDATE ON broker_connections BEGIN
    INSERT INTO ledger_changes (kind, key) SELECT 'account', k FROM (SELECT id AS k FROM accounts WHERE connection_id = OLD.id) WHERE k IS NOT NULL;
    INSERT INTO ledger_changes (kind, key) SELECT 'account', k FROM (SELECT id AS k FROM accounts WHERE connection_id = NEW.id) WHERE k IS NOT NULL;
END;

CREATE TRIGGER broker_connections_changed_delete AFTER DELETE ON broker_connections BEGIN
    INSERT INTO ledger_changes (kind, key) SELECT 'account', k FROM (SELECT id AS k FROM accounts WHERE connection_id = OLD.id) WHERE k IS NOT NULL;
END;

CREATE TRIGGER instruments_changed_insert AFTER INSERT ON instruments BEGIN
    INSERT INTO ledger_changes (kind, key) SELECT 'instrument', k FROM (SELECT NEW.id AS k) WHERE k IS NOT NULL;
END;

CREATE TRIGGER instruments_changed_update AFTER UPDATE ON instruments BEGIN
    INSERT INTO ledger_changes (kind, key) SELECT 'instrument', k FROM (SELECT OLD.id AS k) WHERE k IS NOT NULL;
    INSERT INTO ledger_changes (kind, key) SELECT 'instrument', k FROM (SELECT NEW.id AS k) WHERE k IS NOT NULL;
END;

CREATE TRIGGER instruments_changed_delete AFTER DELETE ON instruments BEGIN
    INSERT INTO ledger_changes (kind, key) SELECT 'instrument', k FROM (SELECT OLD.id AS k) WHERE k IS NOT NULL;
END;

CREATE TRIGGER instrument_sightings_changed_insert AFTER INSERT ON instrument_sightings BEGIN
    INSERT INTO ledger_changes (kind, key) SELECT 'instrument', k FROM (SELECT NEW.instrument_id AS k) WHERE k IS NOT NULL;
END;

CREATE TRIGGER instrument_sightings_changed_update AFTER UPDATE ON instrument_sightings BEGIN
    INSERT INTO ledger_changes (kind, key) SELECT 'instrument', k FROM (SELECT OLD.instrument_id AS k) WHERE k IS NOT NULL;
    INSERT INTO ledger_changes (kind, key) SELECT 'instrument', k FROM (SELECT NEW.instrument_id AS k) WHERE k IS NOT NULL;
END;

CREATE TRIGGER instrument_sightings_changed_delete AFTER DELETE ON instrument_sightings BEGIN
    INSERT INTO ledger_changes (kind, key) SELECT 'instrument', k FROM (SELECT OLD.instrument_id AS k) WHERE k IS NOT NULL;
END;

CREATE TRIGGER instruments_described_changed_insert AFTER INSERT ON instruments_described BEGIN
    INSERT INTO ledger_changes (kind, key) SELECT 'instrument', k FROM (SELECT NEW.instrument_id AS k) WHERE k IS NOT NULL;
END;

CREATE TRIGGER instruments_described_changed_update AFTER UPDATE ON instruments_described BEGIN
    INSERT INTO ledger_changes (kind, key) SELECT 'instrument', k FROM (SELECT OLD.instrument_id AS k) WHERE k IS NOT NULL;
    INSERT INTO ledger_changes (kind, key) SELECT 'instrument', k FROM (SELECT NEW.instrument_id AS k) WHERE k IS NOT NULL;
END;

CREATE TRIGGER instruments_described_changed_delete AFTER DELETE ON instruments_described BEGIN
    INSERT INTO ledger_changes (kind, key) SELECT 'instrument', k FROM (SELECT OLD.instrument_id AS k) WHERE k IS NOT NULL;
END;

CREATE TRIGGER listings_named_changed_insert AFTER INSERT ON listings_named BEGIN
    INSERT INTO ledger_changes (kind, key) SELECT 'instrument', k FROM (SELECT NEW.instrument_id AS k) WHERE k IS NOT NULL;
END;

CREATE TRIGGER listings_named_changed_update AFTER UPDATE ON listings_named BEGIN
    INSERT INTO ledger_changes (kind, key) SELECT 'instrument', k FROM (SELECT OLD.instrument_id AS k) WHERE k IS NOT NULL;
    INSERT INTO ledger_changes (kind, key) SELECT 'instrument', k FROM (SELECT NEW.instrument_id AS k) WHERE k IS NOT NULL;
END;

CREATE TRIGGER listings_named_changed_delete AFTER DELETE ON listings_named BEGIN
    INSERT INTO ledger_changes (kind, key) SELECT 'instrument', k FROM (SELECT OLD.instrument_id AS k) WHERE k IS NOT NULL;
END;

CREATE TRIGGER option_terms_changed_insert AFTER INSERT ON option_terms BEGIN
    INSERT INTO ledger_changes (kind, key) SELECT 'instrument', k FROM (SELECT NEW.instrument_id AS k) WHERE k IS NOT NULL;
END;

CREATE TRIGGER option_terms_changed_update AFTER UPDATE ON option_terms BEGIN
    INSERT INTO ledger_changes (kind, key) SELECT 'instrument', k FROM (SELECT OLD.instrument_id AS k) WHERE k IS NOT NULL;
    INSERT INTO ledger_changes (kind, key) SELECT 'instrument', k FROM (SELECT NEW.instrument_id AS k) WHERE k IS NOT NULL;
END;

CREATE TRIGGER option_terms_changed_delete AFTER DELETE ON option_terms BEGIN
    INSERT INTO ledger_changes (kind, key) SELECT 'instrument', k FROM (SELECT OLD.instrument_id AS k) WHERE k IS NOT NULL;
END;

CREATE TRIGGER instrument_joins_changed_insert AFTER INSERT ON instrument_joins BEGIN
    INSERT INTO ledger_changes (kind, key) SELECT 'instrument', k FROM (SELECT NEW.instrument_id AS k UNION SELECT NEW.into_id UNION SELECT instrument_id FROM option_terms WHERE underlying_id IN (NEW.instrument_id, NEW.into_id)) WHERE k IS NOT NULL;
    INSERT INTO ledger_changes (kind, key) SELECT 'record', k FROM (SELECT DISTINCT record_id AS k FROM transactions WHERE instrument_id IN (NEW.instrument_id, NEW.into_id)) WHERE k IS NOT NULL;
    INSERT INTO ledger_changes (kind, key) SELECT 'trade', k FROM (SELECT id AS k FROM trades WHERE anchor_instrument IN (NEW.instrument_id, NEW.into_id)) WHERE k IS NOT NULL;
    INSERT INTO ledger_changes (kind, key) SELECT 'adjustment', k FROM (SELECT DISTINCT a.applies_record || '/' || a.applies_leg AS k FROM adjustments a JOIN adjustment_legs l ON l.record_id = a.record_id AND l.leg = a.leg WHERE l.from_instrument IN (NEW.instrument_id, NEW.into_id) OR l.to_instrument IN (NEW.instrument_id, NEW.into_id)) WHERE k IS NOT NULL;
END;

CREATE TRIGGER instrument_joins_changed_update AFTER UPDATE ON instrument_joins BEGIN
    INSERT INTO ledger_changes (kind, key) SELECT 'instrument', k FROM (SELECT OLD.instrument_id AS k UNION SELECT OLD.into_id UNION SELECT instrument_id FROM option_terms WHERE underlying_id IN (OLD.instrument_id, OLD.into_id)) WHERE k IS NOT NULL;
    INSERT INTO ledger_changes (kind, key) SELECT 'record', k FROM (SELECT DISTINCT record_id AS k FROM transactions WHERE instrument_id IN (OLD.instrument_id, OLD.into_id)) WHERE k IS NOT NULL;
    INSERT INTO ledger_changes (kind, key) SELECT 'trade', k FROM (SELECT id AS k FROM trades WHERE anchor_instrument IN (OLD.instrument_id, OLD.into_id)) WHERE k IS NOT NULL;
    INSERT INTO ledger_changes (kind, key) SELECT 'adjustment', k FROM (SELECT DISTINCT a.applies_record || '/' || a.applies_leg AS k FROM adjustments a JOIN adjustment_legs l ON l.record_id = a.record_id AND l.leg = a.leg WHERE l.from_instrument IN (OLD.instrument_id, OLD.into_id) OR l.to_instrument IN (OLD.instrument_id, OLD.into_id)) WHERE k IS NOT NULL;
    INSERT INTO ledger_changes (kind, key) SELECT 'instrument', k FROM (SELECT NEW.instrument_id AS k UNION SELECT NEW.into_id UNION SELECT instrument_id FROM option_terms WHERE underlying_id IN (NEW.instrument_id, NEW.into_id)) WHERE k IS NOT NULL;
    INSERT INTO ledger_changes (kind, key) SELECT 'record', k FROM (SELECT DISTINCT record_id AS k FROM transactions WHERE instrument_id IN (NEW.instrument_id, NEW.into_id)) WHERE k IS NOT NULL;
    INSERT INTO ledger_changes (kind, key) SELECT 'trade', k FROM (SELECT id AS k FROM trades WHERE anchor_instrument IN (NEW.instrument_id, NEW.into_id)) WHERE k IS NOT NULL;
    INSERT INTO ledger_changes (kind, key) SELECT 'adjustment', k FROM (SELECT DISTINCT a.applies_record || '/' || a.applies_leg AS k FROM adjustments a JOIN adjustment_legs l ON l.record_id = a.record_id AND l.leg = a.leg WHERE l.from_instrument IN (NEW.instrument_id, NEW.into_id) OR l.to_instrument IN (NEW.instrument_id, NEW.into_id)) WHERE k IS NOT NULL;
END;

CREATE TRIGGER instrument_joins_changed_delete AFTER DELETE ON instrument_joins BEGIN
    INSERT INTO ledger_changes (kind, key) SELECT 'instrument', k FROM (SELECT OLD.instrument_id AS k UNION SELECT OLD.into_id UNION SELECT instrument_id FROM option_terms WHERE underlying_id IN (OLD.instrument_id, OLD.into_id)) WHERE k IS NOT NULL;
    INSERT INTO ledger_changes (kind, key) SELECT 'record', k FROM (SELECT DISTINCT record_id AS k FROM transactions WHERE instrument_id IN (OLD.instrument_id, OLD.into_id)) WHERE k IS NOT NULL;
    INSERT INTO ledger_changes (kind, key) SELECT 'trade', k FROM (SELECT id AS k FROM trades WHERE anchor_instrument IN (OLD.instrument_id, OLD.into_id)) WHERE k IS NOT NULL;
    INSERT INTO ledger_changes (kind, key) SELECT 'adjustment', k FROM (SELECT DISTINCT a.applies_record || '/' || a.applies_leg AS k FROM adjustments a JOIN adjustment_legs l ON l.record_id = a.record_id AND l.leg = a.leg WHERE l.from_instrument IN (OLD.instrument_id, OLD.into_id) OR l.to_instrument IN (OLD.instrument_id, OLD.into_id)) WHERE k IS NOT NULL;
END;

CREATE TRIGGER source_records_changed_insert AFTER INSERT ON source_records BEGIN
    INSERT INTO ledger_changes (kind, key) SELECT 'record', k FROM (SELECT NEW.id AS k) WHERE k IS NOT NULL;
    INSERT INTO ledger_changes (kind, key) SELECT 'instrument', k FROM (SELECT DISTINCT instrument_id AS k FROM instrument_sightings WHERE record_id = NEW.id) WHERE k IS NOT NULL;
END;

CREATE TRIGGER source_records_changed_update AFTER UPDATE ON source_records BEGIN
    INSERT INTO ledger_changes (kind, key) SELECT 'record', k FROM (SELECT OLD.id AS k) WHERE k IS NOT NULL;
    INSERT INTO ledger_changes (kind, key) SELECT 'instrument', k FROM (SELECT DISTINCT instrument_id AS k FROM instrument_sightings WHERE record_id = OLD.id) WHERE k IS NOT NULL;
    INSERT INTO ledger_changes (kind, key) SELECT 'record', k FROM (SELECT NEW.id AS k) WHERE k IS NOT NULL;
    INSERT INTO ledger_changes (kind, key) SELECT 'instrument', k FROM (SELECT DISTINCT instrument_id AS k FROM instrument_sightings WHERE record_id = NEW.id) WHERE k IS NOT NULL;
END;

CREATE TRIGGER source_records_changed_delete AFTER DELETE ON source_records BEGIN
    INSERT INTO ledger_changes (kind, key) SELECT 'record', k FROM (SELECT OLD.id AS k) WHERE k IS NOT NULL;
    INSERT INTO ledger_changes (kind, key) SELECT 'instrument', k FROM (SELECT DISTINCT instrument_id AS k FROM instrument_sightings WHERE record_id = OLD.id) WHERE k IS NOT NULL;
END;

CREATE TRIGGER transactions_changed_insert AFTER INSERT ON transactions BEGIN
    INSERT INTO ledger_changes (kind, key) SELECT 'record', k FROM (SELECT NEW.record_id AS k) WHERE k IS NOT NULL;
END;

CREATE TRIGGER transactions_changed_update AFTER UPDATE ON transactions BEGIN
    INSERT INTO ledger_changes (kind, key) SELECT 'record', k FROM (SELECT OLD.record_id AS k) WHERE k IS NOT NULL;
    INSERT INTO ledger_changes (kind, key) SELECT 'record', k FROM (SELECT NEW.record_id AS k) WHERE k IS NOT NULL;
END;

CREATE TRIGGER transactions_changed_delete AFTER DELETE ON transactions BEGIN
    INSERT INTO ledger_changes (kind, key) SELECT 'record', k FROM (SELECT OLD.record_id AS k) WHERE k IS NOT NULL;
END;

CREATE TRIGGER record_problems_changed_insert AFTER INSERT ON record_problems BEGIN
    INSERT INTO ledger_changes (kind, key) SELECT 'record', k FROM (SELECT NEW.record_id AS k) WHERE k IS NOT NULL;
END;

CREATE TRIGGER record_problems_changed_update AFTER UPDATE ON record_problems BEGIN
    INSERT INTO ledger_changes (kind, key) SELECT 'record', k FROM (SELECT OLD.record_id AS k) WHERE k IS NOT NULL;
    INSERT INTO ledger_changes (kind, key) SELECT 'record', k FROM (SELECT NEW.record_id AS k) WHERE k IS NOT NULL;
END;

CREATE TRIGGER record_problems_changed_delete AFTER DELETE ON record_problems BEGIN
    INSERT INTO ledger_changes (kind, key) SELECT 'record', k FROM (SELECT OLD.record_id AS k) WHERE k IS NOT NULL;
END;

CREATE TRIGGER transfer_links_changed_insert AFTER INSERT ON transfer_links BEGIN
    INSERT INTO ledger_changes (kind, key) SELECT 'record', k FROM (SELECT NEW.out_record AS k UNION SELECT NEW.in_record) WHERE k IS NOT NULL;
END;

CREATE TRIGGER transfer_links_changed_update AFTER UPDATE ON transfer_links BEGIN
    INSERT INTO ledger_changes (kind, key) SELECT 'record', k FROM (SELECT OLD.out_record AS k UNION SELECT OLD.in_record) WHERE k IS NOT NULL;
    INSERT INTO ledger_changes (kind, key) SELECT 'record', k FROM (SELECT NEW.out_record AS k UNION SELECT NEW.in_record) WHERE k IS NOT NULL;
END;

CREATE TRIGGER transfer_links_changed_delete AFTER DELETE ON transfer_links BEGIN
    INSERT INTO ledger_changes (kind, key) SELECT 'record', k FROM (SELECT OLD.out_record AS k UNION SELECT OLD.in_record) WHERE k IS NOT NULL;
END;

CREATE TRIGGER trades_changed_insert AFTER INSERT ON trades BEGIN
    INSERT INTO ledger_changes (kind, key) SELECT 'trade', k FROM (SELECT NEW.id AS k) WHERE k IS NOT NULL;
END;

CREATE TRIGGER trades_changed_update AFTER UPDATE ON trades BEGIN
    INSERT INTO ledger_changes (kind, key) SELECT 'trade', k FROM (SELECT OLD.id AS k) WHERE k IS NOT NULL;
    INSERT INTO ledger_changes (kind, key) SELECT 'trade', k FROM (SELECT NEW.id AS k) WHERE k IS NOT NULL;
END;

CREATE TRIGGER trades_changed_delete AFTER DELETE ON trades BEGIN
    INSERT INTO ledger_changes (kind, key) SELECT 'trade', k FROM (SELECT OLD.id AS k) WHERE k IS NOT NULL;
END;

CREATE TRIGGER trade_groups_changed_insert AFTER INSERT ON trade_groups BEGIN
    INSERT INTO ledger_changes (kind, key) SELECT 'group', k FROM (SELECT NEW.id AS k) WHERE k IS NOT NULL;
END;

CREATE TRIGGER trade_groups_changed_update AFTER UPDATE ON trade_groups BEGIN
    INSERT INTO ledger_changes (kind, key) SELECT 'group', k FROM (SELECT OLD.id AS k) WHERE k IS NOT NULL;
    INSERT INTO ledger_changes (kind, key) SELECT 'group', k FROM (SELECT NEW.id AS k) WHERE k IS NOT NULL;
END;

CREATE TRIGGER trade_groups_changed_delete AFTER DELETE ON trade_groups BEGIN
    INSERT INTO ledger_changes (kind, key) SELECT 'group', k FROM (SELECT OLD.id AS k) WHERE k IS NOT NULL;
END;

CREATE TRIGGER trade_group_members_changed_insert AFTER INSERT ON trade_group_members BEGIN
    INSERT INTO ledger_changes (kind, key) SELECT 'group', k FROM (SELECT NEW.group_id AS k) WHERE k IS NOT NULL;
END;

CREATE TRIGGER trade_group_members_changed_update AFTER UPDATE ON trade_group_members BEGIN
    INSERT INTO ledger_changes (kind, key) SELECT 'group', k FROM (SELECT OLD.group_id AS k) WHERE k IS NOT NULL;
    INSERT INTO ledger_changes (kind, key) SELECT 'group', k FROM (SELECT NEW.group_id AS k) WHERE k IS NOT NULL;
END;

CREATE TRIGGER trade_group_members_changed_delete AFTER DELETE ON trade_group_members BEGIN
    INSERT INTO ledger_changes (kind, key) SELECT 'group', k FROM (SELECT OLD.group_id AS k) WHERE k IS NOT NULL;
END;

CREATE TRIGGER journal_changed_insert AFTER INSERT ON journal BEGIN
    INSERT INTO ledger_changes (kind, key) SELECT 'trade', k FROM (SELECT NEW.trade_id AS k) WHERE k IS NOT NULL;
    INSERT INTO ledger_changes (kind, key) SELECT 'group', k FROM (SELECT NEW.group_id AS k) WHERE k IS NOT NULL;
END;

CREATE TRIGGER journal_changed_update AFTER UPDATE ON journal BEGIN
    INSERT INTO ledger_changes (kind, key) SELECT 'trade', k FROM (SELECT OLD.trade_id AS k) WHERE k IS NOT NULL;
    INSERT INTO ledger_changes (kind, key) SELECT 'group', k FROM (SELECT OLD.group_id AS k) WHERE k IS NOT NULL;
    INSERT INTO ledger_changes (kind, key) SELECT 'trade', k FROM (SELECT NEW.trade_id AS k) WHERE k IS NOT NULL;
    INSERT INTO ledger_changes (kind, key) SELECT 'group', k FROM (SELECT NEW.group_id AS k) WHERE k IS NOT NULL;
END;

CREATE TRIGGER journal_changed_delete AFTER DELETE ON journal BEGIN
    INSERT INTO ledger_changes (kind, key) SELECT 'trade', k FROM (SELECT OLD.trade_id AS k) WHERE k IS NOT NULL;
    INSERT INTO ledger_changes (kind, key) SELECT 'group', k FROM (SELECT OLD.group_id AS k) WHERE k IS NOT NULL;
END;

CREATE TRIGGER journal_tags_changed_insert AFTER INSERT ON journal_tags BEGIN
    INSERT INTO ledger_changes (kind, key) SELECT 'trade', k FROM (SELECT trade_id AS k FROM journal WHERE id = NEW.journal_id) WHERE k IS NOT NULL;
    INSERT INTO ledger_changes (kind, key) SELECT 'group', k FROM (SELECT group_id AS k FROM journal WHERE id = NEW.journal_id) WHERE k IS NOT NULL;
END;

CREATE TRIGGER journal_tags_changed_update AFTER UPDATE ON journal_tags BEGIN
    INSERT INTO ledger_changes (kind, key) SELECT 'trade', k FROM (SELECT trade_id AS k FROM journal WHERE id = OLD.journal_id) WHERE k IS NOT NULL;
    INSERT INTO ledger_changes (kind, key) SELECT 'group', k FROM (SELECT group_id AS k FROM journal WHERE id = OLD.journal_id) WHERE k IS NOT NULL;
    INSERT INTO ledger_changes (kind, key) SELECT 'trade', k FROM (SELECT trade_id AS k FROM journal WHERE id = NEW.journal_id) WHERE k IS NOT NULL;
    INSERT INTO ledger_changes (kind, key) SELECT 'group', k FROM (SELECT group_id AS k FROM journal WHERE id = NEW.journal_id) WHERE k IS NOT NULL;
END;

CREATE TRIGGER journal_tags_changed_delete AFTER DELETE ON journal_tags BEGIN
    INSERT INTO ledger_changes (kind, key) SELECT 'trade', k FROM (SELECT trade_id AS k FROM journal WHERE id = OLD.journal_id) WHERE k IS NOT NULL;
    INSERT INTO ledger_changes (kind, key) SELECT 'group', k FROM (SELECT group_id AS k FROM journal WHERE id = OLD.journal_id) WHERE k IS NOT NULL;
END;

CREATE TRIGGER adjustments_changed_insert AFTER INSERT ON adjustments BEGIN
    INSERT INTO ledger_changes (kind, key) SELECT 'adjustment', k FROM (SELECT NEW.applies_record || '/' || NEW.applies_leg AS k) WHERE k IS NOT NULL;
END;

CREATE TRIGGER adjustments_changed_update AFTER UPDATE ON adjustments BEGIN
    INSERT INTO ledger_changes (kind, key) SELECT 'adjustment', k FROM (SELECT OLD.applies_record || '/' || OLD.applies_leg AS k) WHERE k IS NOT NULL;
    INSERT INTO ledger_changes (kind, key) SELECT 'adjustment', k FROM (SELECT NEW.applies_record || '/' || NEW.applies_leg AS k) WHERE k IS NOT NULL;
END;

CREATE TRIGGER adjustments_changed_delete AFTER DELETE ON adjustments BEGIN
    INSERT INTO ledger_changes (kind, key) SELECT 'adjustment', k FROM (SELECT OLD.applies_record || '/' || OLD.applies_leg AS k) WHERE k IS NOT NULL;
END;

CREATE TRIGGER adjustment_legs_changed_insert AFTER INSERT ON adjustment_legs BEGIN
    INSERT INTO ledger_changes (kind, key) SELECT 'adjustment', k FROM (SELECT applies_record || '/' || applies_leg AS k FROM adjustments WHERE record_id = NEW.record_id AND leg = NEW.leg) WHERE k IS NOT NULL;
END;

CREATE TRIGGER adjustment_legs_changed_update AFTER UPDATE ON adjustment_legs BEGIN
    INSERT INTO ledger_changes (kind, key) SELECT 'adjustment', k FROM (SELECT applies_record || '/' || applies_leg AS k FROM adjustments WHERE record_id = OLD.record_id AND leg = OLD.leg) WHERE k IS NOT NULL;
    INSERT INTO ledger_changes (kind, key) SELECT 'adjustment', k FROM (SELECT applies_record || '/' || applies_leg AS k FROM adjustments WHERE record_id = NEW.record_id AND leg = NEW.leg) WHERE k IS NOT NULL;
END;

CREATE TRIGGER adjustment_legs_changed_delete AFTER DELETE ON adjustment_legs BEGIN
    INSERT INTO ledger_changes (kind, key) SELECT 'adjustment', k FROM (SELECT applies_record || '/' || applies_leg AS k FROM adjustments WHERE record_id = OLD.record_id AND leg = OLD.leg) WHERE k IS NOT NULL;
END;

-- the broker's ids for an instrument or an account: what the page names them by to the broker
CREATE TRIGGER instrument_refs_changed_insert AFTER INSERT ON instrument_refs BEGIN
    INSERT INTO ledger_changes (kind, key) SELECT 'instrument', k FROM (SELECT NEW.instrument_id AS k) WHERE k IS NOT NULL;
END;

CREATE TRIGGER instrument_refs_changed_update AFTER UPDATE ON instrument_refs BEGIN
    INSERT INTO ledger_changes (kind, key) SELECT 'instrument', k FROM (SELECT OLD.instrument_id AS k) WHERE k IS NOT NULL;
    INSERT INTO ledger_changes (kind, key) SELECT 'instrument', k FROM (SELECT NEW.instrument_id AS k) WHERE k IS NOT NULL;
END;

CREATE TRIGGER instrument_refs_changed_delete AFTER DELETE ON instrument_refs BEGIN
    INSERT INTO ledger_changes (kind, key) SELECT 'instrument', k FROM (SELECT OLD.instrument_id AS k) WHERE k IS NOT NULL;
END;

CREATE TRIGGER account_refs_changed_insert AFTER INSERT ON account_refs BEGIN
    INSERT INTO ledger_changes (kind, key) SELECT 'account', k FROM (SELECT NEW.account_id AS k) WHERE k IS NOT NULL;
END;

CREATE TRIGGER account_refs_changed_update AFTER UPDATE ON account_refs BEGIN
    INSERT INTO ledger_changes (kind, key) SELECT 'account', k FROM (SELECT OLD.account_id AS k) WHERE k IS NOT NULL;
    INSERT INTO ledger_changes (kind, key) SELECT 'account', k FROM (SELECT NEW.account_id AS k) WHERE k IS NOT NULL;
END;

CREATE TRIGGER account_refs_changed_delete AFTER DELETE ON account_refs BEGIN
    INSERT INTO ledger_changes (kind, key) SELECT 'account', k FROM (SELECT OLD.account_id AS k) WHERE k IS NOT NULL;
END;

