//! The engine held to `SPEC.md` by cases written from the spec
//! (`docs/plans/stage-2-engine.md`, "Cases written from the spec").
//!
//! Each file in `tests/cases` is a small book, what the market and the facts
//! say, and the figures the spec requires, worked out by hand from the
//! definitions with the working written beside them (`working`), never produced
//! by running an engine. Ids are short labels the runner turns into Bagholder's
//! ids. An expected amount is compared at the places it is written to (`"185.00"`
//! against the engine's figure rounded half to even to two places); a figure
//! the engine cannot state is expected as its gaps (`{"gaps": ["rate-pending"]}`).

mod common;

use std::collections::BTreeSet;
use std::path::Path;

use serde_json::Value;

use bagholder_core::{Dec, InstrumentId, Money, Rounding, TradeId};
use bagholder_engine::gap::{Fig, Gaps};
use bagholder_engine::ledger::{Direction, TripKey};
use bagholder_engine::scope::{Dates, Filters, Preset};
use bagholder_engine::trades::TradeKey;
use bagholder_engine::Engine;
use common::*;

struct Check<'a> {
    case: &'a str,
    failures: Vec<String>,
}

impl Check<'_> {
    fn fail(&mut self, what: String) {
        self.failures.push(format!("{}: {what}", self.case));
    }

    /// An expected figure: a written amount, or `{"gaps": [...]}`.
    fn money(&mut self, what: &str, expect: &Value, got: &Fig<Money>) {
        self.figure(what, expect, &got.clone().map(|m| m.amount));
    }

    fn figure(&mut self, what: &str, expect: &Value, got: &Fig<Dec>) {
        match (expect, got) {
            (Value::String(e), Ok(g)) => {
                let want = dec(e);
                let have = g.round(places(e), Rounding::HalfEven);
                if have != want {
                    self.fail(format!("{what}: expected {e}, got {} (exactly {})", have.to_text(), g.to_text()));
                }
            }
            (Value::Object(o), got) => {
                let want: BTreeSet<String> = o.get("gaps").and_then(Value::as_array).map(|a| a.iter().map(|x| x.as_str().unwrap().to_string()).collect()).unwrap_or_default();
                let have: BTreeSet<String> = match got {
                    Err(g) => g.words().into_iter().map(str::to_string).collect(),
                    Ok(v) => {
                        self.fail(format!("{what}: expected gaps {want:?}, got {}", v.to_text()));
                        return;
                    }
                };
                if have != want {
                    self.fail(format!("{what}: expected gaps {want:?}, got {have:?}"));
                }
            }
            (Value::String(e), Err(g)) => self.fail(format!("{what}: expected {e}, got gaps {:?}", g.words())),
            (e, _) => self.fail(format!("{what}: an expectation the runner does not read: {e}")),
        }
    }

    fn ratio(&mut self, what: &str, expect: &Value, got: Option<f64>) {
        match (expect.as_f64(), got) {
            (Some(e), Some(g)) if (e - g).abs() < 1e-9 => {}
            (None, None) if expect.is_null() => {}
            (e, g) => self.fail(format!("{what}: expected {e:?}, got {g:?}")),
        }
    }

    fn words(&mut self, what: &str, expect: &Value, got: BTreeSet<String>) {
        let want: BTreeSet<String> = expect.as_array().unwrap().iter().map(|x| x.as_str().unwrap().to_string()).collect();
        if want != got {
            self.fail(format!("{what}: expected {want:?}, got {got:?}"));
        }
    }
}

fn gaps_words(g: &Gaps) -> BTreeSet<String> {
    g.words().into_iter().map(str::to_string).collect()
}

