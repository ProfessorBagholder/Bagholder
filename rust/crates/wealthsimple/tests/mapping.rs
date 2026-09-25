//! The mapping on real rows of the owner's history (`tests/replies/wealthsimple`,
//! one row of each kind Wealthsimple sent, anonymised), each rule of
//! `docs/plans/stage-3b-wealthsimple.md` ("The mapping", questions 1–8) held by a
//! row of its kind.

mod common;

use bagholder_book::mapping::{Draft, Mapped};
use bagholder_core::instrument::RefScheme;
use bagholder_core::json::{self, Value};
use bagholder_core::transaction::{Effect, Kind};
use bagholder_core::{Currency, Dec};
use bagholder_sources::reply::Node;
use common::*;

fn dec(s: &str) -> Dec {
    Dec::parse(s).unwrap()
}

fn all() -> Vec<(Value, Mapped)> {
    map_all(&mut replay(&fixtures()))
}

fn text<'a>(row: &'a Value, key: &str) -> Option<&'a str> {
    Node::root(row).opt_text(key).ok().flatten()
}

fn one(rows: &[(Value, Mapped)], ty: &str, sub: Option<&str>, status: &str) -> (Value, Mapped) {
    rows.iter().find(|(r, _)| text(r, "type") == Some(ty) && text(r, "subType") == sub && text(r, "unifiedStatus") == Some(status)).cloned().unwrap_or_else(|| panic!("no {ty} {sub:?} {status} row"))
}

fn security(d: &Draft) -> &str {
    let i = d.instrument.as_ref().expect("an instrument");
    &i.refs.iter().find(|r| matches!(r.scheme, RefScheme::BrokerSecurity(_))).expect("Wealthsimple's id").value
}

#[test]
fn every_row_is_read_and_placed() {
    for (row, m) in all() {
        for p in &m.problems {
            assert!(p.code != "unreadable" && p.code != "unclassified", "{} {:?}: {} {}", text(&row, "type").unwrap(), text(&row, "subType"), p.code, p.detail);
        }
    }
}

#[test]
fn only_what_wealthsimple_states_as_executed_moves() {
    for (row, m) in all() {
        if text(&row, "unifiedStatus") != Some("COMPLETED") {
            assert!(m.legs.is_empty() && m.problems.is_empty(), "{} {:?} {:?} moved", text(&row, "type").unwrap(), text(&row, "subType"), text(&row, "unifiedStatus"));
        }
    }
}

#[test]
fn a_buy_pays_and_a_sale_receives_whatever_the_sign_says() {
    let rows = all();
    let mut seen = 0;
    for (row, m) in &rows {
        for d in &m.legs {
            if text(row, "type") == Some("OPTIONS_MULTILEG") {
                continue;
            }
            match d.kind {
                Kind::Buy => {
                    assert!(d.quantity.unwrap().is_positive() && d.cash.unwrap().amount.is_negative(), "{row:?}");
                    // Wealthsimple writes a buy's sign as positive
                    assert_eq!(text(row, "amountSign"), Some("positive"));
                    seen += 1;
                }
                Kind::Sell => {
                    assert!(d.quantity.unwrap().is_negative() && d.cash.unwrap().amount.is_positive(), "{row:?}");
                    seen += 1;
                }
                _ => {}
            }
        }
    }
    assert!(seen > 20);
}

#[test]
fn a_multi_leg_order_is_its_legs_and_their_cash() {
    let (row, m) = one(&all(), "OPTIONS_MULTILEG", None, "COMPLETED");
    assert_eq!(m.legs.len(), 2);
    let buy = m.legs.iter().find(|d| d.kind == Kind::Buy).unwrap();
    let sell = m.legs.iter().find(|d| d.kind == Kind::Sell).unwrap();
    // each leg's opening or closing as the order states it
    assert_eq!((buy.effect, sell.effect), (Some(Effect::Close), Some(Effect::Open)));
    assert_ne!(security(buy), security(sell));
    // the legs' net is the row's amount, with the opposite sign to the row's
    let net = buy.cash.unwrap().amount.checked_add(sell.cash.unwrap().amount).unwrap();
    assert_eq!(net.abs(), Dec::parse(text(&row, "amount").unwrap()).unwrap());
    assert_eq!(text(&row, "amountSign"), Some("negative"));
    assert!(net.is_positive());
    assert!(buy.price.is_some() && buy.quantity.unwrap().is_positive() && sell.quantity.unwrap().is_negative());
    // the contract size, stated on the contract's record
    assert_eq!(buy.instrument.as_ref().unwrap().option.as_ref().unwrap().multiplier, Some(dec("100")));
    assert!(buy.instrument.as_ref().unwrap().refs.iter().any(|r| r.scheme == RefScheme::Occ));
}

