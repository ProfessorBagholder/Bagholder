//! What the person follows: the watched listings and the tile row, each an
//! instrument found as §5 finds any (`docs/plans/stage-5-interface-and-running.md`, A2).

mod common;

use bagholder_book::clear::Clearing;
use bagholder_book::watched::{ListingDraft, ListingName};
use bagholder_core::instrument::{InstrumentKind, RefScheme, Reference};
use bagholder_core::Currency;
use common::*;

fn named(symbol: &str, mic: Option<&str>) -> ListingName {
    ListingName { symbol: symbol.into(), venue_mic: mic.map(str::to_string), venue_name: None, name: Some(format!("{symbol} Inc")) }
}

fn listing(symbol: &str, mic: &str) -> ListingDraft {
    ListingDraft {
        found: None,
        kind: InstrumentKind::Security,
        currency: Currency::CAD,
        refs: vec![Reference::new(RefScheme::Listing, format!("{symbol}@{mic}"))],
        name: named(symbol, Some(mic)),
    }
}

fn directory(symbol: &str, venue: &str, kind: InstrumentKind, yahoo: &str) -> ListingDraft {
    ListingDraft {
        found: None,
        kind,
        currency: Currency::USD,
        refs: vec![Reference::new(RefScheme::Directory, format!("{symbol}@{}", venue.to_uppercase())), Reference::new(RefScheme::Yahoo, yahoo)],
        name: ListingName { symbol: symbol.into(), venue_mic: None, venue_name: Some(venue.into()), name: None },
    }
}

#[test]
fn a_listing_watched_twice_is_one_row_and_keeps_when_it_was_first_added() {
    let f = Fixture::new();
    let a = f.book.watch(&listing("QNC", "XTSX"), t0()).unwrap();
    let b = f.book.watch(&listing("QNC", "XTSX"), at("2026-09-24T12:00:00Z")).unwrap();
    assert_eq!(a, b);
    let w = f.book.watched().unwrap();
    assert_eq!(w.len(), 1);
    assert_eq!(w[0].added_at, t0());
    assert_eq!(f.book.current_name(a).unwrap(), Some(named("QNC", Some("XTSX"))));
}

#[test]
fn the_watchlist_is_newest_first_and_a_row_goes_by_its_instrument() {
    let f = Fixture::new();
    let a = f.book.watch(&listing("AAA", "XTSE"), at("2026-09-20T12:00:00Z")).unwrap();
    let b = f.book.watch(&listing("BBB", "XTSE"), at("2026-09-21T12:00:00Z")).unwrap();
    assert_eq!(f.book.watched().unwrap().iter().map(|w| w.instrument).collect::<Vec<_>>(), vec![b, a]);
    assert!(f.book.unwatch(b).unwrap());
    assert!(!f.book.unwatch(b).unwrap(), "a row not watched is not removed twice");
    assert_eq!(f.book.watched().unwrap().iter().map(|w| w.instrument).collect::<Vec<_>>(), vec![a]);
}

#[test]
fn a_held_listing_watched_is_that_instrument_and_keeps_its_records_name() {
    let f = Fixture::new();
    f.account(&["acc-1"]);
    f.store(&Spelled::v(1), "r1", &legs(vec![buy("acc-1", share("CA0000000001", "QNC"), "10", "-10", "2026-01-02T15:00:00Z")]));
    let held = f.book.instruments().unwrap()[0].id;
    let d = ListingDraft { found: Some(held), ..listing("QNC", "XTSX") };
    assert_eq!(f.book.watch(&d, t0()).unwrap(), held);
    assert_eq!(f.book.current_name(held).unwrap().map(|n| (n.symbol, n.venue_mic)), Some(("QNC".into(), None)), "the records' name stands before the one picked");
    assert_eq!(f.book.listing_named(held).unwrap(), Some(named("QNC", Some("XTSX"))));
    assert_eq!(f.book.instruments().unwrap().len(), 1);
}

#[test]
fn a_broker_s_security_id_finds_the_instrument_that_holds_it() {
    let f = Fixture::new();
    let ws = |id: &str| Reference::new(RefScheme::BrokerSecurity(bagholder_core::Broker::named("wealthsimple")), id);
    let first = f.book.watch(&ListingDraft { refs: vec![ws("sec-s-1")], ..listing("RY", "XTSE") }, t0()).unwrap();
    f.book.unwatch(first).unwrap();
    let again = f.book.watch(&ListingDraft { refs: vec![ws("sec-s-1")], ..listing("RY", "XTSE") }, t0()).unwrap();
    assert_eq!(first, again);
}

#[test]
fn an_instrument_of_the_directory_is_one_instrument_as_a_tile_and_watched() {
    let f = Fixture::new();
    let spx = directory("SPX", "Index", InstrumentKind::Index, "^GSPC");
    let tiles = f.book.set_tiles(&[spx.clone(), directory("GC", "COMEX", InstrumentKind::Future, "GC=F")], t0()).unwrap();
    let watched = f.book.watch(&spx, t0()).unwrap();
    assert_eq!(tiles[0], watched);
    assert_eq!(f.book.tiles().unwrap(), Some(tiles.clone()));
    let refs = f.book.instrument_refs(watched).unwrap();
    assert!(refs.contains(&Reference::new(RefScheme::Yahoo, "^GSPC")), "the directory's code is kept as the way to ask Yahoo");
}

