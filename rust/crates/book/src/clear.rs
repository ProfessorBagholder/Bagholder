//! Clear data (`docs/plans/stage-3c-switch.md` §7): the book's part of emptying
//! what the person ticks. A kind of data is a set of rows, not of tables: the
//! broker's records and the person's own share the record tables, told apart by
//! their source. What only a cleared row named (an account, an instrument, a
//! connection) goes with it; a trade whose record is cleared while its journal
//! stays is kept, orphaned, with its notes, for the person to re-attach.

use rusqlite::params;

use crate::{Book, Result};

/// The book's kinds of data the person can clear.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Clearing {
    /// The broker's records, statements and the pull's own state.
    pub broker: bool,
    /// What the person entered: Add trade, opening balances, event values, imported files.
    pub entries: bool,
    /// Notes, grades, tags and saved groups, with the trades they are kept on.
    pub journal: bool,
    /// The public facts the figures read: rates, holidays, declared distributions.
    pub market: bool,
}

impl Clearing {
    pub fn all() -> Clearing {
        Clearing { broker: true, entries: true, journal: true, market: true }
    }
}

/// What a table of the book holds, for Clear data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Holds {
    /// Records and what is derived from them: the broker's or the person's, by source.
    Records,
    Broker,
    Journal,
    Market,
    /// What records and statements name: kept while anything left names it.
    Named,
    /// The person's settings: the watched folder's go with the entries; the zone
    /// the page states is how it is shown, not data, and stays.
    Settings,
    /// The file's own version.
    Schema,
}

/// Every table of the book and what it holds. A table missing here fails a test.
pub const TABLES: &[(&str, Holds)] = &[
    ("source_records", Holds::Records),
    ("record_revisions", Holds::Records),
    ("record_problems", Holds::Records),
    ("record_refs", Holds::Records),
    ("transactions", Holds::Records),
    ("instrument_sightings", Holds::Records),
    ("adjustments", Holds::Records),
    ("adjustment_legs", Holds::Records),
    ("links", Holds::Records),
    ("link_records", Holds::Records),
    ("transfer_links", Holds::Records),
    ("broker_reads", Holds::Broker),
    ("activity_reads", Holds::Broker),
    ("account_days", Holds::Broker),
    ("account_links", Holds::Broker),
    ("buying_power", Holds::Broker),
    ("statements", Holds::Broker),
    ("statement_cash", Holds::Broker),
    ("statement_units", Holds::Broker),
    ("journal", Holds::Journal),
    ("journal_tags", Holds::Journal),
    ("trade_groups", Holds::Journal),
    ("trade_group_members", Holds::Journal),
    ("trades", Holds::Journal),
    ("fx_rates", Holds::Market),
    ("fx_reads", Holds::Market),
    ("fx_series", Holds::Market),
    ("bank_holidays", Holds::Market),
    ("declared_reads", Holds::Market),
    ("declared_distributions", Holds::Market),
    ("stated_frequencies", Holds::Market),
    ("broker_connections", Holds::Named),
    ("accounts", Holds::Named),
    ("account_refs", Holds::Named),
    ("instruments", Holds::Named),
    ("instrument_refs", Holds::Named),
    ("instrument_routes", Holds::Named),
    ("option_terms", Holds::Named),
    ("issuers", Holds::Named),
    ("settings", Holds::Settings),
    ("schema_migrations", Holds::Schema),
];

/// The sources whose records are the person's own; every other source's are a broker's.
const PERSON_SOURCES: &str = "('person', 'csv')";

