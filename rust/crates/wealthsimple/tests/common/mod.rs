//! The recorded replies (`tests/replies/wealthsimple/`, or a capture elsewhere),
//! answered through the replay adapter (`bagholder_wealthsimple::replay`), and a
//! row mapped as the book maps its record.

#![allow(dead_code)]

use std::path::PathBuf;

use bagholder_book::mapping::{MapContext, Mapped, Mapping};
use bagholder_book::zones::Zones;
use bagholder_broker::{BookMoves, BrokerAdapter, Moved, MovedWhat};
use bagholder_core::instrument::RefScheme;
use bagholder_core::json::Value;
use bagholder_core::RecordId;
use bagholder_sources::reply::Node;
use bagholder_wealthsimple::mapping::{WealthsimpleMapping, ZONE};
use bagholder_wealthsimple::adapter::{row_of, Wealthsimple};
use bagholder_wealthsimple::replay::Replay;

pub fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/replies/wealthsimple")
}

pub type Ws = Wealthsimple<Replay>;

pub fn replay(dir: &std::path::Path) -> Ws {
    Wealthsimple::new(Replay::read(dir).unwrap())
}

/// The day Wealthsimple files a row under.
pub fn day_of(row: &Value) -> jiff::civil::Date {
    let at: jiff::Timestamp = Node::root(row).text("occurredAt").unwrap().parse().unwrap();
    Zones::default().day(at, ZONE).unwrap()
}

/// What the rows mapped so far moved, as the book would hold it.
#[derive(Default)]
pub struct Moves(pub Vec<Moved>);

impl BookMoves for Moves {
    fn moves(&mut self, accounts: &[String], days: &[jiff::civil::Date]) -> Vec<Moved> {
        self.0.iter().filter(|m| accounts.contains(&m.account) && days.contains(&m.day)).cloned().collect()
    }
}

/// A payload mapped, as the book maps a record.
pub fn map_payload(payload: &Value) -> Mapped {
    let zones = Zones::default();
    let ctx = MapContext { connection: None, record: RecordId::parse("01923e6a-7b1c-7def-8123-456789abcdef").unwrap(), zones: &zones };
    WealthsimpleMapping.map(&ctx, &payload.canonical())
}

/// A row mapped, with the book's moves given.
pub fn map_row_with(r: &mut Ws, row: &Value, moves: &mut Moves) -> Mapped {
    let row = row_of(row, day_of(row)).unwrap();
    let payload = r.record(&row, moves).unwrap();
    map_payload(&payload)
}

pub fn map_row(r: &mut Ws, row: &Value) -> Mapped {
    map_row_with(r, row, &mut Moves::default())
}

/// Every row mapped as a pull stores them: first the rows that move by
/// themselves, their moves kept as the book's, then the rows read against
/// positions, net of those moves.
pub fn map_all(r: &mut Ws) -> Vec<(Value, Mapped)> {
    let rows = r.source.rows.clone();
    // the rows read as a pull reads them, for a move's siblings
    let accounts: std::collections::BTreeSet<String> = rows.iter().map(|x| Node::root(x).text("accountId").unwrap().to_string()).collect();
    for a in accounts {
        r.activity(&a, None).unwrap();
    }
    let mut moves = Moves::default();
    let mut out = Vec::new();
    let reads = |row: &Value| row_of(row, day_of(row)).unwrap().reads_positions;
    for row in rows.iter().filter(|x| !reads(x)) {
        let m = map_row(r, row);
        let account = Node::root(row).text("accountId").unwrap().to_string();
        for d in &m.legs {
            if let (Some(i), Some(q)) = (&d.instrument, d.quantity) {
                if let Some(rf) = i.refs.iter().find(|x| matches!(x.scheme, RefScheme::BrokerSecurity(_))) {
                    moves.0.push(Moved { account: account.clone(), day: d.trade_date, what: MovedWhat::Instrument(rf.clone()), quantity: q });
                }
            }
            if let Some(c) = d.cash {
                moves.0.push(Moved { account: account.clone(), day: d.trade_date, what: MovedWhat::Cash(c.currency), quantity: c.amount });
            }
        }
        out.push((row.clone(), m));
    }
    for row in rows.iter().filter(|x| reads(x)) {
        let m = map_row_with(r, row, &mut moves);
        out.push((row.clone(), m));
    }
    out
}
