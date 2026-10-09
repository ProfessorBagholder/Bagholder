//! A made-up Wealthsimple of any size, answering as the real one does: every
//! reply is one of the recorded replies (`tests/replies`) with its fields set,
//! so the adapter, the mapping and the pull read it exactly as they read the
//! network, and a book built from it is what the app writes (`docs/plans/
//! stage-p-capacity-and-process.md`, part A). Nothing in it is anyone's: the
//! listings are generated names on generated ids, in every currency the
//! accounts hold.
//!
//! The same seed and size give the same replies. Cash never goes below
//! nothing: a purchase the account cannot pay for is funded first, by a
//! deposit (and a conversion for another currency), as a person would.

use std::collections::BTreeMap;

use bagholder_broker::{Answer, Failure};
use bagholder_core::json::{self, Value};
use bagholder_core::Dec;
use bagholder_sources::reply::Node;

use crate::adapter::Source;

/// How big a book to make.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Size {
    /// Accounts, of every kind Wealthsimple has: registered and not, crypto,
    /// spending and a credit card.
    pub accounts: usize,
    /// Listings traded: shares on both sides of the border and coins; each
    /// option round trip adds its own contract.
    pub instruments: usize,
    /// Round trips opened (some still open today).
    pub trades: usize,
    /// How far back the activity starts.
    pub months: u32,
    pub seed: u64,
}

const ACTIVITY: &str = include_str!("../tests/replies/wealthsimple/edited-activity-one-per-kind.json");
const ACCOUNT: &str = include_str!("../tests/replies/wealthsimple-pull/edited-accounts-one.json");
const SECURITIES: &str = include_str!("../tests/replies/wealthsimple/edited-securities.json");
const BALANCES: &str = include_str!("../tests/replies/wealthsimple-pull/edited-balances.json");
const HISTORY: &str = include_str!("../tests/replies/wealthsimple-pull/history-1.json");
const POSITIONS: &str = include_str!("../tests/replies/wealthsimple/positions@anon-resp-2@2025-07-05.json");
const CONVERSION: &str = include_str!("../tests/replies/wealthsimple/FetchInternalTransfer-1.json");
const CARD: &str = include_str!("../tests/replies/wealthsimple/credit-card-account-1.json");

/// The account types, in the order a book of more accounts adds them: the
/// investment accounts first, then crypto, spending and a card.
const TYPES: [&str; 12] = [
    "SELF_DIRECTED_NON_REGISTERED_MARGIN",
    "SELF_DIRECTED_TFSA",
    "SELF_DIRECTED_RRSP",
    "SELF_DIRECTED_CRYPTO",
    "CASH",
    "CREDIT_CARD",
    "SELF_DIRECTED_FHSA",
    "SELF_DIRECTED_NON_REGISTERED",
    "SELF_DIRECTED_RESP_FAMILY",
    "SELF_DIRECTED_LIRA",
    "SELF_DIRECTED_RRIF",
    "SELF_DIRECTED_JOINT_NON_REGISTERED_MARGIN",
];

