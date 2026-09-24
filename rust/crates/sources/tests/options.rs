//! Held option contracts' prices from Cboe's delayed chains, on recorded real
//! replies: when a shown contract's chain is due, what a chain's price for a
//! contract is and when it was current, a read that asks Cboe only for a newer
//! chain, and a contract told apart by its OCC symbol where its terms alone do
//! not name one.

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
use bagholder_sources::cache::{ChainRead, MarketCache};
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

fn contract(n: u8, underlying: &str, expiry: Date, strike: &str, right: OptionRight) -> ContractNeed {
    ContractNeed { id: id(n), currency: Currency::USD, underlying: underlying.into(), expiry, strike: dec(strike), right, occ: None, event_on: None }
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

fn quotes(w: &World) -> BTreeMap<InstrumentId, (Money, Timestamp)> {
    w.cache.quotes().unwrap().into_iter().map(|q| (q.instrument, (q.price, q.quoted_at))).collect()
}

fn chain(session: Date, made_at: &str, last_modified: Option<&str>) -> ChainRead {
    ChainRead { underlying: "BBAI".into(), session, made_at: t(made_at), last_modified: last_modified.map(str::to_string), received_at: t(made_at) }
}

fn zone() -> TimeZone {
    TimeZone::get("America/Toronto").unwrap()
}

fn outcomes(w: &World) -> Vec<(Option<InstrumentId>, OutcomeKind, String)> {
    w.cache.outcomes(&SourceName::named(SOURCE)).unwrap().into_iter().map(|o| (o.instrument, o.outcome, o.detail)).collect()
}

#[test]
fn a_shown_contracts_chain_is_due_by_what_the_chain_held_states() {
    let z = zone();
    let due = |held: Option<&ChainRead>, now: &str| options::chain_due(held, t(now), &z);
    // nothing read: due
    assert!(due(None, "2026-09-26T15:00:00Z"));
    // a chain made in session on Tuesday the 22nd (14:28 Eastern)
    let in_session = chain(date(2026, 9, 22), "2026-09-22T18:28:00Z", Some("Tue, 22 Sep 2026 18:28:03 GMT"));
    assert!(!options::is_final(&in_session, &z));
    // due while the market trades, and after the session settles for its final prices
    assert!(due(Some(&in_session), "2026-09-22T19:00:00Z"));
    assert!(due(Some(&in_session), "2026-09-22T20:30:00Z"));
    assert!(due(Some(&in_session), "2026-09-23T03:00:00Z"));
    // a chain made after the session settled holds its final prices
    let closing = chain(date(2026, 9, 22), "2026-09-23T02:08:13Z", Some("Wed, 23 Sep 2026 02:08:16 GMT"));
    assert!(options::is_final(&closing, &z));
    // they stand overnight and before the next session opens
    assert!(!due(Some(&closing), "2026-09-23T03:00:00Z"));
    assert!(!due(Some(&closing), "2026-09-23T13:29:00Z"));
    // the next session opens at 09:30 Eastern: due
    assert!(due(Some(&closing), "2026-09-23T13:30:00Z"));
    // Friday's final prices stand through the weekend
    let friday = chain(date(2026, 9, 25), "2026-09-26T01:00:00Z", None);
    assert!(!due(Some(&friday), "2026-09-26T15:00:00Z"));
    assert!(!due(Some(&friday), "2026-09-27T15:00:00Z"));
    assert!(due(Some(&friday), "2026-09-28T13:30:00Z"));
}

#[test]
fn a_shown_contract_is_priced_from_its_chain_with_the_time_it_was_current() {
    let w = world(&[1, 2, 3, 4, 5]);
    let recorded = Arc::new(common::Recorded::new().with(&url("BBAI"), 200, "cboe-options", "chain-BBAI.json").with(&url("ZZZQX"), 403, "cboe-options", "chain-ZZZQX-status-403.json"));
    let mut by_occ = contract(2, "BBAI", date(2028, 1, 21), "5", OptionRight::Put);
    by_occ.occ = Some("BBAI280121P00005000".into());
    let shown = [
        // quoted both sides: the midpoint, at the chain's time less Cboe's fifteen minutes
        contract(1, "BBAI", date(2028, 1, 21), "10", OptionRight::Call),
        by_occ,
        // no bid: the last trade, at its own time
        contract(3, "BBAI", date(2026, 9, 25), "2", OptionRight::Put),
        contract(4, "BBAI", date(2026, 9, 25), "5", OptionRight::Call),
        // an underlying Cboe does not list
        contract(5, "ZZZQX", date(2026, 9, 25), "5", OptionRight::Call),
    ];
    run(&w, &recorded, "2026-09-23T03:00:00Z", &shown);
    let q = quotes(&w);
    assert_eq!(q[&id(1)], (usd("0.37"), t("2026-09-23T01:53:13Z")));
    assert_eq!(q[&id(2)].0, usd("2.49"));
    assert_eq!(q[&id(3)].0, usd("0.01"));
    assert_eq!(q[&id(4)], (usd("0.01"), t("2026-09-21T18:03:24Z")));
    assert!(!q.contains_key(&id(5)));
    // one request per underlying; the unlisted one is not carried, not a failure
    assert_eq!(*recorded.asked.lock().unwrap(), vec![url("BBAI"), url("ZZZQX")]);
    assert!(outcomes(&w).contains(&(None, OutcomeKind::NotCarried, "ZZZQX: status 403".into())));
    assert!(outcomes(&w).iter().all(|o| !o.1.is_failure()), "{:?}", outcomes(&w));
    // the chain held is the session's final one: nothing is asked again that night
    let quiet = Arc::new(common::Recorded::new());
    run(&w, &quiet, "2026-09-23T04:00:00Z", &shown[..4]);
    assert!(quiet.asked.lock().unwrap().is_empty());
}

#[test]
fn a_later_read_asks_only_for_a_newer_chain_and_a_304_keeps_the_prices_held() {
    let w = world(&[1]);
    let shown = [contract(1, "BBAI", date(2028, 1, 21), "10", OptionRight::Call)];
    // the 22nd's final chain, then asked again once the 23rd's session opens, when
    // Cboe has published nothing newer (as it answered on 2026-09-24)
    let first = Arc::new(common::Recorded::new().with(&url("BBAI"), 200, "cboe-options", "chain-BBAI.json"));
    run(&w, &first, "2026-09-23T03:00:00Z", &shown);
    let held = w.cache.option_chain("BBAI").unwrap().unwrap();
    assert_eq!(held.last_modified.as_deref(), Some("Wed, 23 Sep 2026 02:08:16 GMT"));
    let before = quotes(&w);
    let not_modified = Arc::new(common::Recorded::new().with(&url("BBAI"), 304, "cboe-options", "chain-BBAI-status-304.json"));
    run(&w, &not_modified, "2026-09-23T14:00:00Z", &shown);
    // the read named the chain held
    let sent = not_modified.headers.lock().unwrap()[0].clone();
    assert!(sent.iter().any(|(k, v)| k == "If-Modified-Since" && Some(v.as_str()) == held.last_modified.as_deref()), "{sent:?} {held:?}");
    assert_eq!(quotes(&w), before);
    let after = w.cache.option_chain("BBAI").unwrap().unwrap();
    assert_eq!((after.session, after.made_at, after.received_at), (held.session, held.made_at, t("2026-09-23T14:00:00Z")));
    // a contract shown for the first time is read whatever the chain held
    let mut two = shown.to_vec();
    two.push(contract(2, "BBAI", date(2026, 9, 25), "5", OptionRight::Call));
    let full = Arc::new(common::Recorded::new().with(&url("BBAI"), 200, "cboe-options", "chain-BBAI.json"));
    run(&w, &full, "2026-09-23T14:01:00Z", &two);
    assert!(full.headers.lock().unwrap()[0].iter().all(|(k, _)| k != "If-Modified-Since"));
    assert!(quotes(&w).contains_key(&id(2)));
}

#[test]
fn a_contract_past_its_expiry_is_not_asked_for() {
    let w = world(&[1]);
    let quiet = Arc::new(common::Recorded::new());
    run(&w, &quiet, "2026-09-23T15:00:00Z", &[contract(1, "BBAI", date(2026, 9, 18), "5", OptionRight::Put)]);
    assert!(quiet.asked.lock().unwrap().is_empty());
}

#[test]
fn a_chain_whose_session_cannot_be_told_prices_nothing_and_rests() {
    for (name, kind) in [("wrong-meaning-chain-BBAI-no-session.json", OutcomeKind::Meaning), ("wrong-shape-chain-BBAI-bid-as-text.json", OutcomeKind::Mismatch)] {
        let w = world(&[1]);
        let shown = [contract(1, "BBAI", date(2028, 1, 21), "10", OptionRight::Call)];
        let failing = Arc::new(common::Recorded::new().with(&url("BBAI"), 200, "cboe-options", name));
        let rest = common::net(&failing, "2026-09-23T15:00:00Z").limiter().pace(cboe_options::HOST).rest;
        let at = t("2026-09-23T15:00:00Z");
        let plus = |d: std::time::Duration| (at + bagholder_core::jiff::SignedDuration::try_from(d).unwrap()).to_string();
        run(&w, &failing, "2026-09-23T15:00:00Z", &shown);
        assert!(w.cache.quotes().unwrap().is_empty(), "{name}");
        assert_eq!(outcomes(&w)[0].1, kind, "{name}");
        // within the rest: not asked, though shown and in session
        let quiet = Arc::new(common::Recorded::new());
        run(&w, &quiet, &plus(rest - std::time::Duration::from_secs(1)), &shown);
        assert!(quiet.asked.lock().unwrap().is_empty(), "{name}");
        // once the rest is over: asked again, and priced
        let answering = Arc::new(common::Recorded::new().with(&url("BBAI"), 200, "cboe-options", "chain-BBAI.json"));
        run(&w, &answering, &plus(rest), &shown);
        assert_eq!(quotes(&w)[&id(1)].0, usd("0.37"), "{name}");
    }
}

#[test]
fn a_contract_is_told_apart_by_its_occ_symbol_where_its_terms_match_two() {
    let w = world(&[1, 2, 3]);
    let recorded = Arc::new(common::Recorded::new().with(&url("BBAI"), 200, "cboe-options", "edited-chain-BBAI-adjusted-beside.json"));
    let by_terms = contract(1, "BBAI", date(2028, 1, 21), "10", OptionRight::Call);
    let mut by_occ = contract(2, "BBAI", date(2028, 1, 21), "10", OptionRight::Call);
    by_occ.occ = Some("BBAI280121C00010000".into());
    // a corporate event on the underlying while it was held: only the OCC symbol says which
    let mut after_event = contract(3, "BBAI", date(2028, 1, 21), "5", OptionRight::Put);
    after_event.event_on = Some(date(2026, 9, 10));
    run(&w, &recorded, "2026-09-23T03:00:00Z", &[by_terms, by_occ, after_event]);
    assert_eq!(quotes(&w).keys().copied().collect::<Vec<_>>(), vec![id(2)]);
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