impl Book {
    /// The tables the book's file holds, by name.
    pub fn tables(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn().prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name")?;
        let names = stmt.query_map([], |r| r.get::<_, String>(0))?;
        Ok(names.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Empty what `what` ticks, in one transaction.
    pub fn clear(&self, what: &Clearing) -> Result<()> {
        let c = self.conn();
        self.atomically(|| {
            let chosen = match (what.broker, what.entries) {
                (true, true) => Some("1 = 1".to_string()),
                (true, false) => Some(format!("source NOT IN {PERSON_SOURCES}")),
                (false, true) => Some(format!("source IN {PERSON_SOURCES}")),
                (false, false) => None,
            };
            if let Some(which) = chosen {
                c.execute_batch(&format!("DROP TABLE IF EXISTS temp.cleared; CREATE TEMP TABLE cleared AS SELECT id FROM source_records WHERE {which};"))?;
                // a trade kept on a cleared record is orphaned with its journal
                c.execute(
                    "UPDATE trades SET anchor_record = NULL, anchor_leg = NULL, anchor_instrument = NULL, orphaned_reason = ?
                     WHERE anchor_record IN (SELECT id FROM temp.cleared)",
                    params!["the record it opened on was cleared"],
                )?;
                // an adjustment kept on a cleared record's transaction says it is gone
                let cleared: Vec<String> = c.prepare("SELECT id FROM temp.cleared")?.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
                for id in &cleared {
                    let record = crate::text::parsed("source_records", "id", id, bagholder_core::RecordId::parse)?;
                    for (adjustment, applies) in self.adjustments_applying_to(record)? {
                        if !cleared.contains(&adjustment.record.to_string()) {
                            self.adjustment_target_gone(&adjustment, &applies, "was cleared")?;
                        }
                    }
                }
                c.execute_batch(
                    "DELETE FROM transfer_links WHERE out_record IN (SELECT id FROM temp.cleared) OR in_record IN (SELECT id FROM temp.cleared);
                     CREATE TEMP TABLE cleared_links AS SELECT DISTINCT link_id FROM link_records WHERE record_id IN (SELECT id FROM temp.cleared);
                     DELETE FROM link_records WHERE link_id IN (SELECT link_id FROM temp.cleared_links);
                     DELETE FROM links WHERE id IN (SELECT link_id FROM temp.cleared_links);
                     DROP TABLE temp.cleared_links;
                     DELETE FROM instrument_sightings WHERE record_id IN (SELECT id FROM temp.cleared);
                     DELETE FROM adjustment_legs WHERE record_id IN (SELECT id FROM temp.cleared);
                     DELETE FROM adjustments WHERE record_id IN (SELECT id FROM temp.cleared);
                     DELETE FROM record_problems WHERE record_id IN (SELECT id FROM temp.cleared);
                     DELETE FROM record_refs WHERE record_id IN (SELECT id FROM temp.cleared);
                     DELETE FROM record_revisions WHERE record_id IN (SELECT id FROM temp.cleared);
                     DELETE FROM transactions WHERE record_id IN (SELECT id FROM temp.cleared);
                     DELETE FROM source_records WHERE id IN (SELECT id FROM temp.cleared);
                     DROP TABLE temp.cleared;",
                )?;
            }
            if what.broker {
                c.execute_batch(
                    "DELETE FROM statement_units; DELETE FROM statement_cash; DELETE FROM statements;
                     DELETE FROM buying_power; DELETE FROM account_days; DELETE FROM account_links;
                     DELETE FROM activity_reads; DELETE FROM broker_reads;",
                )?;
            }
            if what.entries {
                c.execute("DELETE FROM settings WHERE key LIKE 'watch.%'", [])?;
            }
            if what.journal {
                c.execute_batch("DELETE FROM journal_tags; DELETE FROM journal; DELETE FROM trade_group_members; DELETE FROM trade_groups; DELETE FROM trades;")?;
            }
            if what.market {
                c.execute_batch(
                    "DELETE FROM declared_distributions; DELETE FROM declared_reads; DELETE FROM stated_frequencies;
                     DELETE FROM fx_rates; DELETE FROM fx_reads; DELETE FROM fx_series; DELETE FROM bank_holidays;",
                )?;
            }
            self.clear_unnamed()

        })
    }

    /// Remove every account, instrument, issuer and connection nothing left names.
    fn clear_unnamed(&self) -> Result<()> {
        let c = self.conn();
        // an instrument is named by a transaction, an adjustment, a trade, a statement,
        // a public fact, a sighting, or a contract on it that is itself named: a
        // contract goes first, and then what it was on, if nothing else names that
        loop {
            c.execute_batch(
                "DROP TABLE IF EXISTS temp.unnamed;
                 CREATE TEMP TABLE unnamed AS SELECT id FROM instruments WHERE
                    id NOT IN (SELECT instrument_id FROM transactions WHERE instrument_id IS NOT NULL)
                    AND id NOT IN (SELECT from_instrument FROM adjustment_legs WHERE from_instrument IS NOT NULL)
                    AND id NOT IN (SELECT to_instrument FROM adjustment_legs WHERE to_instrument IS NOT NULL)
                    AND id NOT IN (SELECT anchor_instrument FROM trades WHERE anchor_instrument IS NOT NULL)
                    AND id NOT IN (SELECT instrument_id FROM statement_units)
                    AND id NOT IN (SELECT instrument_id FROM declared_reads)
                    AND id NOT IN (SELECT instrument_id FROM stated_frequencies)
                    AND id NOT IN (SELECT instrument_id FROM instrument_sightings)
                    AND id NOT IN (SELECT underlying_id FROM option_terms);",
            )?;
            let n: i64 = c.query_row("SELECT COUNT(*) FROM temp.unnamed", [], |r| r.get(0))?;
            if n == 0 {
                break;
            }
            c.execute_batch(
                "DELETE FROM option_terms WHERE instrument_id IN (SELECT id FROM temp.unnamed);
                 DELETE FROM instrument_refs WHERE instrument_id IN (SELECT id FROM temp.unnamed);
                 DELETE FROM instrument_routes WHERE instrument_id IN (SELECT id FROM temp.unnamed);
                 DELETE FROM instruments WHERE id IN (SELECT id FROM temp.unnamed);",
            )?;
        }
        c.execute_batch(
            "DROP TABLE IF EXISTS temp.unnamed;
             DELETE FROM issuers WHERE id NOT IN (SELECT issuer_id FROM instruments WHERE issuer_id IS NOT NULL);
             CREATE TEMP TABLE unnamed_accounts AS SELECT id FROM accounts WHERE
                id NOT IN (SELECT account_id FROM transactions)
                AND id NOT IN (SELECT account_id FROM statements)
                AND id NOT IN (SELECT account_id FROM buying_power)
                AND id NOT IN (SELECT account_id FROM account_days)
                AND id NOT IN (SELECT account_id FROM account_links)
                AND id NOT IN (SELECT linked_to FROM account_links WHERE linked_to IS NOT NULL)
                AND id NOT IN (SELECT account_id FROM activity_reads);
             DELETE FROM account_refs WHERE account_id IN (SELECT id FROM temp.unnamed_accounts);
             DELETE FROM accounts WHERE id IN (SELECT id FROM temp.unnamed_accounts);
             DROP TABLE temp.unnamed_accounts;
             DELETE FROM broker_connections WHERE
                id NOT IN (SELECT connection_id FROM accounts)
                AND id NOT IN (SELECT connection_id FROM source_records WHERE connection_id IS NOT NULL)
                AND id NOT IN (SELECT connection_id FROM broker_reads);",
        )?;
        Ok(())
    }
}
