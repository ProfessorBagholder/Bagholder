//! A fill of Bagholder's own order, booked the moment a read-back says more of it
//! has filled (`docs/plans/stage-4-execution.md`, "Fills"), so the position moves
//! within seconds, not at the next pull. Each is a record of this source keyed by the
//! order and the cumulative quantity it brings the order to, so reading the same state
//! twice books nothing new; its value is what makes the order's booked total equal
//! the broker's stated average times its cumulative quantity, exactly. When the pull
//! brings the broker's own activity row for the order, that row takes the place of
//! every such record of the order (`supersede`), and from then on the broker's row
//! is the order's fill.

use bagholder_core::account::AccountRef;
use bagholder_core::instrument::{InstrumentKind, RefScheme, Reference};
use bagholder_core::order::{OrderFold, Side};
use bagholder_core::transaction::Kind;
use bagholder_core::record::RecordState;
use bagholder_core::{Broker, Currency, Dec, DecError, Leg, Money, Rounding, SourceName};
use serde::{Deserialize, Serialize};

use crate::mapping::{Draft, InstrumentDraft, MapContext, Mapped, Mapping, NameDraft};
use crate::orders::OrderRequest;
use crate::records::Incoming;
use crate::{Book, BookError, Result};

/// The source of the fills Bagholder books from its own orders' read-backs.
pub fn source() -> SourceName {
    SourceName::named("bagholder-fill")
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Payload {
    /// The app's id for the order, and the broker's.
    order: String,
    broker_order: Option<String>,
    broker: String,
    account: String,
    security: String,
    kind: String,
    currency: String,
    symbol: String,
    at: String,
    day: String,
    side: String,
    /// The units this fill adds, and what the order had filled after it.
    quantity: String,
    filled: String,
    /// A unit's price, and the cash it moved where the contract's size is known.
    price: String,
    cash: Option<String>,
}

pub struct FillMapping;

impl Mapping for FillMapping {
    fn source(&self) -> SourceName {
        source()
    }

    fn version(&self) -> u32 {
        1
    }

    fn map(&self, _ctx: &MapContext, payload: &str) -> Mapped {
        let read = || -> std::result::Result<Draft, String> {
            let p: Payload = serde_json::from_str(payload).map_err(|e| e.to_string())?;
            let broker = Broker::parse(&p.broker).map_err(|e| e.to_string())?;
            let currency = Currency::parse(&p.currency).map_err(|e| e.to_string())?;
            let day: jiff::civil::Date = p.day.parse().map_err(|e: jiff::Error| e.to_string())?;
            let q = Dec::parse(&p.quantity).map_err(|e| e.to_string())?;
            let (kind, q) = match p.side.as_str() {
                "buy" => (Kind::Buy, q),
                "sell" => (Kind::Sell, q.neg()),
                other => return Err(format!("a side {other:?}")),
            };
            Ok(Draft {
                leg: Leg::named("fill"),
                account: AccountRef::new(broker.clone(), p.account.clone()),
                occurred_at: Some(p.at.parse().map_err(|e: jiff::Error| e.to_string())?),
                trade_date: day,
                settle_date: None,
                kind,
                effect: None,
                instrument: Some(InstrumentDraft {
                    refs: vec![Reference::new(RefScheme::BrokerSecurity(broker), p.security.clone())],
                    kind: InstrumentKind::parse(&p.kind).map_err(|e| e.to_string())?,
                    currency,
                    name: Some(NameDraft { symbol: p.symbol.clone(), venue_mic: None, venue_name: None, name: None, seen: day }),
                    option: None,
                }),
                quantity: Some(q),
                price: Some(Money::new(Dec::parse(&p.price).map_err(|e| e.to_string())?, currency)),
                cash: p.cash.as_deref().map(|c| Dec::parse(c).map(|c| Money::new(c, currency))).transpose().map_err(|e| e.to_string())?,
                fee: None,
                fx_rate: None,
                paid_on: None,
                value: None,
            })
        };
        match read() {
            Ok(d) => Mapped { legs: vec![d], problems: vec![], adjustments: vec![] },
            Err(why) => Mapped::unreadable(format!("a fill that does not read: {why}")),
        }
    }
}

impl Book {
    /// The broker's own activity row for an order, by the broker's id for it, in any
    /// connection.
    fn broker_row(&self, broker: &str, broker_order: &str) -> Result<Option<bagholder_core::RecordId>> {
        let found: Option<String> = rusqlite::OptionalExtension::optional(self.conn().query_row(
            "SELECT id FROM source_records WHERE source = ? AND source_key = ?",
            rusqlite::params![broker, broker_order],
            |r| r.get(0),
        ))?;
        found.map(|s| crate::text::parsed("source_records", "id", &s, bagholder_core::RecordId::parse)).transpose()
    }

    /// Book what an order newly filled, `before` being what it had filled. Nothing
    /// when the broker's own row for the order is in the book already: it is the fill.
    pub(crate) fn book_fill(&self, o: &OrderRequest, before: &OrderFold, after: &OrderFold, at: jiff::Timestamp) -> Result<()> {
        let d = |e: DecError| BookError::Refused(format!("order {}: {e}", o.id));
        // what is booked already: the earlier readings, each booked when it had an average
        let (booked, booked_total) = match before.average {
            Some(a) => (before.filled, before.filled.checked_mul(a).map_err(d)?),
            None => (Dec::ZERO, Dec::ZERO),
        };
        let more = after.filled.checked_sub(booked).map_err(d)?;
        if !more.is_positive() {
            return Ok(());
        }
        if let Some(b) = &after.broker_id {
            if self.broker_row(&o.broker, b)?.is_some() {
                return Ok(());
            }
        }
        // a reading with no price to book at: the next reading that states one books it all,
        // and the broker's own row takes its place either way
        let Some(average) = after.average else { return Ok(()) };
        // what the instrument is to the book, and its size where that is known
        let broker = Broker::parse(&o.broker).map_err(|e| BookError::Refused(e.to_string()))?;
        let r = Reference::new(RefScheme::BrokerSecurity(broker), o.broker_security.clone());
        let (kind, size) = match self.instrument_by_ref(&r)? {
            Some(i) => {
                let held = self.instrument(i)?;
                let size = match held.kind {
                    InstrumentKind::OptionContract => self.option_terms(i)?.and_then(|t| t.multiplier),
                    _ => Some(Dec::ONE),
                };
                (held.kind, size)
            }
            // a listing the book has not met: the ticket trades shares of one only
            None => (InstrumentKind::Security, Some(Dec::ONE)),
        };
        // the booked total follows the broker's average exactly: this fill is the difference
        let per_share = after.filled.checked_mul(average).map_err(d)?.checked_sub(booked_total).map_err(d)?;
        let price = per_share.div_rounded(more, 12, Rounding::HalfEven).map_err(d)?;
        let cash = match size {
            Some(s) => {
                let v = per_share.checked_mul(s).map_err(d)?;
                Some(if o.side == Side::Buy { v.neg() } else { v })
            }
            None => None,
        };
        let day = match self.zone()? {
            Some(z) => at.to_zoned(z.zone).date(),
            None => at.to_zoned(jiff::tz::TimeZone::UTC).date(),
        };
        let p = Payload {
            order: o.id.clone(),
            broker_order: after.broker_id.clone(),
            broker: o.broker.clone(),
            account: o.broker_account.clone(),
            security: o.broker_security.clone(),
            kind: kind.as_str().to_string(),
            currency: o.currency.as_str().to_string(),
            symbol: o.symbol.clone(),
            at: crate::text::at(at),
            day: day.to_string(),
            side: o.side.as_str().to_string(),
            quantity: more.to_text(),
            filled: after.filled.to_text(),
            price: price.to_text(),
            cash: cash.map(Dec::to_text),
        };
        let payload = serde_json::to_string(&p).map_err(|e| BookError::Refused(e.to_string()))?;
        let key = format!("{}:{}", o.id, after.filled.to_text());
        let mut refs = vec![("order".to_string(), o.id.clone())];
        if let Some(b) = &after.broker_id {
            refs.push(("broker-order".to_string(), b.clone()));
        }
        self.store(&FillMapping, &Incoming { connection: None, source_key: &key, payload: &payload, refs }, at)?;
        Ok(())
    }

    /// Let each broker row that arrived for one of Bagholder's own orders take the place
    /// of the fills booked for that order: the number of orders whose fills gave way.
    pub fn fills_give_way(&self, at: jiff::Timestamp) -> Result<usize> {
        let mut by_order: std::collections::BTreeMap<(String, String), Vec<bagholder_core::RecordId>> = std::collections::BTreeMap::new();
        for id in self.live_records(&source())? {
            let Some(b) = self.record_ref(id, "broker-order")? else { continue };
            let payload = self.revisions(id)?.pop().map(|(_, _, p)| p).unwrap_or_default();
            let p: Payload = serde_json::from_str(&payload).map_err(|e| crate::text::corrupt("source_records", "payload", &payload, e))?;
            by_order.entry((p.broker, b)).or_default().push(id);
        }
        let mut n = 0;
        for ((broker, b), fills) in by_order {
            let Some(row) = self.broker_row(&broker, &b)? else { continue };
            if self.record(row)?.state != RecordState::Live {
                continue;
            }
            self.supersede(&fills, &[row], "the broker's own row for the order's fill", at)?;
            n += 1;
        }
        Ok(n)
    }
}
