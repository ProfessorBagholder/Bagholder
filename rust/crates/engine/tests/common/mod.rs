//! What the engine's tests share: a case (a small book written as JSON, with
//! short labels for ids) turned into the engine's inputs, and the engine built on
//! it with a trade for every round trip that needs one.

#![allow(dead_code)]

use std::collections::BTreeMap;

use serde_json::{Map, Value};

use bagholder_core::account::{Account, AccountKind, AccountStatus, AccountType, Registration};
use bagholder_core::adjustment::{Adjustment, AdjustmentLeg};
use bagholder_core::instrument::{Instrument, InstrumentKind, Name, OptionRight, OptionTerms};
use bagholder_core::jiff::civil::Date;
use bagholder_core::jiff::tz::TimeZone;
use bagholder_core::jiff::Timestamp;
use bagholder_core::journal::{Anchor, Grade, Group, JournalEntry, JournalSubject, Opening, Trade};
use bagholder_core::record::Problem;
use bagholder_core::transaction::{Effect, Kind, Transaction};
use bagholder_core::{AccountId, Broker, ConnectionId, Currency, Dec, GroupId, InstrumentId, Leg, MappingVersion, Money, RecordId, SourceName, TradeId, TransactionId};
use bagholder_engine::input::{Read, AccountInfo, BrokerAccount, Clock, Declared, DeclaredRead, DistributionKind, Facts, Inputs, InstrumentInfo, Ledger, Market, Quote, QuoteSource, Rates, RecordInfo, Sourced};
use bagholder_engine::{Change, Engine};

/// Labels to ids: the same label is the same id throughout a case.
#[derive(Default)]
pub struct Ids {
    pub n: u64,
    pub by_label: BTreeMap<String, String>,
}

impl Ids {
    pub fn uuid(&mut self, label: &str) -> String {
        if let Some(u) = self.by_label.get(label) {
            return u.clone();
        }
        self.n += 1;
        let u = format!("01900000-0000-7000-8000-{:012x}", self.n);
        self.by_label.insert(label.to_string(), u.clone());
        u
    }
    pub fn account(&mut self, l: &str) -> AccountId {
        AccountId::parse(&self.uuid(&format!("account:{l}"))).unwrap()
    }
    pub fn instrument(&mut self, l: &str) -> InstrumentId {
        InstrumentId::parse(&self.uuid(&format!("instrument:{l}"))).unwrap()
    }
    pub fn record(&mut self, l: &str) -> RecordId {
        RecordId::parse(&self.uuid(&format!("record:{l}"))).unwrap()
    }
    pub fn trade(&mut self, l: &str) -> TradeId {
        TradeId::parse(&self.uuid(&format!("trade:{l}"))).unwrap()
    }
    pub fn group(&mut self, l: &str) -> GroupId {
        GroupId::parse(&self.uuid(&format!("group:{l}"))).unwrap()
    }
}

pub fn s<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    v.get(k).and_then(Value::as_str)
}

pub fn dec(v: &str) -> Dec {
    Dec::parse(v).unwrap_or_else(|e| panic!("{v:?}: {e}"))
}

pub fn day(v: &str) -> Date {
    v.parse().unwrap_or_else(|e| panic!("{v:?}: {e}"))
}

pub fn at(v: &str) -> Timestamp {
    v.parse().unwrap_or_else(|e| panic!("{v:?}: {e}"))
}

pub fn ccy(v: &str) -> Currency {
    Currency::parse(v).unwrap()
}

pub fn obj(v: &Value, k: &str) -> Map<String, Value> {
    v.get(k).and_then(Value::as_object).cloned().unwrap_or_default()
}

pub fn arr(v: &Value, k: &str) -> Vec<Value> {
    v.get(k).and_then(Value::as_array).cloned().unwrap_or_default()
}

/// The places a written decimal has.
pub fn places(v: &str) -> u32 {
    v.split_once('.').map(|(_, f)| f.len() as u32).unwrap_or(0)
}

pub struct Built {
    pub inputs: Inputs,
    /// Each transaction label's id.
    pub tx: BTreeMap<String, TransactionId>,
    pub ids: Ids,
}

