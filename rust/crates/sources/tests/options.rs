//! Option closes and quotes from Cboe's delayed chains, on recorded real
//! replies: each held contract's close written once into the book, dated to the
//! session the chain states; a chain in session, or one whose session cannot be
//! told, writing none; a contract told apart by its OCC symbol where its terms
//! alone do not name one.

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use bagholder_book::Book;
use bagholder_core::instrument::OptionRight;
use bagholder_core::jiff::civil::{date, Date};
use bagholder_core::jiff::tz::TimeZone;
use bagholder_core::jiff::Timestamp;
use bagholder_core::{Currency, Dec, InstrumentId, Money, SourceName};
use bagholder_sources::adapters::cboe_options;
use bagholder_sources::cache::MarketCache;
use bagholder_sources::contract::DataKind;
use bagholder_sources::needs::ContractNeed;
use bagholder_sources::options;
use bagholder_sources::outcome::{Outcome, OutcomeKind};
use bagholder_sources::read::Ctx;

const SOURCE: &str = "cboe-options";

fn url(symbol: &str) -> String {
    format!("https://cdn.cboe.com/api/global/delayed_quotes/options/{symbol}.json")
}

fn t(s: &str) -> Timestamp {
    s.parse().unwrap()
}

fn dec(s: &str) -> Dec {
    Dec::parse(s).unwrap()
}

fn id(n: u8) -> InstrumentId {
    InstrumentId::parse(&format!("0192a000-0000-7000-8000-0000000000{n:02}")).unwrap()
}

fn usd(s: &str) -> Money {
    Money::new(dec(s), Currency::USD)
}

fn contract(n: u8, underlying: &str, expiry: Date, strike: &str, right: OptionRight, to: Date) -> ContractNeed {
    ContractNeed { id: id(n), currency: Currency::USD, underlying: underlying.into(), expiry, strike: dec(strike), right, occ: None, event_on: None, from: date(2026, 9, 1), to }
}

struct World {
    _dir: tempfile::TempDir,
    book: Book,
    cache: MarketCache,
    zone: TimeZone,
}

fn world(contracts: &[u8]) -> World {
    let dir = tempfile::tempdir().unwrap();
    let at = t("2026-09-22T00:00:00Z");
    let (book, _) = Book::open(&dir.path().join("book.db"), "test", at).unwrap();
    for n in contracts {
        common::instrument_in_book(&dir.path().join("book.db"), id(*n), "option", "USD");
    }
    let (cache, _) = MarketCache::open(&dir.path().join("market.db"), "test", at).unwrap();
    World { _dir: dir, book, cache, zone: TimeZone::get("America/Toronto").unwrap() }
}

fn run(w: &World, recorded: &Arc<common::Recorded>, now: &str, contracts: &[ContractNeed]) {
    let net = common::net(recorded, now);
    let ctx = Ctx { book: &w.book, cache: &w.cache, net: &net, now: t(now), bank: &w.zone };
    options::read(&ctx, contracts).unwrap();
}

fn closes(w: &World) -> BTreeMap<InstrumentId, BTreeMap<Date, Money>> {
    w.book.closes().unwrap()
}

fn outcomes(w: &World) -> Vec<(Option<InstrumentId>, OutcomeKind, String)> {
    w.cache.outcomes(&SourceName::named(SOURCE)).unwrap().into_iter().map(|o| (o.instrument, o.outcome, o.detail)).collect()
}