#[test]
fn a_tile_row_never_chosen_is_none_and_one_chosen_empty_is_empty() {
    let f = Fixture::new();
    assert_eq!(f.book.tiles().unwrap(), None);
    f.book.set_tiles(&[], t0()).unwrap();
    assert_eq!(f.book.tiles().unwrap(), Some(vec![]));
}

#[test]
fn the_tile_row_keeps_its_order_and_refuses_one_instrument_twice() {
    let f = Fixture::new();
    let (a, b) = (directory("SPX", "Index", InstrumentKind::Index, "^GSPC"), directory("VIX", "Index", InstrumentKind::Index, "^VIX"));
    let first = f.book.set_tiles(&[a.clone(), b.clone()], t0()).unwrap();
    let turned = f.book.set_tiles(&[b.clone(), a.clone()], t0()).unwrap();
    assert_eq!(turned, vec![first[1], first[0]]);
    assert_eq!(f.book.tiles().unwrap(), Some(turned.clone()));
    assert!(f.book.set_tiles(&[a.clone(), a], t0()).is_err());
    assert_eq!(f.book.tiles().unwrap(), Some(turned), "a refused row changes nothing");
}

#[test]
fn a_kind_or_currency_other_than_the_instrument_s_is_refused() {
    let f = Fixture::new();
    let id = f.book.watch(&listing("QNC", "XTSX"), t0()).unwrap();
    f.book.unwatch(id).unwrap();
    let usd = ListingDraft { currency: Currency::USD, ..listing("QNC", "XTSX") };
    assert!(f.book.watch(&usd, t0()).is_err());
    assert!(f.book.watched().unwrap().is_empty());
}

#[test]
fn the_earlier_store_s_rows_are_carried_once_all_or_none() {
    let f = Fixture::new();
    let rows = vec![(listing("AAA", "XTSE"), at("2026-09-01T12:00:00Z")), (listing("BBB", "XNAS"), at("2026-09-02T12:00:00Z"))];
    let tiles = vec![directory("SPX", "Index", InstrumentKind::Index, "^GSPC")];
    // one bad row refuses the whole carry
    let bad = vec![rows[0].clone(), (ListingDraft { currency: Currency::USD, ..listing("AAA", "XTSE") }, t0())];
    assert!(f.book.carry_following(&bad, Some(&tiles), t0()).is_err());
    assert!(!f.book.following_carried().unwrap());
    assert!(f.book.watched().unwrap().is_empty() && f.book.tiles().unwrap().is_none() && f.book.instruments().unwrap().is_empty());
    f.book.carry_following(&rows, Some(&tiles), t0()).unwrap();
    assert!(f.book.following_carried().unwrap());
    let w = f.book.watched().unwrap();
    assert_eq!(w.iter().map(|w| w.added_at).collect::<Vec<_>>(), vec![rows[1].1, rows[0].1]);
    // carried once: a second carry is nothing
    f.book.carry_following(&[(listing("CCC", "XTSE"), t0())], None, t0()).unwrap();
    assert_eq!(f.book.watched().unwrap().len(), 2);
    assert_eq!(f.book.tiles().unwrap().map(|t| t.len()), Some(1));
}

#[test]
fn clearing_what_is_followed_takes_the_instruments_nothing_else_names() {
    let f = Fixture::new();
    f.account(&["acc-1"]);
    f.store(&Spelled::v(1), "r1", &legs(vec![buy("acc-1", share("CA0000000001", "QNC"), "10", "-10", "2026-01-02T15:00:00Z")]));
    let held = f.book.instruments().unwrap()[0].id;
    f.book.watch(&ListingDraft { found: Some(held), ..listing("QNC", "XTSX") }, t0()).unwrap();
    f.book.watch(&listing("ZZZ", "XTSE"), t0()).unwrap();
    f.book.set_tiles(&[directory("SPX", "Index", InstrumentKind::Index, "^GSPC")], t0()).unwrap();
    // clearing the records leaves what is followed, and its instruments
    f.book.clear(&Clearing { broker: true, entries: true, ..Clearing::default() }).unwrap();
    assert_eq!(f.book.watched().unwrap().len(), 2);
    assert_eq!(f.book.current_name(held).unwrap(), Some(named("QNC", Some("XTSX"))), "with its records gone, it is still called what it was picked as");
    assert_eq!(f.book.instruments().unwrap().len(), 3);
    f.book.clear(&Clearing { following: true, ..Clearing::default() }).unwrap();
    assert!(f.book.watched().unwrap().is_empty());
    assert_eq!(f.book.tiles().unwrap(), None);
    assert!(f.book.instruments().unwrap().is_empty());
    let named: i64 = f.book.conn_for_tests().query_row("SELECT COUNT(*) FROM listings_named", [], |r| r.get(0)).unwrap();
    assert_eq!(named, 0);
}