#[test]
fn an_order_that_filled_nothing_moves_nothing() {
    let rows = all();
    for status in ["CANCELLED", "EXPIRED"] {
        let (_, m) = one(&rows, "OPTIONS_MULTILEG", None, status);
        assert!(m.legs.is_empty() && m.problems.is_empty());
    }
}

#[test]
fn a_row_whose_legs_do_not_net_to_it_is_kept_with_a_problem() {
    let rec = replay(&fixtures());
    let row = rec.source.rows.iter().find(|r| text(r, "type") == Some("OPTIONS_MULTILEG") && text(r, "unifiedStatus") == Some("COMPLETED")).unwrap().clone();
    // the order with one leg's cash changed by hand
    let edited = json::parse(&std::fs::read_to_string(fixtures().join("wrong-meaning-multileg-legs-not-the-row.json")).unwrap()).unwrap();
    let dir = tempfile::tempdir().unwrap();
    for e in std::fs::read_dir(fixtures()).unwrap() {
        let p = e.unwrap().path();
        let name = p.file_name().unwrap().to_string_lossy().into_owned();
        if p.is_file() && !name.starts_with("multileg-") {
            std::fs::copy(&p, dir.path().join(&name)).unwrap();
        }
    }
    std::fs::write(dir.path().join("multileg-edited.json"), edited.canonical()).unwrap();
    let mut rec = replay(dir.path());
    let m = map_row(&mut rec, &row);
    assert!(m.legs.is_empty());
    assert_eq!(m.problems.iter().map(|p| p.code.as_str()).collect::<Vec<_>>(), vec!["legs-disagree"]);
}

#[test]
fn a_reply_of_another_shape_is_unreadable_and_named() {
    let v = json::parse(&std::fs::read_to_string(fixtures().join("wrong-shape-activity-amount-as-number.json")).unwrap()).unwrap();
    let row = Node::root(&v).obj("data").unwrap().obj("activityFeedItems").unwrap().list("edges").unwrap()[0].obj("node").unwrap().value().clone();
    let mut rec = replay(&fixtures());
    let m = map_row(&mut rec, &row);
    assert!(m.legs.is_empty());
    assert_eq!(m.problems[0].code, "unreadable");
    assert!(m.problems[0].detail.contains("amount"), "{}", m.problems[0].detail);
}

#[test]
fn a_consolidation_moves_the_units_stated_onto_the_security_the_positions_show() {
    let (row, m) = one(&all(), "CORPORATE_ACTION", Some("CONSOLIDATION"), "COMPLETED");
    let old = text(&row, "securityId").unwrap();
    let given = m.legs.iter().find(|d| d.quantity.is_some_and(|q| q.is_negative())).unwrap();
    let got = m.legs.iter().find(|d| d.quantity.is_some_and(|q| q.is_positive())).unwrap();
    assert_eq!((security(given), given.quantity), (old, Some(dec("-177"))));
    assert_ne!(security(got), old);
    assert_eq!(got.quantity, Some(dec("35.4")));
    // the whole cost continues, at the ratio the units state exactly
    let a = &m.adjustments[0];
    assert_eq!(a.legs[0].units_per_unit, Some(dec("0.2")));
    assert_eq!(a.legs[0].cost_share, Some(Dec::ONE));
    assert_eq!(a.legs[0].to.as_ref().unwrap()[0].value, security(got));
}