#[test]
fn a_chain_read_after_the_close_writes_each_held_contracts_close_once_dated_to_its_session() {
    let w = world(&[1, 2, 3, 4, 5]);
    let recorded = Arc::new(common::Recorded::new().with(&url("BBAI"), 200, "cboe-options", "chain-BBAI.json").with(&url("ZZZQX"), 403, "cboe-options", "chain-ZZZQX-status-403.json"));
    let today = date(2026, 9, 22);
    let mut by_occ = contract(2, "BBAI", date(2028, 1, 21), "5", OptionRight::Put, today);
    by_occ.occ = Some("BBAI280121P00005000".into());
    let held = [
        // quoted both sides at the close: the midpoint
        contract(1, "BBAI", date(2028, 1, 21), "10", OptionRight::Call, today),
        by_occ,
        // no bid: the last trade, made that session
        contract(3, "BBAI", date(2026, 9, 25), "2", OptionRight::Put, today),
        // no bid, last traded the session before: no close stated for the day
        contract(4, "BBAI", date(2026, 9, 25), "5", OptionRight::Call, today),
        // an underlying Cboe does not list
        contract(5, "ZZZQX", date(2026, 9, 25), "5", OptionRight::Call, today),
    ];
    // Cboe made the chain at 02:08 UTC on the 23rd, after the 22nd's session
    run(&w, &recorded, "2026-09-23T03:00:00Z", &held);
    let got = closes(&w);
    assert_eq!(got[&id(1)], BTreeMap::from([(today, usd("0.37"))]));
    assert_eq!(got[&id(2)], BTreeMap::from([(today, usd("2.49"))]));
    assert_eq!(got[&id(3)], BTreeMap::from([(today, usd("0.01"))]));
    assert!(!got.contains_key(&id(4)) && !got.contains_key(&id(5)));
    // the contracts held today are quoted from the same chain: the midpoint at the
    // chain's time less the fifteen minutes it runs behind, else the last trade at its time
    let quotes: BTreeMap<InstrumentId, (Money, Timestamp)> = w.cache.quotes().unwrap().into_iter().map(|q| (q.instrument, (q.price, q.quoted_at))).collect();
    assert_eq!(quotes[&id(1)], (usd("0.37"), t("2026-09-23T01:53:13Z")));
    assert_eq!(quotes[&id(4)], (usd("0.01"), t("2026-09-21T18:03:24Z")));
    assert!(!quotes.contains_key(&id(5)));
    // one request per underlying; the unlisted one is not carried, not a failure
    assert_eq!(*recorded.asked.lock().unwrap(), vec![url("BBAI"), url("ZZZQX")]);
    assert!(outcomes(&w).contains(&(None, OutcomeKind::NotCarried, "ZZZQX: status 403".into())));
    assert!(outcomes(&w).iter().all(|o| !o.1.is_failure()), "{:?}", outcomes(&w));
    // the next day, before its session settles, nothing is due and nothing asked
    let quiet = Arc::new(common::Recorded::new());
    run(&w, &quiet, "2026-09-23T12:00:00Z", &held.map(|mut c| {
        c.to = today;
        c
    }));
    assert!(quiet.asked.lock().unwrap().is_empty());
    assert_eq!(closes(&w), got);
}

#[test]
fn a_chain_still_on_an_older_session_writes_that_sessions_closes_and_leaves_the_day_due() {
    let w = world(&[1]);
    let recorded = Arc::new(common::Recorded::new().with(&url("BBAI"), 200, "cboe-options", "chain-BBAI.json"));
    let held = [contract(1, "BBAI", date(2026, 9, 25), "2.5", OptionRight::Call, date(2026, 9, 24))];
    // on the 24th the 23rd is due, and the chain still carries the 22nd
    run(&w, &recorded, "2026-09-24T14:32:00Z", &held);
    assert_eq!(closes(&w)[&id(1)], BTreeMap::from([(date(2026, 9, 22), usd("0.4"))]));
    let reads = w.cache.reads(&id(1).to_string(), DataKind::OptionClose).unwrap();
    assert_eq!((reads[0].first, reads[0].last, reads[0].outcome), (date(2026, 9, 22), date(2026, 9, 22), OutcomeKind::Answered));
    // the 23rd is still due: the next read asks again
    run(&w, &recorded, "2026-09-24T14:40:00Z", &held);
    assert_eq!(recorded.asked.lock().unwrap().len(), 2);
}

#[test]
fn a_contract_expiring_on_the_session_is_closed_that_day() {
    let w = world(&[1]);
    let recorded = Arc::new(common::Recorded::new().with(&url("BBAI"), 200, "cboe-options", "edited-chain-BBAI-expiry-day.json"));
    let expiry = date(2026, 9, 25);
    run(&w, &recorded, "2026-09-26T03:00:00Z", &[contract(1, "BBAI", expiry, "2.5", OptionRight::Call, expiry)]);
    assert_eq!(closes(&w)[&id(1)], BTreeMap::from([(expiry, usd("0.4"))]));
}