/// splitmix64: the same seed, the same book.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    /// 0..n
    fn below(&mut self, n: u64) -> u64 {
        if n == 0 {
            0
        } else {
            self.next() % n
        }
    }
    fn range(&mut self, lo: i64, hi: i64) -> i64 {
        lo + self.below((hi - lo + 1).max(1) as u64) as i64
    }
    fn chance(&mut self, per_thousand: u64) -> bool {
        self.below(1000) < per_thousand
    }
    fn hex(&mut self) -> String {
        format!("{:016x}{:016x}", self.next(), self.next())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Cur {
    Cad,
    Usd,
}

impl Cur {
    fn code(self) -> &'static str {
        match self {
            Cur::Cad => "CAD",
            Cur::Usd => "USD",
        }
    }
    fn cash_id(self) -> &'static str {
        match self {
            Cur::Cad => "sec-c-cad",
            Cur::Usd => "sec-c-usd",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Sort {
    Share,
    Coin,
    Contract,
}

/// A listing: shares, a coin or an option contract.
#[derive(Clone, Debug)]
struct Listing {
    id: String,
    symbol: String,
    sort: Sort,
    cur: Cur,
    /// A unit's price in cents (a coin's per whole coin; a contract's premium per share).
    price: i64,
    /// A contract's underlying and terms.
    contract: Option<(String, jiff::civil::Date, i64)>,
}

impl Listing {
    /// Units are whole for shares and contracts, millionths for a coin.
    fn scale(&self) -> u32 {
        if self.sort == Sort::Coin {
            6
        } else {
            0
        }
    }
    /// What `units` (in the listing's own scale) cost at `price` cents, in cents.
    fn cost(&self, units: i64, price: i64) -> i64 {
        match self.sort {
            Sort::Share => units * price,
            Sort::Contract => units * price * 100,
            Sort::Coin => ((units as i128 * price as i128) / 1_000_000) as i64,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Role {
    /// Trades shares and options, holds CAD and USD.
    Invest,
    Crypto,
    Spending,
    Card,
}

#[derive(Clone, Debug)]
struct Acct {
    id: String,
    custodian: String,
    ty: &'static str,
    role: Role,
    /// Cash in cents, per currency; a card's is what it owes, negative.
    cash: BTreeMap<Cur, i64>,
    /// Units held, per listing id, in the listing's scale.
    held: BTreeMap<String, i64>,
    /// Each day's units at its end, for the positions asked of a day: per
    /// listing id (cash too, by its `sec-c-` id), the day it changed and to what.
    timeline: BTreeMap<String, Vec<(jiff::civil::Date, i64)>>,
    /// Net deposits in cents, CAD, by the day they changed.
    deposits: Vec<(jiff::civil::Date, i64)>,
    opened: jiff::civil::Date,
}

/// One planned piece of activity, in the order it happens.
#[derive(Clone, Debug)]
enum Plan {
    Buy { account: usize, listing: usize, units: i64 },
    Sell { account: usize, listing: usize, all: bool },
    Expire { account: usize, listing: usize },
    Dividend { account: usize, listing: usize },
    Staking { account: usize, listing: usize },
    Move { from: usize, to: usize },
    MoveHolding { from: usize, to: usize },
    Deposit { account: usize, cents: i64 },
    Withdraw { account: usize },
    Interest { account: usize },
    CardPurchase { account: usize, cents: i64 },
    CardPayment { card: usize, from: usize },
}

/// The made-up Wealthsimple.
pub struct Generated {
    today: jiff::civil::Date,
    accounts: Vec<Acct>,
    listings: Vec<Listing>,
    rows: Vec<Value>,
    conversions: BTreeMap<String, Value>,
    templates: Templates,
    asked: usize,
}

struct Templates {
    rows: BTreeMap<(String, String), Value>,
    account: Value,
    securities: BTreeMap<&'static str, Value>,
    balances: Value,
    history: Value,
    position: Value,
    conversion: Value,
    card: Value,
}

fn parse(text: &str) -> Value {
    json::parse(text).expect("a recorded reply is JSON")
}

fn data(v: &Value) -> Node<'_> {
    Node::root(v).obj("data").expect("a recorded reply has data")
}

impl Templates {
    fn read() -> Templates {
        let activity = parse(ACTIVITY);
        let mut rows = BTreeMap::new();
        for e in data(&activity).obj("activityFeedItems").and_then(|f| f.list("edges")).expect("the activity's edges") {
            let n = e.obj("node").expect("an edge's node");
            let key = (n.text("type").expect("a type").to_string(), n.opt_text("subType").expect("a subtype").unwrap_or("").to_string());
            rows.entry(key).or_insert_with(|| n.value().clone());
        }
        let accounts = parse(ACCOUNT);
        let account = data(&accounts).obj("identity").and_then(|i| i.obj("accounts")).and_then(|a| a.list("edges")).expect("the accounts")[0].obj("node").expect("an account").value().clone();
        let secs = parse(SECURITIES);
        let mut securities = BTreeMap::new();
        for s in data(&secs).list("securities").expect("securities") {
            let ty = match s.text("securityType").expect("a type") {
                "EQUITY" => "share",
                "OPTION" => "contract",
                "CRYPTOCURRENCY" => "coin",
                _ => continue,
            };
            securities.entry(ty).or_insert_with(|| s.value().clone());
        }
        let bal = parse(BALANCES);
        let balances = data(&bal).list("accounts").expect("balances")[0].value().clone();
        let hist = parse(HISTORY);
        let history = data(&hist).obj("account").and_then(|a| a.obj("financials")).and_then(|f| f.obj("historicalDaily")).and_then(|h| h.list("edges")).expect("history")[0].obj("node").expect("a day").value().clone();
        let pos = parse(POSITIONS);
        let position = data(&pos)
            .list("accounts")
            .expect("accounts")[0]
            .obj("financials")
            .and_then(|f| f.obj("current"))
            .and_then(|c| c.obj("positionsAsOfDate"))
            .and_then(|p| p.list("edges"))
            .expect("positions")
            .into_iter()
            .find(|e| e.obj("node").and_then(|n| n.obj("security")).and_then(|s| s.text("id")).is_ok_and(|id| id.starts_with("sec-s-")))
            .expect("a fund's position")
            .obj("node")
            .expect("a node")
            .value()
            .clone();
        let conv = parse(CONVERSION);
        let conversion = data(&conv).obj("internalTransfer").expect("a transfer").value().clone();
        let card = parse(CARD);
        let card = data(&card).obj("creditCardAccount").expect("a card").value().clone();
        Templates { rows, account, securities, balances, history, position, conversion, card }
    }

    fn row(&self, ty: &str, sub: &str) -> Value {
        self.rows.get(&(ty.to_string(), sub.to_string())).unwrap_or_else(|| panic!("a recorded {ty} {sub} row")).clone()
    }
}

/// `v` with each field set (a path of keys into nested objects).
fn set(mut v: Value, fields: &[(&[&str], Value)]) -> Value {
    for (path, value) in fields {
        let mut at = &mut v;
        for (i, key) in path.iter().enumerate() {
            let Value::Object(m) = at else { panic!("{key} is not in an object") };
            if i + 1 == path.len() {
                m.insert(key.to_string(), value.clone());
                break;
            }
            at = m.entry(key.to_string()).or_insert_with(|| Value::Object(BTreeMap::new()));
        }
    }
    v
}

fn text(s: impl Into<String>) -> Value {
    Value::String(s.into())
}

/// Cents as decimal text.
fn money(cents: i64) -> String {
    Dec::new(cents, 2).expect("cents fit").to_text()
}

/// Units in a listing's scale as decimal text.
fn units_text(units: i64, scale: u32) -> String {
    Dec::new(units, scale).expect("units fit").to_text()
}

fn at(day: jiff::civil::Date, n: u32) -> String {
    // 15:00 UTC is the same day in Toronto all year; `n` keeps a day's rows in order
    format!("{day}T15:{:02}:{:02}.000000+00:00", (n / 60) % 60, n % 60)
}

impl Generated {
    pub fn new(size: Size, today: jiff::civil::Date) -> Generated {
        let templates = Templates::read();
        let mut r = Rng(size.seed);
        let start = today.checked_sub(jiff::Span::new().months(size.months as i64)).expect("a start");
        let accounts: Vec<Acct> = (0..size.accounts)
            .map(|i| {
                let ty = TYPES[i % TYPES.len()];
                let role = match ty {
                    "SELF_DIRECTED_CRYPTO" => Role::Crypto,
                    "CASH" => Role::Spending,
                    "CREDIT_CARD" => Role::Card,
                    _ => Role::Invest,
                };
                Acct { id: format!("gen-account-{i}"), custodian: format!("gen-custodian-{i}"), ty, role, cash: BTreeMap::new(), held: BTreeMap::new(), timeline: BTreeMap::new(), deposits: Vec::new(), opened: start }
            })
            .collect();
        // listings: shares on both sides of the border, and coins
        let mut listings = Vec::new();
        for i in 0..size.instruments.max(1) {
            let coin = i % 8 == 7;
            let cur = if coin || i % 2 == 0 { Cur::Cad } else { Cur::Usd };
            let symbol = name(i);
            let id = if coin { format!("sec-z-{}-{}", symbol.to_lowercase(), r.hex()) } else { format!("sec-s-{}", r.hex()) };
            let price = if coin { r.range(5, 60_000) * 100 } else { r.range(200, 40_000) };
            listings.push(Listing { id, symbol, sort: if coin { Sort::Coin } else { Sort::Share }, cur, price, contract: None });
        }
        let mut g = Generated { today, accounts, listings, rows: Vec::new(), conversions: BTreeMap::new(), templates, asked: 0 };
        let plans = g.plan(&mut r, size, start);
        g.run(&mut r, plans);
        g
    }

    fn of_role(&self, role: Role) -> Vec<usize> {
        (0..self.accounts.len()).filter(|i| self.accounts[*i].role == role).collect()
    }

    /// Every piece of activity, by day.
    fn plan(&mut self, r: &mut Rng, size: Size, start: jiff::civil::Date) -> BTreeMap<jiff::civil::Date, Vec<Plan>> {
        let days = (self.today - start).get_days().max(2) as i64;
        let day = |n: i64| start.checked_add(jiff::Span::new().days(n)).expect("a day");
        let mut out: BTreeMap<jiff::civil::Date, Vec<Plan>> = BTreeMap::new();
        let invest = self.of_role(Role::Invest);
        let crypto = self.of_role(Role::Crypto);
        let spending = self.of_role(Role::Spending);
        let cards = self.of_role(Role::Card);
        let shares: Vec<usize> = (0..self.listings.len()).filter(|i| self.listings[*i].sort == Sort::Share).collect();
        let coins: Vec<usize> = (0..self.listings.len()).filter(|i| self.listings[*i].sort == Sort::Coin).collect();
        for _ in 0..size.trades {
            let opened = r.range(1, days - 2);
            // a quarter still open today
            let closed = (!r.chance(250)).then(|| opened + r.range(1, 150)).filter(|c| *c < days);
            let kind = r.below(10);
            let (account, listing) = if kind == 0 && !crypto.is_empty() && !coins.is_empty() {
                (crypto[r.below(crypto.len() as u64) as usize], coins[r.below(coins.len() as u64) as usize])
            } else if !invest.is_empty() && !shares.is_empty() {
                let account = invest[r.below(invest.len() as u64) as usize];
                let under = shares[r.below(shares.len() as u64) as usize];
                if kind <= 2 && self.listings[under].cur == Cur::Usd {
                    // a contract of its own on a US listing, expiring after it opens
                    let expiry = day(opened + r.range(7, 90));
                    let u = self.listings[under].clone();
                    let strike = (u.price / 100 + r.range(-3, 3)).max(1) * 100;
                    self.listings.push(Listing { id: format!("sec-o-{}", r.hex()), symbol: u.symbol.clone(), sort: Sort::Contract, cur: Cur::Usd, price: r.range(5, 900), contract: Some((u.id.clone(), expiry, strike)) });
                    let contract = self.listings.len() - 1;
                    out.entry(day(opened)).or_default().push(Plan::Buy { account, listing: contract, units: r.range(1, 5) });
                    let exp = (expiry - day(0)).get_days() as i64;
                    match closed.filter(|c| *c < exp) {
                        Some(c) => out.entry(day(c)).or_default().push(Plan::Sell { account, listing: contract, all: true }),
                        None if exp < days => out.entry(expiry).or_default().push(Plan::Expire { account, listing: contract }),
                        None => {}
                    }
                    continue;
                }
                (account, under)
            } else {
                continue;
            };
            let l = &self.listings[listing];
            let units = match l.sort {
                Sort::Coin => r.range(1_000, 5_000_000),
                _ => r.range(1, 200),
            };
            out.entry(day(opened)).or_default().push(Plan::Buy { account, listing, units });
            if r.chance(300) {
                out.entry(day((opened + 1).min(days - 1))).or_default().push(Plan::Buy { account, listing, units: (units / 2).max(1) });
            }
            if let Some(c) = closed {
                if r.chance(300) && c > opened + 1 {
                    out.entry(day(c - 1)).or_default().push(Plan::Sell { account, listing, all: false });
                }
                out.entry(day(c)).or_default().push(Plan::Sell { account, listing, all: true });
            }
            // income while held
            let held_to = closed.unwrap_or(days - 1);
            let mut d = opened + 30;
            while d < held_to {
                let p = if l.sort == Sort::Coin { Plan::Staking { account, listing } } else { Plan::Dividend { account, listing } };
                out.entry(day(d)).or_default().push(p);
                d += if l.sort == Sort::Coin { 7 } else { 91 };
            }
        }
        // money in and out, between accounts, interest, and the card
        for d in 0..days {
            if invest.len() >= 2 && r.chance(30) {
                let from = invest[r.below(invest.len() as u64) as usize];
                let to = invest[r.below(invest.len() as u64) as usize];
                if from != to {
                    // a holding moved: within one tax class or across two, whichever the pair is
                    let p = if r.chance(200) { Plan::MoveHolding { from, to } } else { Plan::Move { from, to } };
                    out.entry(day(d)).or_default().push(p);
                }
            }
            for &s in &spending {
                if d % 14 == 0 {
                    out.entry(day(d)).or_default().push(Plan::Deposit { account: s, cents: r.range(150_000, 400_000) });
                }
                if r.chance(150) {
                    out.entry(day(d)).or_default().push(Plan::Withdraw { account: s });
                }
                if d % 30 == 29 {
                    out.entry(day(d)).or_default().push(Plan::Interest { account: s });
                }
            }
            for &c in &cards {
                for _ in 0..r.below(3) {
                    out.entry(day(d)).or_default().push(Plan::CardPurchase { account: c, cents: r.range(300, 25_000) });
                }
                if d % 30 == 15 {
                    if let Some(&from) = spending.first() {
                        out.entry(day(d)).or_default().push(Plan::CardPayment { card: c, from });
                    }
                }
            }
        }
        out
    }

    fn run(&mut self, r: &mut Rng, plans: BTreeMap<jiff::civil::Date, Vec<Plan>>) {
        let mut n: u32 = 0;
        for (day, list) in plans {
            for p in list {
                n += 1;
                self.apply(r, day, p, &mut n);
            }
        }
    }

    fn next_id(&self, kind: &str) -> String {
        format!("gen-{kind}-{}", self.rows.len() + self.conversions.len())
    }

    fn cash(&self, a: usize, c: Cur) -> i64 {
        self.accounts[a].cash.get(&c).copied().unwrap_or(0)
    }

    fn move_cash(&mut self, a: usize, c: Cur, cents: i64, day: jiff::civil::Date) {
        let acct = &mut self.accounts[a];
        let now = acct.cash.get(&c).copied().unwrap_or(0) + cents;
        acct.cash.insert(c, now);
        acct.timeline.entry(c.cash_id().to_string()).or_default().push((day, now));
    }

    fn move_units(&mut self, a: usize, listing: usize, units: i64, day: jiff::civil::Date) {
        let id = self.listings[listing].id.clone();
        let acct = &mut self.accounts[a];
        let now = acct.held.get(&id).copied().unwrap_or(0) + units;
        if now == 0 {
            acct.held.remove(&id);
        } else {
            acct.held.insert(id.clone(), now);
        }
        acct.timeline.entry(id).or_default().push((day, now));
    }

    fn net_deposit(&mut self, a: usize, cents: i64, day: jiff::civil::Date) {
        self.accounts[a].deposits.push((day, cents));
    }

    fn push(&mut self, row: Value) {
        self.rows.push(row);
    }

    /// A row of `ty`/`sub` from its recording, for an account, at an instant.
    fn row(&self, ty: &str, sub: &str, a: usize, when: String, id: &str) -> Value {
        set(
            self.templates.row(ty, sub),
            &[
                (&["accountId"], text(&self.accounts[a].id)),
                (&["canonicalId"], text(id)),
                (&["externalCanonicalId"], text(format!("{id}-x"))),
                (&["occurredAt"], text(when)),
                (&["unifiedStatus"], text("COMPLETED")),
            ],
        )
    }

    fn with_security(&self, row: Value, listing: usize) -> Value {
        let l = &self.listings[listing];
        let mut fields: Vec<(&[&str], Value)> = vec![(&["securityId"], text(&l.id)), (&["security", "id"], text(&l.id)), (&["security", "stock", "symbol"], text(&l.symbol)), (&["assetSymbol"], text(&l.symbol))];
        if let Some((_, expiry, strike)) = &l.contract {
            fields.push((&["expiryDate"], text(expiry.to_string())));
            fields.push((&["strikePrice"], text(money(*strike))));
            fields.push((&["contractType"], text("call")));
        }
        set(row, &fields)
    }

    /// Cash enough for `cents` of `c` in account `a`: a deposit of CAD, and
    /// for another currency a conversion from it, before what needs it.
    fn fund(&mut self, a: usize, c: Cur, cents: i64, day: jiff::civil::Date, n: &mut u32) {
        let short = cents - self.cash(a, c);
        if short <= 0 {
            return;
        }
        let want = short + 50_000;
        let cad = match c {
            Cur::Cad => want,
            // at 1.35 CAD a USD, the conversion's own rate
            Cur::Usd => want * 135 / 100 + 1,
        };
        *n += 1;
        let id = self.next_id("deposit");
        let row = set(self.row("DEPOSIT", "EFT", a, at(day, *n), &id), &[(&["amount"], text(money(cad))), (&["currency"], text("CAD")), (&["amountSign"], text("positive"))]);
        self.push(row);
        self.move_cash(a, Cur::Cad, cad, day);
        self.net_deposit(a, cad, day);
        if c == Cur::Usd {
            *n += 1;
            let id = self.next_id("conversion");
            let detail_id = format!("{id}-x");
            let row = set(self.row("FUNDS_CONVERSION", "", a, at(day, *n), &id), &[(&["amount"], text(money(want))), (&["currency"], text("USD")), (&["amountSign"], text("positive"))]);
            let detail = set(
                self.templates.conversion.clone(),
                &[
                    (&["id"], text(&detail_id)),
                    (&["amount"], text(money(cad))),
                    (&["currency"], text("CAD")),
                    (&["fxAdjustedAmount"], text(money(want))),
                    (&["fxRate"], text("0.740740")),
                    (&["source_account", "id"], text(&self.accounts[a].id)),
                    (&["source_account", "unifiedAccountType"], text(self.accounts[a].ty)),
                ],
            );
            self.conversions.insert(detail_id, detail);
            self.push(row);
            self.move_cash(a, Cur::Cad, -cad, day);
            self.move_cash(a, Cur::Usd, want, day);
        }
    }

    fn apply(&mut self, r: &mut Rng, day: jiff::civil::Date, p: Plan, n: &mut u32) {
        match p {
            Plan::Buy { account, listing, units } => {
                let l = self.listings[listing].clone();
                let price = (l.price + r.range(-l.price / 20, l.price / 20)).max(1);
                let cost = l.cost(units, price).max(1);
                self.fund(account, l.cur, cost, day, n);
                *n += 1;
                let id = self.next_id("buy");
                let (ty, sub) = match l.sort {
                    Sort::Share => ("DIY_BUY", "LIMIT_ORDER"),
                    Sort::Coin => ("CRYPTO_BUY", "MARKET_ORDER"),
                    Sort::Contract => ("OPTIONS_BUY", "LIMIT_ORDER"),
                };
                let row = self.row(ty, sub, account, at(day, *n), &id);
                let row = set(self.with_security(row, listing), &[(&["amount"], text(money(cost))), (&["currency"], text(l.cur.code())), (&["assetQuantity"], text(units_text(units, l.scale()))), (&["status"], text("FILLED"))]);
                self.push(row);
                self.move_cash(account, l.cur, -cost, day);
                self.move_units(account, listing, units, day);
            }
            Plan::Sell { account, listing, all } => {
                let l = self.listings[listing].clone();
                let held = self.accounts[account].held.get(&l.id).copied().unwrap_or(0);
                if held <= 0 {
                    return;
                }
                let units = if all { held } else { (held / 2).max(1) };
                let price = (l.price + r.range(-l.price / 5, l.price / 5)).max(1);
                let proceeds = l.cost(units, price);
                *n += 1;
                let id = self.next_id("sell");
                let (ty, sub) = match l.sort {
                    Sort::Share => ("DIY_SELL", "LIMIT_ORDER"),
                    Sort::Coin => ("CRYPTO_SELL", "MARKET_ORDER"),
                    Sort::Contract => ("OPTIONS_SELL", "LIMIT_ORDER"),
                };
                let row = self.row(ty, sub, account, at(day, *n), &id);
                let row = set(self.with_security(row, listing), &[(&["amount"], text(money(proceeds))), (&["currency"], text(l.cur.code())), (&["assetQuantity"], text(units_text(units, l.scale()))), (&["status"], text("FILLED"))]);
                self.push(row);
                self.move_cash(account, l.cur, proceeds, day);
                self.move_units(account, listing, -units, day);
            }
            Plan::Expire { account, listing } => {
                let held = self.accounts[account].held.get(&self.listings[listing].id).copied().unwrap_or(0);
                if held <= 0 {
                    return;
                }
                *n += 1;
                let id = self.next_id("expiry");
                let row = self.row("OPTIONS_EXPIRY", "", account, format!("{day}T21:30:00.000000+00:00"), &id);
                let row = set(self.with_security(row, listing), &[(&["assetQuantity"], text(held.to_string())), (&["currency"], text("USD"))]);
                self.push(row);
                self.move_units(account, listing, -held, day);
            }
            Plan::Dividend { account, listing } => {
                let l = self.listings[listing].clone();
                let held = self.accounts[account].held.get(&l.id).copied().unwrap_or(0);
                if held <= 0 {
                    return;
                }
                // about one percent a quarter
                let cents = (l.cost(held, l.price) / 100).max(1);
                *n += 1;
                let id = self.next_id("dividend");
                let row = self.row("DIVIDEND", "DIY_DIVIDEND", account, at(day, *n), &id);
                let row = set(self.with_security(row, listing), &[(&["amount"], text(money(cents))), (&["currency"], text(l.cur.code()))]);
                self.push(row);
                self.move_cash(account, l.cur, cents, day);
            }
            Plan::Staking { account, listing } => {
                let held = self.accounts[account].held.get(&self.listings[listing].id).copied().unwrap_or(0);
                if held <= 0 {
                    return;
                }
                let units = (held / 1000).max(1);
                *n += 1;
                let id = self.next_id("staking");
                let row = self.row("CRYPTO_STAKING_REWARD", "", account, at(day, *n), &id);
                let row = set(self.with_security(row, listing), &[(&["assetQuantity"], text(units_text(units, 6)))]);
                self.push(row);
                self.move_units(account, listing, units, day);
            }
            Plan::Move { from, to } => {
                let cents = self.cash(from, Cur::Cad) / 3;
                if cents < 100 {
                    return;
                }
                *n += 1;
                let id = self.next_id("move");
                let group = format!("{id}-x");
                let out = set(self.row("INTERNAL_TRANSFER", "SOURCE", from, at(day, *n), &format!("{id}-out")), &[(&["amount"], text(money(cents))), (&["amountSign"], text("negative")), (&["opposingAccountId"], text(&self.accounts[to].id)), (&["externalCanonicalId"], text(&group)), (&["groupId"], text(&group))]);
                let inn = set(self.row("INTERNAL_TRANSFER", "DESTINATION", to, at(day, *n), &format!("{id}-in")), &[(&["amount"], text(money(cents))), (&["amountSign"], text("positive")), (&["opposingAccountId"], text(&self.accounts[from].id)), (&["externalCanonicalId"], text(&group)), (&["groupId"], text(&group))]);
                self.push(out);
                self.push(inn);
                self.move_cash(from, Cur::Cad, -cents, day);
                self.move_cash(to, Cur::Cad, cents, day);
                self.net_deposit(from, -cents, day);
                self.net_deposit(to, cents, day);
            }
            Plan::MoveHolding { from, to } => {
                // one share holding moved whole, on a day no other move touches these accounts
                let Some((lid, units)) = self.accounts[from].held.iter().find(|(id, _)| id.starts_with("sec-s-")).map(|(i, u)| (i.clone(), *u)) else { return };
                let listing = self.listings.iter().position(|l| l.id == lid).expect("a listing held");
                let near = |d: jiff::civil::Date| self.rows.iter().any(|x| {
                    let r = Node::root(x);
                    matches!(r.text("type"), Ok("ASSET_MOVEMENT")) && r.text("occurredAt").ok().and_then(|t| t.get(..10)).and_then(|t| t.parse::<jiff::civil::Date>().ok()).is_some_and(|x| (x - d).get_days().abs() <= 2)
                });
                if near(day) {
                    return;
                }
                let l = self.listings[listing].clone();
                let worth = l.cost(units, l.price);
                *n += 1;
                let id = self.next_id("asset");
                let group = format!("{id}-x");
                let out = set(self.row("ASSET_MOVEMENT", "SOURCE", from, at(day, *n), &format!("{id}-out")), &[(&["amount"], text(money(worth))), (&["currency"], text(l.cur.code())), (&["amountSign"], text("negative")), (&["opposingAccountId"], text(&self.accounts[to].id)), (&["externalCanonicalId"], text(&group))]);
                let inn = set(self.row("ASSET_MOVEMENT", "DESTINATION", to, at(day, *n), &format!("{id}-in")), &[(&["amount"], text(money(worth))), (&["currency"], text(l.cur.code())), (&["amountSign"], text("positive")), (&["opposingAccountId"], text(&self.accounts[from].id)), (&["externalCanonicalId"], text(&group))]);
                self.push(out);
                self.push(inn);
                self.move_units(from, listing, -units, day);
                self.move_units(to, listing, units, day);
            }
            Plan::Deposit { account, cents } => {
                *n += 1;
                let id = self.next_id("deposit");
                let row = set(self.row("DEPOSIT", "EFT", account, at(day, *n), &id), &[(&["amount"], text(money(cents))), (&["currency"], text("CAD")), (&["amountSign"], text("positive"))]);
                self.push(row);
                self.move_cash(account, Cur::Cad, cents, day);
                self.net_deposit(account, cents, day);
            }
            Plan::Withdraw { account } => {
                let cents = self.cash(account, Cur::Cad) / 4;
                if cents < 100 {
                    return;
                }
                *n += 1;
                let id = self.next_id("withdrawal");
                let row = set(self.row("WITHDRAWAL", "EFT", account, at(day, *n), &id), &[(&["amount"], text(money(cents))), (&["currency"], text("CAD")), (&["amountSign"], text("negative"))]);
                self.push(row);
                self.move_cash(account, Cur::Cad, -cents, day);
                self.net_deposit(account, -cents, day);
            }
            Plan::Interest { account } => {
                let cents = self.cash(account, Cur::Cad) / 400;
                if cents < 1 {
                    return;
                }
                *n += 1;
                let id = self.next_id("interest");
                let row = set(self.row("INTEREST", "", account, at(day, *n), &id), &[(&["amount"], text(money(cents))), (&["currency"], text("CAD")), (&["amountSign"], text("positive"))]);
                self.push(row);
                self.move_cash(account, Cur::Cad, cents, day);
            }
            Plan::CardPurchase { account, cents } => {
                *n += 1;
                let id = self.next_id("card");
                let row = set(self.row("CREDIT_CARD", "PURCHASE", account, at(day, *n), &id), &[(&["amount"], text(money(cents))), (&["currency"], text("CAD")), (&["amountSign"], text("negative")), (&["spendMerchant"], text(format!("gen-merchant-{}", r.below(40))))]);
                self.push(row);
                self.move_cash(account, Cur::Cad, -cents, day);
            }
            Plan::CardPayment { card, from } => {
                let owed = -self.cash(card, Cur::Cad);
                if owed <= 0 {
                    return;
                }
                self.fund(from, Cur::Cad, owed, day, n);
                *n += 1;
                let id = self.next_id("payment");
                let paid = set(self.row("CREDIT_CARD_PAYMENT", "", from, at(day, *n), &id), &[(&["amount"], text(money(owed))), (&["currency"], text("CAD")), (&["amountSign"], text("negative"))]);
                let got = set(self.row("CREDIT_CARD", "PAYMENT", card, at(day, *n), &format!("{id}-card")), &[(&["amount"], text(money(owed))), (&["currency"], text("CAD")), (&["amountSign"], text("positive"))]);
                self.push(paid);
                self.push(got);
                self.move_cash(from, Cur::Cad, -owed, day);
                self.move_cash(card, Cur::Cad, owed, day);
            }
        }
    }

    /// The units an account held of each security (cash by its `sec-c-` id) at
    /// the end of a day.
    fn held_on(&self, a: usize, day: jiff::civil::Date) -> BTreeMap<String, i64> {
        let mut out = BTreeMap::new();
        for (id, changes) in &self.accounts[a].timeline {
            if let Some((_, q)) = changes.iter().rev().find(|(d, _)| *d <= day) {
                if *q != 0 {
                    out.insert(id.clone(), *q);
                }
            }
        }
        out
    }

    fn account_index(&self, id: &str) -> Option<usize> {
        self.accounts.iter().position(|a| a.id == id)
    }

    /// What the account's holdings and cash were worth at the end of a day, in
    /// CAD cents (USD at 1.35, the conversions' rate).
    fn worth(&self, a: usize, day: jiff::civil::Date) -> i64 {
        self.held_on(a, day)
            .iter()
            .map(|(id, q)| {
                let (cents, cur) = match id.as_str() {
                    "sec-c-cad" => (*q, Cur::Cad),
                    "sec-c-usd" => (*q, Cur::Usd),
                    _ => match self.listings.iter().find(|l| &l.id == id) {
                        Some(l) => (l.cost(*q, l.price), l.cur),
                        None => (0, Cur::Cad),
                    },
                };
                if cur == Cur::Usd {
                    cents * 135 / 100
                } else {
                    cents
                }
            })
            .sum()
    }

    fn security(&self, l: &Listing) -> Value {
        let (sort, ty) = match l.sort {
            Sort::Share => ("share", "EQUITY"),
            Sort::Coin => ("coin", "CRYPTOCURRENCY"),
            Sort::Contract => ("contract", "OPTION"),
        };
        let mut fields: Vec<(&[&str], Value)> = vec![(&["id"], text(&l.id)), (&["currency"], text(l.cur.code())), (&["securityType"], text(ty)), (&["stock", "symbol"], text(&l.symbol)), (&["stock", "name"], text(format!("{} Generated Listing", l.symbol)))];
        if l.sort == Sort::Share {
            let (exchange, mic) = if l.cur == Cur::Cad { ("TSX", "XTSE") } else { ("NASDAQ", "XNAS") };
            fields.push((&["stock", "primaryExchange"], text(exchange)));
            fields.push((&["stock", "primaryMic"], text(mic)));
        }
        if let Some((under, expiry, strike)) = &l.contract {
            fields.push((&["optionDetails", "expiryDate"], text(expiry.to_string())));
            fields.push((&["optionDetails", "strikePrice"], text(money(*strike))));
            fields.push((&["optionDetails", "optionType"], text("CALL")));
            fields.push((&["optionDetails", "multiplier"], Value::Number("100".into())));
            fields.push((&["optionDetails", "osiSymbol"], text(format!("{:<6}{}C{:08}", l.symbol, expiry.strftime("%y%m%d"), strike * 10))));
            fields.push((&["optionDetails", "underlyingSecurity", "id"], text(under)));
        }
        set(self.templates.securities[sort].clone(), &fields)
    }

    fn position(&self, id: &str, q: i64) -> Value {
        let (cur, symbol, ty, units, value) = match id {
            "sec-c-cad" => (Cur::Cad, "CAD".to_string(), "CURRENCY", money(q), q),
            "sec-c-usd" => (Cur::Usd, "USD".to_string(), "CURRENCY", money(q), q),
            _ => {
                let l = self.listings.iter().find(|l| l.id == id).expect("a listing held");
                let ty = match l.sort {
                    Sort::Share => "EQUITY",
                    Sort::Coin => "CRYPTOCURRENCY",
                    Sort::Contract => "OPTION",
                };
                (l.cur, l.symbol.clone(), ty, units_text(q, l.scale()), l.cost(q, l.price))
            }
        };
        let m = |cents: i64| Value::Object([("amount".to_string(), text(money(cents))), ("currency".to_string(), text(cur.code()))].into());
        set(
            self.templates.position.clone(),
            &[
                (&["quantity"], text(units)),
                (&["bookValue"], m(value)),
                (&["marketBookValue"], m(value)),
                (&["totalValue"], m(value)),
                (&["marketUnrealizedReturns"], m(0)),
                (&["security", "id"], text(id)),
                (&["security", "securityType"], text(ty)),
                (&["security", "stock", "symbol"], text(&symbol)),
                (&["security", "stock", "listedSymbol"], text(&symbol)),
            ],
        )
    }

    /// The rows made, for a test or a count.
    pub fn rows(&self) -> &[Value] {
        &self.rows
    }
}

/// A listing's generated symbol: `G` and letters, distinct per index.
fn name(mut i: usize) -> String {
    let mut s = String::from("G");
    loop {
        s.push((b'A' + (i % 26) as u8) as char);
        i /= 26;
        if i == 0 {
            break;
        }
    }
    s
}

impl Source for Generated {
    fn accounts(&mut self) -> Answer<Vec<Value>> {
        self.asked += 1;
        Ok(self
            .accounts
            .iter()
            .enumerate()
            .map(|(i, a)| {
                let worth = self.worth(i, self.today);
                set(
                    self.templates.account.clone(),
                    &[
                        (&["id"], text(&a.id)),
                        (&["unifiedAccountType"], text(a.ty)),
                        (&["nickname"], text(format!("Generated {i}"))),
                        (&["currency"], text("CAD")),
                        (&["createdAt"], text(format!("{}T12:00:00.000000Z", a.opened))),
                        (&["custodianAccounts"], Value::Array(vec![Value::Object([("branch".to_string(), text("TR")), ("custodian".to_string(), text("so")), ("id".to_string(), text(&a.custodian)), ("status".to_string(), text("open")), ("updatedAt".to_string(), text("2024-01-01T00:00:00.000000Z"))].into())])),
                        (&["accountOwners"], Value::Array(vec![])),
                        (&["accountFeatures"], Value::Array(vec![])),
                        (&["financials", "currentCombined", "netLiquidationValue", "amount"], text(money(worth))),
                    ],
                )
            })
            .collect())
    }

    fn activity(&mut self, account: &str, from: Option<jiff::civil::Date>) -> Answer<Vec<Value>> {
        self.asked += 1;
        Ok(self
            .rows
            .iter()
            .filter(|r| {
                let n = Node::root(r);
                n.text("accountId").ok() == Some(account) && from.is_none_or(|f| n.text("occurredAt").ok().and_then(|t| t.get(..10)).and_then(|t| t.parse::<jiff::civil::Date>().ok()).is_some_and(|d| d >= f))
            })
            .cloned()
            .collect())
    }

    fn securities(&mut self, ids: &[String]) -> Answer<Vec<Value>> {
        self.asked += 1;
        Ok(ids.iter().filter_map(|id| self.listings.iter().find(|l| &l.id == id)).map(|l| self.security(l)).collect())
    }

    fn order(&mut self, _batch: &str) -> Answer<Option<Value>> {
        self.asked += 1;
        Ok(None)
    }

    fn entitlements(&mut self, _activity: &str) -> Answer<Option<Value>> {
        self.asked += 1;
        Ok(None)
    }

    fn conversion(&mut self, id: &str) -> Answer<Option<Value>> {
        self.asked += 1;
        Ok(self.conversions.get(id).cloned())
    }

    fn transfer(&mut self, _id: &str) -> Answer<Option<Value>> {
        self.asked += 1;
        Ok(None)
    }

    fn card(&mut self, account: &str) -> Answer<Value> {
        self.asked += 1;
        let a = self.account_index(account).ok_or_else(|| Failure::Refused(format!("no card account {account}")))?;
        let owed = -self.cash(a, Cur::Cad);
        Ok(set(self.templates.card.clone(), &[(&["id"], text(account)), (&["balance", "current"], text(money(owed))), (&["balance", "outstanding"], text(money(owed))), (&["balance", "pending"], text("0.00"))]))
    }

    fn positions(&mut self, account: &str, day: jiff::civil::Date) -> Answer<Value> {
        self.asked += 1;
        let a = self.account_index(account).ok_or_else(|| Failure::Refused(format!("no account {account}")))?;
        Ok(Value::Array(self.held_on(a, day).iter().map(|(id, q)| self.position(id, *q)).collect()))
    }

    fn balances(&mut self, accounts: &[String]) -> Answer<Vec<Value>> {
        self.asked += 1;
        Ok(accounts
            .iter()
            .filter_map(|id| self.account_index(id))
            .filter(|a| self.accounts[*a].role != Role::Card)
            .map(|a| {
                let acct = &self.accounts[a];
                let balance: Vec<Value> = acct.cash.iter().map(|(c, q)| Value::Object([("__typename".to_string(), text("Balance")), ("quantity".to_string(), text(money(*q))), ("securityId".to_string(), text(c.cash_id()))].into())).collect();
                set(self.templates.balances.clone(), &[(&["id"], text(&acct.id)), (&["custodianAccounts"], Value::Array(vec![Value::Object([("__typename".to_string(), text("CustodianAccount")), ("id".to_string(), text(&acct.custodian)), ("financials".to_string(), Value::Object([("__typename".to_string(), text("CustodianAccountFinancialsSo")), ("balance".to_string(), Value::Array(balance))].into()))].into())]))])
            })
            .collect())
    }

    fn buying_power(&mut self, account: &str) -> Answer<Value> {
        self.asked += 1;
        let a = self.account_index(account).ok_or_else(|| Failure::Refused(format!("no account {account}")))?;
        let cad = self.cash(a, Cur::Cad) + self.cash(a, Cur::Usd) * 135 / 100;
        let reply = format!(r#"{{"id":"{account}","financials":{{"current":{{"id":"c","marginV3":{{"trading":{{"buyingPower":{{"__typename":"BuyingPowerMetricAvailable","total":{{"amount":"{}","currency":"CAD"}}}},"__typename":"MarginTrading"}},"__typename":"MarginV3"}},"__typename":"Current"}},"__typename":"Financials"}},"__typename":"Account"}}"#, money(cad));
        json::parse(&reply).map_err(|e| Failure::Mismatch(e.to_string()))
    }

    fn history(&mut self, account: &str, from: Option<jiff::civil::Date>) -> Answer<Vec<Value>> {
        self.asked += 1;
        let a = self.account_index(account).ok_or_else(|| Failure::Refused(format!("no account {account}")))?;
        let acct = &self.accounts[a];
        let mut day = from.unwrap_or(acct.opened).max(acct.opened);
        let mut out = Vec::new();
        while day <= self.today {
            let deposits: i64 = acct.deposits.iter().filter(|(d, _)| *d <= day).map(|(_, c)| c).sum();
            let worth = self.worth(a, day);
            let m = |cents: i64| Value::Object([("amount".to_string(), text(money(cents))), ("cents".to_string(), Value::Number(cents.to_string())), ("currency".to_string(), text("CAD"))].into());
            out.push(set(self.templates.history.clone(), &[(&["date"], text(day.to_string())), (&["netDepositsV2"], m(deposits)), (&["netLiquidationValueV2"], m(worth))]));
            day = day.tomorrow().map_err(|e| Failure::Mismatch(e.to_string()))?;
        }
        Ok(out)
    }

    fn statement(&mut self, _account: &str, _month: jiff::civil::Date, _kind: &str) -> Answer<Option<Value>> {
        self.asked += 1;
        // the cash the book holds is what the balances state, so no month is asked
        Ok(None)
    }

    fn requests(&self) -> usize {
        self.asked
    }
}