#[test]
fn a_change_of_code_continues_the_holding_under_the_new_security() {
    let (row, m) = one(&all(), "CORPORATE_ACTION", Some("INTERNATIONAL_CODE_CHANGE"), "COMPLETED");
    let got = m.legs.iter().find(|d| d.quantity.is_some_and(|q| q.is_positive())).unwrap();
    assert_ne!(security(got), text(&row, "securityId").unwrap());
    assert_eq!(got.quantity, Some(dec("1800")));
    assert_eq!(m.adjustments[0].legs[0].units_per_unit, Some(Dec::ONE));
}

#[test]
fn a_conversion_is_both_its_sides_where_the_detail_states_them() {
    let rows = all();
    let convs: Vec<&(Value, Mapped)> = rows.iter().filter(|(r, _)| text(r, "type") == Some("FUNDS_CONVERSION")).collect();
    let both = convs.iter().find(|(_, m)| m.legs.len() == 2).unwrap();
    let cur: Vec<Currency> = both.1.legs.iter().map(|d| d.cash.unwrap().currency).collect();
    assert_ne!(cur[0], cur[1]);
    assert!(both.1.legs.iter().any(|d| d.cash.unwrap().amount.is_negative()) && both.1.legs.iter().any(|d| d.cash.unwrap().amount.is_positive()));
    let one_side = convs.iter().find(|(_, m)| m.legs.len() == 1).unwrap();
    assert_eq!(one_side.1.problems[0].code, "conversion-side-unstated");
}

#[test]
fn a_move_between_accounts_that_moved_no_holdings_is_its_stated_cash() {
    let rows = all();
    for (row, m) in rows.iter().filter(|(r, _)| text(r, "type") == Some("ASSET_MOVEMENT")) {
        let amount = Dec::parse(text(row, "amount").unwrap()).unwrap();
        assert_eq!(m.legs.len(), 1, "{:?}", m.problems);
        let c = m.legs[0].cash.unwrap().amount;
        match text(row, "subType") {
            Some("SOURCE") => assert_eq!((m.legs[0].kind, c), (Kind::TransferOut, amount.neg())),
            _ => assert_eq!((m.legs[0].kind, c), (Kind::TransferIn, amount)),
        }
    }
}

#[test]
fn an_assignment_pays_or_receives_by_the_contract_s_right() {
    let (_, m) = one(&all(), "OPTIONS_ASSIGN", Some("AUTO_ASSIGN"), "COMPLETED");
    let d = &m.legs[0];
    let right = d.instrument.as_ref().unwrap().option.as_ref().unwrap().right;
    // a call written and assigned: the shares go at the strike, cash received
    assert_eq!(right, bagholder_core::instrument::OptionRight::Call);
    assert!(d.cash.unwrap().amount.is_positive());
    assert_eq!(d.quantity, Some(dec("10")));
}

#[test]
fn an_expiry_takes_a_long_out_and_brings_a_short_back() {
    let rows = all();
    let (_, long) = one(&rows, "OPTIONS_EXPIRY", None, "COMPLETED");
    let (_, short) = one(&rows, "OPTIONS_SHORT_EXPIRY", None, "COMPLETED");
    assert!(long.legs[0].quantity.unwrap().is_negative());
    assert!(short.legs[0].quantity.unwrap().is_positive());
    assert!(long.legs[0].cash.is_none() && short.legs[0].cash.is_none());
}