#[test]
fn a_chain_past_the_day_due_settles_it_as_not_read() {
    let w = world(&[1]);
    let recorded = Arc::new(common::Recorded::new().with(&url("BBAI"), 200, "cboe-options", "edited-chain-BBAI-expiry-day.json"));
    // held to the 24th, read only once the chain carries the 25th
    run(&w, &recorded, "2026-09-26T03:00:00Z", &[contract(1, "BBAI", date(2026, 10, 2), "2.5", OptionRight::Call, date(2026, 9, 24))]);
    assert!(!closes(&w).contains_key(&id(1)));
    let reads = w.cache.reads(&id(1).to_string(), DataKind::OptionClose).unwrap();
    assert_eq!((reads[0].first, reads[0].last, reads[0].outcome), (date(2026, 9, 24), date(2026, 9, 24), OutcomeKind::NotCarried));
    assert!(outcomes(&w).iter().any(|o| o.0 == Some(id(1)) && o.2.contains("2026-09-24: the chain has moved on to 2026-09-25")));
}

#[test]
fn a_contract_past_its_expiry_is_neither_quoted_nor_asked_again() {
    let w = world(&[1]);
    let recorded = Arc::new(common::Recorded::new().with(&url("BBAI"), 200, "cboe-options", "chain-BBAI.json"));
    // still held by the ledger (no expiry row), its expiry four days before the chain's session
    let held = [contract(1, "BBAI", date(2026, 9, 18), "5", OptionRight::Put, date(2026, 9, 22))];
    run(&w, &recorded, "2026-09-23T03:00:00Z", &held);
    assert!(closes(&w).is_empty() && w.cache.quotes().unwrap().is_empty());
    assert!(outcomes(&w).iter().any(|o| o.0 == Some(id(1)) && o.2.contains("2026-09-18: the chain has moved on to 2026-09-22")));
    run(&w, &recorded, "2026-09-23T03:10:00Z", &held);
    assert_eq!(recorded.asked.lock().unwrap().len(), 1);
}

#[test]
fn a_chain_made_in_session_quotes_but_closes_nothing() {
    let w = world(&[1]);
    let recorded = Arc::new(common::Recorded::new().with(&url("BBAI"), 200, "cboe-options", "edited-chain-BBAI-in-session.json"));
    let today = date(2026, 9, 22);
    run(&w, &recorded, "2026-09-22T19:30:00Z", &[contract(1, "BBAI", date(2028, 1, 21), "10", OptionRight::Call, today)]);
    assert!(closes(&w).is_empty());
    assert!(w.cache.reads(&id(1).to_string(), DataKind::OptionClose).unwrap().is_empty());
    assert_eq!(w.cache.quotes().unwrap()[0].price, usd("0.37"));
}

#[test]
fn a_chain_whose_session_cannot_be_told_writes_nothing() {
    for (name, kind) in [("wrong-meaning-chain-BBAI-no-session.json", OutcomeKind::Meaning), ("wrong-shape-chain-BBAI-bid-as-text.json", OutcomeKind::Mismatch)] {
        let w = world(&[1]);
        let recorded = Arc::new(common::Recorded::new().with(&url("BBAI"), 200, "cboe-options", name));
        run(&w, &recorded, "2026-09-23T03:00:00Z", &[contract(1, "BBAI", date(2028, 1, 21), "10", OptionRight::Call, date(2026, 9, 22))]);
        assert!(closes(&w).is_empty() && w.cache.quotes().unwrap().is_empty(), "{name}");
        assert_eq!(outcomes(&w)[0].1, kind, "{name}");
        // a failed read rests: the day is asked again on the source's rest
        assert!(w.cache.reads(&id(1).to_string(), DataKind::OptionClose).unwrap()[0].outcome.is_failure());
    }
}

#[test]
fn a_contract_is_told_apart_by_its_occ_symbol_where_its_terms_match_two() {
    let w = world(&[1, 2, 3]);
    let recorded = Arc::new(common::Recorded::new().with(&url("BBAI"), 200, "cboe-options", "edited-chain-BBAI-adjusted-beside.json"));
    let today = date(2026, 9, 22);
    let by_terms = contract(1, "BBAI", date(2028, 1, 21), "10", OptionRight::Call, today);
    let mut by_occ = contract(2, "BBAI", date(2028, 1, 21), "10", OptionRight::Call, today);
    by_occ.occ = Some("BBAI280121C00010000".into());
    // a corporate event on the underlying while it was held: only the OCC symbol says which
    let mut after_event = contract(3, "BBAI", date(2028, 1, 21), "5", OptionRight::Put, today);
    after_event.event_on = Some(date(2026, 9, 10));
    run(&w, &recorded, "2026-09-23T03:00:00Z", &[by_terms, by_occ, after_event]);
    let got = closes(&w);
    assert_eq!(got.keys().copied().collect::<Vec<_>>(), vec![id(2)]);
    let o = outcomes(&w);
    assert!(o.iter().any(|o| o.0 == Some(id(1)) && o.1 == OutcomeKind::Meaning && o.2.contains("BBAI280121C00010000 and BBAI1280121C00010000")), "{o:?}");
    assert!(o.iter().any(|o| o.0 == Some(id(3)) && o.1 == OutcomeKind::Meaning && o.2.contains("corporate event of 2026-09-10")), "{o:?}");
}