pub fn build(case: &Value) -> Built {
    let mut ids = Ids::default();
    let connection = ConnectionId::parse(&ids.uuid("connection")).unwrap();
    let broker = Broker::named("testbroker");
    let mut accounts = BTreeMap::new();
    for a in arr(case, "accounts") {
        let label = s(&a, "id").unwrap();
        let id = ids.account(label);
        let kind = AccountKind::parse(s(&a, "kind").unwrap_or("cash")).unwrap();
        let registration = Registration::parse(s(&a, "registration").unwrap_or("none")).unwrap();
        let status = AccountStatus::parse(s(&a, "status").unwrap_or("open")).unwrap();
        let account = Account { id, connection, account_type: AccountType::Known { kind, registration, managed: false, joint: false }, status, nickname: Some(label.to_string()) };
        accounts.insert(id, AccountInfo { account, broker: broker.clone() });
    }
    let first_day = arr(case, "transactions").iter().filter_map(|t| s(t, "day")).map(day).min().unwrap_or(day("2020-01-01"));
    let mut instruments = BTreeMap::new();
    let source = SourceName::named("case");
    for i in arr(case, "instruments") {
        let label = s(&i, "id").unwrap();
        let id = ids.instrument(label);
        let kind = InstrumentKind::parse(s(&i, "kind").unwrap_or("security")).unwrap();
        let currency = ccy(s(&i, "currency").unwrap_or("CAD"));
        let name = Name {
            symbol: s(&i, "symbol").unwrap_or(label).to_string(),
            venue_mic: None,
            venue_name: s(&i, "venue").map(str::to_string),
            name: s(&i, "name").map(str::to_string),
            first_seen: first_day,
            last_seen: first_day,
            source: source.clone(),
        };
        let terms = s(&i, "underlying").map(|u| OptionTerms {
            underlying: ids.instrument(u),
            expiry: day(s(&i, "expiry").unwrap()),
            strike: dec(s(&i, "strike").unwrap()),
            right: OptionRight::parse(s(&i, "right").unwrap()).unwrap(),
            multiplier: s(&i, "multiplier").map(dec),
            source: source.clone(),
        });
        instruments.insert(id, InstrumentInfo { instrument: Instrument { id, kind, currency, issuer: None }, names: vec![name], terms });
    }
    let mut tx = BTreeMap::new();
    let mut transactions = Vec::new();
    let mut records: BTreeMap<RecordId, RecordInfo> = BTreeMap::new();
    for t in arr(case, "transactions") {
        let label = s(&t, "id").unwrap().to_string();
        let record_label = s(&t, "record").unwrap_or(&label).to_string();
        let record = ids.record(&record_label);
        let id = TransactionId::new(record, Leg::parse(s(&t, "leg").unwrap_or("trade")).unwrap());
        let money = |amount: Option<&str>, currency: Option<&str>| amount.map(|a| Money::new(dec(a), ccy(currency.unwrap_or("CAD"))));
        let instrument = s(&t, "instrument").map(|i| ids.instrument(i));
        let instrument_ccy = s(&t, "instrument").and_then(|l| arr(case, "instruments").into_iter().find(|i| s(i, "id") == Some(l))).and_then(|i| s(&i, "currency").map(str::to_string));
        let default_ccy = s(&t, "currency").map(str::to_string).or(instrument_ccy).unwrap_or_else(|| "CAD".into());
        let transaction = Transaction {
            id: id.clone(),
            mapping: MappingVersion { source: source.clone(), version: 1 },
            account: ids.account(s(&t, "account").unwrap()),
            occurred_at: s(&t, "at").map(at),
            trade_date: day(s(&t, "day").unwrap()),
            settle_date: None,
            kind: Kind::parse(s(&t, "kind").unwrap()).unwrap(),
            effect: s(&t, "effect").map(|e| Effect::parse(e).unwrap()),
            instrument,
            quantity: s(&t, "qty").map(dec),
            price: money(s(&t, "price"), Some(&default_ccy)),
            cash: money(s(&t, "cash"), Some(s(&t, "cash_currency").unwrap_or(&default_ccy))),
            fee: money(s(&t, "fee"), Some(s(&t, "fee_currency").or(s(&t, "cash_currency")).unwrap_or(&default_ccy))),
            fx_rate: s(&t, "fx_rate").map(dec),
        };
        let info = records.entry(record).or_insert_with(|| RecordInfo { source_key: record_label.clone(), problems: vec![] });
        for p in arr(&t, "problems") {
            info.problems.push(Problem::new(p.as_str().unwrap(), "stated by the case"));
        }
        tx.insert(label, id);
        transactions.push(transaction);
    }
    let transfer_links = arr(case, "transfer_links").iter().map(|l| (tx[s(l, "out").unwrap()].clone(), tx[s(l, "in").unwrap()].clone())).collect();
    let mut trades = Vec::new();
    for t in arr(case, "trades") {
        let opened_by = &tx[s(&t, "opened_by").unwrap()];
        let instrument = ids.instrument(s(&t, "instrument").unwrap());
        trades.push(Trade { id: ids.trade(s(&t, "id").unwrap()), anchor: Anchor::Opening(Opening { transaction: opened_by.clone(), instrument }), legacy_key: None });
    }
    let groups = arr(case, "groups").iter().map(|g| Group { id: ids.group(s(g, "id").unwrap()), locked: true, members: arr(g, "members").iter().map(|m| ids.trade(m.as_str().unwrap())).collect() }).collect();
    let mut journal = BTreeMap::new();
    for j in arr(case, "journal") {
        let subject = match (s(&j, "trade"), s(&j, "group")) {
            (Some(t), _) => JournalSubject::Trade(ids.trade(t)),
            (_, Some(g)) => JournalSubject::Group(ids.group(g)),
            _ => panic!("a journal entry on nothing"),
        };
        let entry = JournalEntry { thesis: s(&j, "thesis").unwrap_or("").into(), grade: s(&j, "grade").map(|g| Grade::parse(g).unwrap()), tags: arr(&j, "tags").iter().map(|t| t.as_str().unwrap().to_string()).collect() };
        journal.insert(subject, entry);
    }
    let ledger = Ledger { accounts, instruments, transactions, records, transfer_links, trades, groups, journal };

    let mut rates = Rates::default();
    for (c, days) in obj(case, "rates") {
        for (d, r) in days.as_object().unwrap() {
            rates.by_currency.entry(ccy(&c)).or_default().insert(day(d), dec(r.as_str().unwrap()));
        }
    }
    for (c, spans) in obj(case, "covered") {
        for span in spans.as_array().unwrap() {
            let span = span.as_array().unwrap();
            // a read is taken at the case's moment unless it says when
            let read_at = span.get(2).and_then(Value::as_str).map(at).unwrap_or_else(|| case_now(case));
            rates.covered.entry(ccy(&c)).or_default().push(Read { first: day(span[0].as_str().unwrap()), last: day(span[1].as_str().unwrap()), at: read_at });
        }
    }
    rates.published = arr(case, "published").iter().map(|c| ccy(c.as_str().unwrap())).collect();
    rates.holidays = arr(case, "holidays").iter().map(|d| day(d.as_str().unwrap())).collect();
    let mut declared = BTreeMap::new();
    for (i, items) in obj(case, "declared") {
        let items = items
            .as_array()
            .unwrap()
            .iter()
            .map(|d| Declared {
                ex_date: day(s(d, "ex").unwrap()),
                record_date: None,
                pay_date: s(d, "pay").map(day),
                amount: Money::new(dec(s(d, "amount").unwrap()), ccy(s(d, "currency").unwrap_or("CAD"))),
                kind: match s(d, "kind").unwrap_or("regular") {
                    "regular" => DistributionKind::Regular,
                    "special" => DistributionKind::Special,
                    _ => DistributionKind::NonCash,
                },
            })
            .collect();
        declared.insert(ids.instrument(&i), DeclaredRead { read_at: Timestamp::UNIX_EPOCH, source: SourceName::named("tmx"), items });
    }
    let frequencies = obj(case, "frequencies").into_iter().map(|(i, n)| (ids.instrument(&i), Sourced { value: n.as_u64().unwrap() as u32, source: SourceName::named("tmx") })).collect();
    let mut adjustments = BTreeMap::new();
    for a in arr(case, "adjustments") {
        let applies_to = tx[s(&a, "applies_to").unwrap()].clone();
        let legs = arr(&a, "legs")
            .iter()
            .map(|l| AdjustmentLeg {
                from: s(l, "from").map(|i| ids.instrument(i)),
                to: s(l, "to").map(|i| ids.instrument(i)),
                units_per_unit: s(l, "units_per_unit").map(dec),
                cost_share: s(l, "cost_share").map(dec),
                cash_per_unit: s(l, "cash_per_unit").map(|c| Money::new(dec(c), ccy(s(l, "currency").unwrap_or("CAD")))),
                cost: s(l, "cost").map(|c| Money::new(dec(c), ccy(s(l, "currency").unwrap_or("CAD")))),
                acquired: s(l, "acquired").map(day),
            })
            .collect();
        let source = SourceName::parse(s(&a, "source").unwrap_or("issuer")).unwrap();
        adjustments.insert(applies_to.clone(), Adjustment { applies_to, legs, source });
    }
    let facts = Facts { rates, declared, frequencies, adjustments };

    let mut market = Market::default();
    for (i, q) in obj(case, "quotes") {
        let id = ids.instrument(&i);
        let currency = ledger.instruments[&id].instrument.currency;
        let source = match s(&q, "source").unwrap_or("listing") {
            "crypto" => QuoteSource::Crypto,
            "option" => QuoteSource::OptionChain,
            _ => QuoteSource::Listing,
        };
        market.quotes.insert(id, Quote { price: Money::new(dec(s(&q, "price").unwrap()), currency), change: s(&q, "change").map(dec), change_pct: None, at: None, source });
    }
    for (i, days) in obj(case, "closes") {
        let id = ids.instrument(&i);
        for (d, c) in days.as_object().unwrap() {
            market.closes.entry(id).or_default().insert(day(d), dec(c.as_str().unwrap()));
        }
    }
    for (k, days) in obj(case, "benchmarks") {
        for (d, c) in days.as_object().unwrap() {
            market.benchmarks.entry(k.clone()).or_default().insert(day(d), dec(c.as_str().unwrap()));
        }
    }
    for (a, b) in obj(case, "brokers") {
        let id = ids.account(&a);
        let mut acct = BrokerAccount::default();
        for (d, v) in obj(&b, "net_value") {
            acct.net_value.insert(day(&d), dec(v.as_str().unwrap()));
        }
        for (d, v) in obj(&b, "net_deposits") {
            acct.net_deposits.insert(day(&d), dec(v.as_str().unwrap()));
        }
        for (c, v) in obj(&b, "cash") {
            acct.cash.insert(ccy(&c), dec(v.as_str().unwrap()));
        }
        for (i, v) in obj(&b, "held") {
            acct.held.insert(ids.instrument(&i), dec(v.as_str().unwrap()));
        }
        acct.net_value_now = s(&b, "now").map(dec);
        acct.as_of = s(&b, "as_of").map(at);
        acct.activity_read_at = s(&b, "activity_read_at").map(at);
        acct.buying_power = s(&b, "buying_power").map(|v| Ok(dec(v)));
        market.brokers.insert(id, acct);
    }
    let today = day(s(case, "today").unwrap());
    let bank = TimeZone::get("America/Toronto").unwrap();
    let now = case_now(case);
    let clock = Clock { today, now, home: TimeZone::get(s(case, "home").unwrap_or("America/Edmonton")).unwrap(), bank };
    Built { inputs: Inputs { ledger, facts, market, clock }, tx, ids }
}

/// The engine on a case, with a trade for every round trip that needs one (as
/// the book would open them, one after another).
pub fn engine(b: &mut Built) -> Engine {
    let mut e = Engine::build(b.inputs.clone());
    let mut trades = b.inputs.ledger.trades.clone();
    for (n, key) in e.identity().needs_trade.clone().into_iter().enumerate() {
        let id = b.ids.trade(&format!("auto-{n}-{}", key.opening));
        trades.push(Trade { id, anchor: Anchor::Opening(Opening { transaction: key.opening, instrument: key.instrument }), legacy_key: None });
    }
    e.apply(Change::Trades(trades));
    e
}


/// The case's present moment: its `now`, else the end of its `today` in the
/// Bank's zone.
pub fn case_now(case: &Value) -> Timestamp {
    let today = day(s(case, "today").unwrap());
    s(case, "now").map(at).unwrap_or_else(|| today.at(23, 0, 0, 0).to_zoned(TimeZone::get("America/Toronto").unwrap()).unwrap().timestamp())
}