#[test]
fn a_move_whose_row_states_no_amount_moves_what_both_accounts_net_deposits_show() {
    // "full in kind" moves of accounts sold to cash first: neither the row nor
    // its detail states an amount, and the cash moves days after the row, on
    // the day one account's net deposits fall by what the other's rise
    let rows = all();
    let moves: Vec<&(Value, Mapped)> = rows.iter().filter(|(r, _)| text(r, "type") == Some("INTERNAL_TRANSFER") && text(r, "amount").is_none() && text(r, "unifiedStatus") == Some("COMPLETED")).collect();
    assert_eq!(moves.len(), 4);
    let mut seen = std::collections::BTreeMap::new();
    for (row, m) in &moves {
        assert!(m.problems.is_empty(), "{:?}", m.problems);
        let [leg] = m.legs.as_slice() else { panic!("{:?}", m.legs) };
        let cash = leg.cash.expect("the cash moved");
        assert!(leg.trade_date > day_of(row), "moved {} for a row of {}", leg.trade_date, day_of(row));
        match text(row, "subType") {
            Some("SOURCE") => assert_eq!(leg.kind, Kind::TransferOut),
            _ => assert_eq!(leg.kind, Kind::TransferIn),
        }
        *seen.entry((text(row, "externalCanonicalId").unwrap().to_string(), leg.trade_date)).or_insert(Dec::ZERO) = seen.get(&(text(row, "externalCanonicalId").unwrap().to_string(), leg.trade_date)).copied().unwrap_or(Dec::ZERO).checked_add(cash.amount).unwrap();
        assert!(cash.amount.abs() == dec("9307.07") || cash.amount.abs() == dec("6861.21"), "{cash:?}");
    }
    // each move's two sides on one day, one the other's opposite
    assert_eq!(seen.len(), 2);
    assert!(seen.values().all(|v| v.is_zero()));
}

#[test]
fn a_transfer_from_another_institution_books_what_arrived_on_the_day_it_completed() {
    // the row states the 80,807.80 asked for; the detail states no value
    // arrived; the account's positions show 80,650.30 arriving by the day the
    // detail says the transfer completed
    let rows = all();
    let (_, m) = rows.iter().find(|(r, _)| text(r, "type") == Some("INSTITUTIONAL_TRANSFER_INTENT") && text(r, "amount") == Some("80807.8")).cloned().expect("the transfer");
    assert!(m.problems.is_empty(), "{:?}", m.problems);
    let [leg] = m.legs.as_slice() else { panic!("{:?}", m.legs) };
    assert_eq!(leg.kind, Kind::TransferIn);
    assert_eq!(leg.cash, Some(bagholder_core::Money::new(dec("80650.3"), Currency::CAD)));
    assert_eq!(leg.trade_date, "2023-09-15".parse().unwrap());
    // one whose detail was not read waits on it, named, and moves nothing
    let (_, unread) = rows.iter().find(|(r, _)| text(r, "type") == Some("INSTITUTIONAL_TRANSFER_INTENT") && text(r, "amount") != Some("80807.8") && text(r, "unifiedStatus") == Some("COMPLETED")).cloned().expect("another transfer");
    assert_eq!(unread.problems.iter().map(|p| p.code.as_str()).collect::<Vec<_>>(), vec!["transfer-arrival-unstated"]);
    assert!(unread.legs.iter().all(|l| l.cash.is_none() && l.quantity.is_none()));
}

#[test]
fn tax_withheld_from_a_withdrawal_is_part_of_its_gross_amount() {
    // the registered account's row states the gross 2,347.33; the tax row
    // 234.73; the account paid into receives 2,112.60
    let rows = all();
    let (tax_row, tax) = rows.iter().find(|(r, _)| text(r, "type") == Some("WITHHOLDING_TAX") && text(r, "amount") == Some("234.73")).cloned().expect("the tax row");
    let id = text(&tax_row, "externalCanonicalId").unwrap();
    let side = |sub: &str| rows.iter().find(|(r, _)| text(r, "externalCanonicalId") == Some(id) && text(r, "subType") == Some(sub)).cloned().unwrap().1;
    let cash = |m: &Mapped| m.legs.iter().map(|l| l.cash.unwrap().amount).fold(Dec::ZERO, |a, b| a.checked_add(b).unwrap());
    assert_eq!(cash(&side("SOURCE")), dec("-2112.60"));
    assert_eq!(cash(&side("DESTINATION")), dec("2112.60"));
    assert_eq!(cash(&tax), dec("-234.73"));
}

#[test]
fn a_credit_card_s_balance_is_what_is_owed_on_it() {
    let v = json::parse(&std::fs::read_to_string(fixtures().join("credit-card-account-1.json")).unwrap()).unwrap();
    let card = Node::root(&v).obj("data").unwrap().obj("creditCardAccount").unwrap().value().clone();
    assert_eq!(bagholder_wealthsimple::read::card_balance(&card, "anon-ca-4").unwrap(), dec("5140.28"));
}