#[test]
fn an_occ_symbol_states_its_terms() {
    assert_eq!(cboe_options::occ_terms("BBAI280121C00010000"), Some((date(2028, 1, 21), OptionRight::Call, dec("10"))));
    assert_eq!(cboe_options::occ_terms("BBAI260925C00000500"), Some((date(2026, 9, 25), OptionRight::Call, dec("0.5"))));
    assert_eq!(cboe_options::occ_terms("BRKB260925P00270000"), Some((date(2026, 9, 25), OptionRight::Put, dec("270"))));
    for bad in ["280121C00010000", "BBAI280121X00010000", "BBAI281321C00010000", "BBAI280121C0001000"] {
        assert_eq!(cboe_options::occ_terms(bad), None, "{bad}");
    }
}

#[test]
fn prices_are_the_decimals_their_digits_spell() {
    let Outcome::Answered(chain) = cboe_options::parse(&common::json("cboe-options", "chain-BBAI.json"), "BBAI") else { panic!() };
    assert_eq!((chain.made_at, chain.session), (t("2026-09-23T02:08:13Z"), date(2026, 9, 22)));
    let k = chain.contracts.iter().find(|k| k.occ == "BBAI280121P00005000").unwrap();
    assert_eq!((k.bid, k.ask, k.last.unwrap().0), (dec("2.36"), dec("2.62"), dec("2.59")));
    // another underlying's chain is not this one's
    assert!(matches!(cboe_options::parse(&common::json("cboe-options", "chain-BBAI.json"), "BBAIX"), Outcome::Meaning(_)));
}

#[test]
fn the_recorded_shape_is_the_answers_union() {
    common::shape_is_the_answers_union("cboe-options.paths", "cboe-options", "chain-", &[]);
}

#[test]
fn after_a_failed_chain_read_the_contracts_day_waits_out_the_sources_rest_then_is_asked_again() {
    let w = world(&[1]);
    // held to the 22nd, so it is asked only while its close is due, never to be quoted
    let held = [contract(1, "BBAI", date(2028, 1, 21), "10", OptionRight::Call, date(2026, 9, 22))];
    // (a reply of another status than 200 is not recorded for Cboe's chains but the 403 that
    // means "not carried"; a chain whose session cannot be told is the failure here)
    let failing = Arc::new(common::Recorded::new().with(&url("BBAI"), 200, "cboe-options", "wrong-meaning-chain-BBAI-no-session.json"));
    let rest = common::net(&failing, "2026-09-23T12:00:00Z").limiter().pace(cboe_options::HOST).rest;
    let at = t("2026-09-23T12:00:00Z");
    let plus = |d: std::time::Duration| (at + bagholder_core::jiff::SignedDuration::try_from(d).unwrap()).to_string();
    run(&w, &failing, "2026-09-23T12:00:00Z", &held);
    assert_eq!(failing.asked.lock().unwrap().len(), 1);
    assert!(closes(&w).is_empty());
    let reads = w.cache.reads(&id(1).to_string(), DataKind::OptionClose).unwrap();
    assert_eq!((reads[0].first, reads[0].last, reads[0].outcome, reads[0].at), (date(2026, 9, 22), date(2026, 9, 22), OutcomeKind::Meaning, at));
    // within the rest: not asked
    let quiet = Arc::new(common::Recorded::new());
    run(&w, &quiet, &plus(rest - std::time::Duration::from_secs(1)), &held);
    assert!(quiet.asked.lock().unwrap().is_empty());
    // once the rest is over: asked again, and the session's close written
    let answering = Arc::new(common::Recorded::new().with(&url("BBAI"), 200, "cboe-options", "chain-BBAI.json"));
    run(&w, &answering, &plus(rest), &held);
    assert_eq!(*answering.asked.lock().unwrap(), vec![url("BBAI")]);
    assert_eq!(closes(&w)[&id(1)], BTreeMap::from([(date(2026, 9, 22), usd("0.37"))]));
}