fn run(path: &Path) -> Vec<String> {
    let text = std::fs::read_to_string(path).unwrap();
    let file: Value = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut failures = Vec::new();
    for case in file.as_array().expect("a file of cases") {
        let name = format!("{} / {}", path.file_name().unwrap().to_string_lossy(), s(case, "name").unwrap());
        assert!(!arr(case, "working").is_empty(), "{name}: a case says how its figures were worked out");
        let mut b = build(case);
        let e = engine(&mut b);
        let f = e.figures();
        let mut c = Check { case: &name, failures: vec![] };
        let expect = case.get("expect").cloned().unwrap_or(Value::Null);
        let tx = &b.tx;
        let find_trade = |label: &str, instrument: Option<InstrumentId>| {
            let id = &tx[label];
            f.trades.iter().find(|t| match &t.key {
                TradeKey::Trip(k) => &k.opening == id && instrument.is_none_or(|i| k.instrument == i),
                TradeKey::Group(_) => false,
            })
        };
        if let Some(n) = expect.get("trade_count") {
            if f.trades.len() as u64 != n.as_u64().unwrap() {
                c.fail(format!("trade count: expected {n}, got {}", f.trades.len()));
            }
        }
        for want in arr(&expect, "trades") {
            let instrument = s(&want, "instrument").map(|i| b.ids.instrument(i));
            let label = s(&want, "opened_by").unwrap();
            let Some(t) = find_trade(label, instrument) else {
                c.fail(format!("no trade opened by {label}"));
                continue;
            };
            let w = |k: &str| format!("trade {label} {k}");
            if let Some(v) = want.get("qty") {
                c.figure(&w("qty"), v, &Ok(t.qty));
            }
            if let Some(v) = want.get("entry") {
                c.figure(&w("entry"), v, &t.entry);
            }
            if let Some(v) = want.get("exit") {
                c.figure(&w("exit"), v, &t.exit);
            }
            if let Some(v) = want.get("pnl") {
                c.money(&w("pnl"), v, &t.pnl);
            }
            if let Some(v) = want.get("pnl_cad") {
                c.money(&w("pnl_cad"), v, &t.pnl_cad);
            }
            if let Some(v) = want.get("fees") {
                c.money(&w("fees"), v, &Ok(t.fees));
            }
            if let Some(v) = want.get("pnl_pct") {
                c.ratio(&w("pnl_pct"), v, t.pnl_pct());
            }
            if let Some(v) = want.get("hold_days") {
                if t.hold_days != v.as_i64().unwrap() {
                    c.fail(format!("{}: expected {v}, got {}", w("hold_days"), t.hold_days));
                }
            }
            if let Some(v) = s(&want, "closed") {
                if t.closed_on != day(v) {
                    c.fail(format!("{}: expected {v}, got {}", w("closed"), t.closed_on));
                }
            }
            if let Some(v) = s(&want, "opened") {
                if t.opened_on != day(v) {
                    c.fail(format!("{}: expected {v}, got {}", w("opened"), t.opened_on));
                }
            }
            if let Some(v) = s(&want, "direction") {
                let d = if v == "short" { Direction::Short } else { Direction::Long };
                if t.direction != d {
                    c.fail(format!("{}: expected {v}", w("direction")));
                }
            }
            if let Some(v) = want.get("flags") {
                c.words(&w("flags"), v, t.flags.iter().map(|x| x.to_string()).collect());
            }
            if let Some(v) = s(&want, "named_after") {
                if t.instrument != b.ids.instrument(v) {
                    c.fail(format!("{}: expected {v}", w("named_after")));
                }
            }
            if let Some(v) = want.get("slices") {
                if t.slices.len() as u64 != v.as_u64().unwrap() {
                    c.fail(format!("{}: expected {v}, got {}", w("slices"), t.slices.len()));
                }
            }
            // a closed round trip whose fills all state their cash, and whose
            // lots were never moved at cost: its P&L is its fills' cash, exactly
            if want.get("cash_invariant").and_then(Value::as_bool) == Some(true) {
                let cash = b.inputs.ledger.transactions.iter().filter(|x| t.fills.contains(&x.id)).try_fold(Dec::ZERO, |a, x| a.checked_add(x.cash.expect("the case states the cash").amount));
                match (&t.pnl, cash) {
                    (Ok(p), Ok(c2)) if p.amount == c2 => {}
                    (p, c2) => c.fail(format!("{}: P&L {:?} is not the fills' cash {:?}", w("cash invariant"), p, c2)),
                }
            }
        }
        if let Some(n) = expect.get("position_count") {
            if f.positions.len() as u64 != n.as_u64().unwrap() {
                c.fail(format!("position count: expected {n}, got {}", f.positions.len()));
            }
        }
        for want in arr(&expect, "positions") {
            let account = b.ids.account(s(&want, "account").unwrap());
            let instrument = b.ids.instrument(s(&want, "instrument").unwrap());
            let direction = if s(&want, "direction") == Some("short") { Direction::Short } else { Direction::Long };
            let label = format!("position {} {}", s(&want, "account").unwrap(), s(&want, "instrument").unwrap());
            let Some(p) = f.positions.iter().find(|p| p.account == account && p.instrument == instrument && p.direction == direction) else {
                c.fail(format!("no {label}"));
                continue;
            };
            let w = |k: &str| format!("{label} {k}");
            if let Some(v) = want.get("qty") {
                c.figure(&w("qty"), v, &Ok(p.qty));
            }
            if let Some(v) = want.get("book") {
                c.money(&w("book"), v, &p.book);
            }
            if let Some(v) = want.get("avg") {
                c.figure(&w("avg"), v, &p.avg);
            }
            if let Some(v) = want.get("market") {
                c.money(&w("market"), v, &p.market);
            }
            if let Some(v) = want.get("unrealized") {
                c.money(&w("unrealized"), v, &p.unrealized);
            }
            if let Some(v) = want.get("market_cad") {
                c.money(&w("market_cad"), v, &p.market_cad);
            }
            if let Some(v) = want.get("gaps") {
                c.words(&w("gaps"), v, gaps_words(&p.gaps));
            }
            if let Some(v) = want.get("held_days") {
                if p.held_days != v.as_i64().unwrap() {
                    c.fail(format!("{}: expected {v}, got {}", w("held_days"), p.held_days));
                }
            }
            if let Some(v) = s(&want, "opened_by") {
                if p.key != (TripKey { opening: tx[v].clone(), instrument }) && p.key.opening != tx[v] {
                    c.fail(format!("{}: expected the round trip opened by {v}", w("key")));
                }
            }
            if let Some(v) = want.get("flags") {
                c.words(&w("flags"), v, p.flags.iter().map(|x| x.to_string()).collect());
            }
        }
        for want in arr(&expect, "cash") {
            let label = s(&want, "tx").unwrap();
            let Some(r) = f.cash.iter().find(|r| r.id == tx[label]) else {
                if want.get("absent").and_then(Value::as_bool) != Some(true) {
                    c.fail(format!("no cashflow row for {label}"));
                }
                continue;
            };
            if want.get("absent").and_then(Value::as_bool) == Some(true) {
                c.fail(format!("cashflow row {label}: expected none"));
            }
            if let Some(v) = want.get("amount_cad") {
                c.money(&format!("cash {label} amount_cad"), v, &r.amount_cad);
            }
        }
        for want in arr(&expect, "payers") {
            let i = b.ids.instrument(s(&want, "instrument").unwrap());
            let Some(p) = f.payers.get(&i) else {
                c.fail(format!("no payer {}", s(&want, "instrument").unwrap()));
                continue;
            };
            let w = |k: &str| format!("payer {} {k}", s(&want, "instrument").unwrap());
            if let Some(v) = want.get("per") {
                c.money(&w("per"), v, &p.per);
            }
            if let Some(v) = want.get("per_year") {
                match (v, &p.per_year) {
                    (Value::Number(n), Ok(g)) if n.as_u64() == Some(*g as u64) => {}
                    (Value::Object(_), Err(_)) => c.figure(&w("per_year"), v, &p.per_year.clone().map(|n| Dec::from_int(n as i64))),
                    _ => c.fail(format!("{}: expected {v}, got {:?}", w("per_year"), p.per_year)),
                }
            }
            if let Some(v) = s(&want, "next_ex") {
                if p.next_ex != Some(day(v)) {
                    c.fail(format!("{}: expected {v}, got {:?}", w("next_ex"), p.next_ex));
                }
            }
        }
        for want in arr(&expect, "rates") {
            let currency = ccy(s(&want, "currency").unwrap());
            let d = day(s(&want, "day").unwrap());
            let got = if want.get("live").and_then(Value::as_bool) == Some(true) {
                bagholder_engine::fx::live_rate(&b.inputs.facts.rates, &b.inputs.clock, currency).map(|(r, _)| r)
            } else {
                bagholder_engine::fx::rate(&b.inputs.facts.rates, &b.inputs.clock, currency, d)
            };
            c.figure(&format!("rate {currency} {d}"), want.get("value").unwrap(), &got);
        }
        let scoped = |f: Filters| e.scope(&f);
        if let Some(k) = expect.get("kpi") {
            let filters = match s(k, "preset") {
                Some("ytd") => Filters { dates: Dates::Preset(Preset::YearToDate), ..Filters::default() },
                _ => Filters::default(),
            };
            let filters = Filters {
                accounts: arr(k, "accounts").iter().map(|a| b.ids.account(a.as_str().unwrap())).collect(),
                benchmark: s(k, "benchmark").unwrap_or("SP500").to_string(),
                ..filters
            };
            let sc = scoped(filters);
            if let Some(v) = k.get("realized") {
                c.money("kpi realized", v, &Ok(sc.kpi.realized));
            }
            for (field, got) in [("count", sc.kpi.count), ("left_out", sc.kpi.left_out), ("wins", sc.kpi.wins), ("losses", sc.kpi.losses), ("breakeven", sc.kpi.breakeven)] {
                if let Some(v) = k.get(field) {
                    if v.as_u64() != Some(got as u64) {
                        c.fail(format!("kpi {field}: expected {v}, got {got}"));
                    }
                }
            }
            if let Some(v) = k.get("win_rate") {
                c.ratio("kpi win_rate", v, sc.kpi.win_rate);
            }
            if let Some(v) = k.get("profit_factor") {
                c.ratio("kpi profit_factor", v, sc.kpi.profit_factor);
            }
            if let Some(v) = k.get("market_value") {
                c.money("portfolio market value", v, &Ok(sc.portfolio.market_value.total));
            }
            if let Some(v) = k.get("market_value_left_out") {
                if v.as_u64() != Some(sc.portfolio.market_value.left_out as u64) {
                    c.fail(format!("portfolio market value left out: expected {v}, got {}", sc.portfolio.market_value.left_out));
                }
            }
            if let Some(v) = k.get("dividends") {
                c.money("cashflow all-time dividends", v, &Ok(sc.cashflow.total.total));
            }
            for y in arr(k, "years") {
                let year = y.get("year").and_then(Value::as_i64).unwrap() as i16;
                let got = sc.equity.years.iter().find(|r| r.year == year);
                match got {
                    None => c.fail(format!("no yearly return for {year}")),
                    Some(r) => {
                        c.ratio(&format!("{year} return"), y.get("r").unwrap(), Some(r.r));
                        if let Some(v) = y.get("benchmark") {
                            c.ratio(&format!("{year} benchmark"), v, r.benchmark);
                        }
                        if let Some(v) = s(&y, "from") {
                            if r.from != day(v) {
                                c.fail(format!("{year} from: expected {v}, got {}", r.from));
                            }
                        }
                    }
                }
            }
            if let Some(v) = k.get("drawdown") {
                c.ratio("drawdown", v, sc.equity.drawdown.pct);
            }
            if let Some(v) = s(k, "drawdown_at") {
                if sc.equity.drawdown.at != Some(day(v)) {
                    c.fail(format!("drawdown at: expected {v}, got {:?}", sc.equity.drawdown.at));
                }
            }
        }
        for want in arr(&expect, "income") {
            let sc = e.scope(&Filters::default());
            let account = b.ids.account(s(&want, "account").unwrap());
            let instrument = b.ids.instrument(s(&want, "instrument").unwrap());
            let Some(h) = sc.cashflow.holdings.iter().find(|h| f.positions[h.position].account == account && f.positions[h.position].instrument == instrument) else {
                c.fail(format!("no income holding {} {}", s(&want, "account").unwrap(), s(&want, "instrument").unwrap()));
                continue;
            };
            if let Some(v) = want.get("all_time") {
                c.money("income all time", v, &Ok(h.all_time.total));
            }
            if let Some(v) = want.get("trailing_year") {
                c.money("income trailing year", v, &Ok(h.trailing_year.total));
            }
        }
        for want in arr(&expect, "checks") {
            let account = b.ids.account(s(&want, "account").unwrap());
            let Some(chk) = f.checks.iter().find(|x| x.account == account) else {
                c.fail(format!("no broker check for {}", s(&want, "account").unwrap()));
                continue;
            };
            let got = chk.differences.len() as u64;
            if Some(got) != want.get("differences").and_then(Value::as_u64) {
                c.fail(format!("broker check differences: expected {:?}, got {:?}", want.get("differences"), chk.differences));
            }
            if let Some(p) = want.get("pending").and_then(Value::as_bool) {
                if chk.pending != p {
                    c.fail(format!("broker check pending: expected {p}"));
                }
            }
        }
        for want in arr(&expect, "equity") {
            let account = b.ids.account(s(&want, "account").unwrap());
            let d = day(s(&want, "day").unwrap());
            let Some(eq) = f.equity.get(&account) else {
                c.fail(format!("no equity for {}", s(&want, "account").unwrap()));
                continue;
            };
            let point = eq.points.iter().find(|p| p.day == d);
            let what = format!("equity {} {d}", s(&want, "account").unwrap());
            match (want.get("value"), point) {
                (Some(Value::Null), None) => {}
                (Some(Value::Null), Some(p)) => c.fail(format!("{what}: expected no value, got {}", p.value.to_text())),
                (Some(v), Some(p)) => {
                    c.figure(&what, v, &Ok(p.value));
                    if let Some(src) = s(&want, "source") {
                        let got = if p.source == bagholder_engine::equity::ValueSource::Own { "own" } else { "broker" };
                        if got != src {
                            c.fail(format!("{what}: expected source {src}, got {got}"));
                        }
                    }
                }
                (Some(v), None) => c.fail(format!("{what}: expected {v}, got no value")),
                (None, _) => {}
            }
            if let Some(v) = want.get("own_gaps") {
                match eq.own.get(&d) {
                    Some(Err(g)) => c.words(&format!("{what} own gaps"), v, gaps_words(g)),
                    other => c.fail(format!("{what}: expected own gaps {v}, got {other:?}")),
                }
            }
            if let Some(v) = want.get("return") {
                let r = eq.returns.iter().find(|(rd, _, _)| *rd == d).map(|(_, r, _)| *r);
                c.ratio(&format!("{what} return"), v, r);
            }
        }
        if let Some(v) = expect.get("needs_trade") {
            let fresh = Engine::build(b.inputs.clone());
            if fresh.identity().needs_trade.len() as u64 != v.as_u64().unwrap() {
                c.fail(format!("needs_trade: expected {v}, got {}", fresh.identity().needs_trade.len()));
            }
        }
        if let Some(v) = expect.get("unclaimed") {
            let fresh = Engine::build(b.inputs.clone());
            let got: BTreeSet<TradeId> = fresh.identity().unclaimed.iter().copied().collect();
            let want: BTreeSet<TradeId> = v.as_array().unwrap().iter().map(|p| b.ids.trade(p.as_str().unwrap())).collect();
            if got != want {
                c.fail(format!("unclaimed: expected {want:?}, got {got:?}"));
            }
        }
        if let Some(v) = expect.get("joined") {
            let fresh = Engine::build(b.inputs.clone());
            let got: BTreeSet<(TradeId, TradeId)> = fresh.identity().joined.iter().cloned().collect();
            let want: BTreeSet<(TradeId, TradeId)> = v.as_array().unwrap().iter().map(|p| (b.ids.trade(p[0].as_str().unwrap()), b.ids.trade(p[1].as_str().unwrap()))).collect();
            if got != want {
                c.fail(format!("joined: expected {want:?}, got {got:?}"));
            }
        }
        if let Some(v) = expect.get("trade_ids") {
            // a round trip opened by a transaction carries the trade the case names
            for (label, trade) in v.as_object().unwrap() {
                let found = f.trades.iter().chain(std::iter::empty()).find(|t| match &t.key {
                    TradeKey::Trip(k) => k.opening == tx[label],
                    _ => false,
                });
                let pos = f.positions.iter().find(|p| p.key.opening == tx[label]);
                let got = found.and_then(|t| t.trade).or(pos.and_then(|p| p.trade));
                if got != Some(b.ids.trade(trade.as_str().unwrap())) {
                    c.fail(format!("round trip opened by {label}: expected trade {trade}, got {got:?}"));
                }
            }
        }
        if let Some(v) = expect.get("beyond") {
            let got: BTreeSet<String> = f.matched.beyond.iter().map(|x| tx.iter().find(|(_, id)| **id == x.transaction).map(|(l, _)| l.clone()).unwrap_or_default()).collect();
            c.words("beyond held", v, got);
        }
        failures.extend(c.failures);
    }
    failures
}

#[test]
fn every_case_gives_the_figures_the_spec_requires() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/cases");
    let mut files: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().is_some_and(|x| x == "json")).collect();
    files.sort();
    assert!(!files.is_empty(), "no cases in {}", dir.display());
    let failures: Vec<String> = files.iter().flat_map(|p| run(p)).collect();
    assert!(failures.is_empty(), "{} failures:\n{}", failures.len(), failures.join("\n"));
}
