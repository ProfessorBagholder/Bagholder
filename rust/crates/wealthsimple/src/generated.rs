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
//!
//! The shapes the real feed has that a smooth book lacks (brief 76) are each
//! off unless a test names it (`Shape`, `Generated::shaped`): a move listed by
//! its arrival alone, an account under two ids Wealthsimple links, a closed
//! account with history, a row listed in it after its one full read, a
//! conversion whose detail states the side received alone, the monthly
//! statements, each row in a form a recorded statement writes
//! (`tests/replies/wealthsimple-statements`, held by `tests/forms.rs`), and calls
//! written and assigned.

use std::collections::{BTreeMap, BTreeSet};

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
    /// Round trips more on one listing in one investment account, each a purchase
    /// and a sale of all of it, the last still open: one instrument's history
    /// this deep (twice as many rows, less one), beside the rest of the book
    /// (the capacity test's depth pair).
    pub deep: usize,
}

const ACTIVITY: &str = include_str!("../tests/replies/wealthsimple/edited-activity-one-per-kind.json");
const ACCOUNT: &str = include_str!("../tests/replies/wealthsimple-pull/edited-accounts-one.json");
const SECURITIES: &str = include_str!("../tests/replies/wealthsimple/edited-securities.json");
const BALANCES: &str = include_str!("../tests/replies/wealthsimple-pull/edited-balances.json");
const HISTORY: &str = include_str!("../tests/replies/wealthsimple-pull/history-1.json");
const POSITIONS: &str = include_str!("../tests/replies/wealthsimple/positions@anon-resp-2@2025-07-05.json");
const CONVERSION: &str = include_str!("../tests/replies/wealthsimple/FetchInternalTransfer-1.json");
const CARD: &str = include_str!("../tests/replies/wealthsimple/credit-card-account-1.json");
const FUNDING: &str = include_str!("../tests/replies/wealthsimple/FetchFundingIntent-1.json");
const STATEMENT: &str = include_str!("../tests/replies/wealthsimple-statements/lira-2025-06.json");

/// A shape of the real feed (brief 76), off unless a book names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Shape {
    /// Every other move between two accounts, of cash or of a holding (an
    /// account retired by the shapes below among them), listed by its arrival
    /// alone: the arriving row names the account it came from, whose feed lists
    /// no row of it (a managed account's cash moved as it closed, read
    /// 2026-10-20; the recorded `ASSET_MOVEMENT` of edges 91 and 100; rule 15).
    ArrivalOnly,
    /// An investment account under two ids Wealthsimple links: halfway through,
    /// the first id's holdings are sold and its cash moved to a second id, and
    /// the first is closed; each names the other as its `linkedAccount`.
    LinkedIds,
    /// An investment account closed with history: two thirds of the way
    /// through, its holdings are sold and its cash moved to the first
    /// investment account; nothing is listed in it after.
    Closed,
    /// With `Closed`: a little of the closed account's cash kept, and moved to
    /// the first investment account on the last day, listed only once
    /// `publish_late` is called: a row in a closed account dated after its one
    /// full read.
    LateRow,
    /// Every other conversion a funding intent
    /// (`FetchFundingIntent`), whose detail states the side received alone; the
    /// statements state the side paid.
    FundingIntent,
    /// Each investment and crypto account's monthly statement for every month
    /// it was open and that has ended: each currency's movements with their
    /// running balance, as `FetchMonthlyStatementWithTransactions` states them.
    Statements,
    /// Typed, never recorded (`READ_BY_2_10_9`): every third fill of a month a
    /// statement covers pays a fee the activity feed's amount leaves out, the
    /// statement's cash and the account's carrying it.
    FeeFolded,
    /// Typed, never recorded (`READ_BY_2_10_9`): every seventh purchase of a month
    /// a statement covers is listed by the statement and the positions alone; one
    /// sold the day it was bought is listed after all.
    FeedOmits,
    /// Typed, never recorded (`READ_BY_2_10_9`): every fourth purchase of a US
    /// share in a month a statement covers is paid out of CAD at the made-up
    /// broker's rate, the feed stating it in USD, the statement in CAD.
    FxFunded,
    /// Calls written on a US share held for them, assigned at expiry: the feed
    /// states the contracts sold and their assignment, the statement the
    /// contracts sold, their assignment at no cash, and the shares sold at the
    /// strike a few days after (`ASSIGN`, then `SELL`), as the recorded forms
    /// state them (`tests/replies/wealthsimple-statements`).
    Assigned,
    /// A company listed in the US and in Canada, the same units of each bought the
    /// same day in the first investment account and sold together: the feed and the
    /// securities reply name the Canadian listing `SYM.TO`, in CAD at its exchange,
    /// and the US one `SYM`, in USD, while the statement writes the bare `SYM` in
    /// its CAD list for the Canadian listing's fills
    /// (`tests/replies/wealthsimple-statements/feed-fill-dot-to.json`, and the `.TO`
    /// securities of `edited-securities.json`).
    Interlisted,
    /// The coin account's statements from the book's middle month on, as the
    /// recorded later ones: lists per currency, and each coin fill two rows, the
    /// fill at its cash before the fee and the fee on a row of its own (`Fee for
    /// purchase of N SYM …`, `Trading fee for sale of N SYM …`). A sale's two rows
    /// come to the feed's amount; a purchase's to a cent under it in the form's
    /// first month and to the amount after, as recorded. The account's cash is
    /// the feed's.
    Coin2025,
    /// Every third coin sale in a month a statement covers is a limit order the
    /// feed dates the day it was placed, 9 to 12 days before the execution the
    /// statement states: the recorded limit sells' feed rows state no other date.
    LimitOrder,
    /// A US share of its own in the first investment account, bought and sold
    /// whole, then bought again and held, in the newest month a statement covers:
    /// the statement states each fill in its
    /// USD list as two rows, the cash at `Bought 0.0000 shares at $P per share
    /// (executed at D), FX Rate: R` and the units at no cash, its `unit` the
    /// symbol (`forms-brokerage.json`, recorded again with its zero).
    SplitFill,
}

/// Every shape.
pub const SHAPES: [Shape; 6] = [Shape::ArrivalOnly, Shape::LinkedIds, Shape::Closed, Shape::LateRow, Shape::FundingIntent, Shape::Statements];

/// The fills as the feed and the statements state them on recorded books, the
/// statements' forms and the feed's dates as recorded.
pub const FILL_FORMS: [Shape; 4] = [Shape::Interlisted, Shape::Coin2025, Shape::LimitOrder, Shape::SplitFill];

/// The typed shapes the two books of `tests/take_back.rs` were written from by
/// 2.10.8 and 2.10.9, kept only to read those books again: nothing recorded
/// shows any of them. They go with `take_back_2_10_9`.
pub const READ_BY_2_10_9: [Shape; 3] = [Shape::FeeFolded, Shape::FeedOmits, Shape::FxFunded];

/// An assignment as the feed and the statement each state it. No reading of
/// statements is proven to close its months.
pub const ASSIGNED: [Shape; 1] = [Shape::Assigned];

/// The made-up broker's fee on a coin's fill, in thousandths of its cash, inside
/// the cash both the feed and the statement state: made up, as every figure here.
const COIN_FEE_PER_THOUSAND: i64 = 15;

/// The made-up broker's fee a contract written, in cents, inside the cash both
/// the feed and the statement state: made up.
const CONTRACT_FEE_CENTS: i64 = 75;

/// Shares a contract delivers on assignment, as every recorded contract states
/// its multiplier (`optionDetails.multiplier`).
const SHARES_A_CONTRACT: i64 = 100;

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
    /// A contract that is a put (else a call).
    put: bool,
    /// The name and venue (exchange, MIC) Wealthsimple states, where they are
    /// stated rather than generated.
    name: Option<String>,
    venue: Option<(String, String)>,
    /// What Wealthsimple states of a share's id: `TRADING`, or `CORPORATE_ACTION`
    /// for one retired by a corporate action, the listing trading on under a
    /// successor id of the same symbol and venue.
    status: &'static str,
}

impl Listing {
    /// Units are whole for shares and contracts, ten-billionths for a coin: the
    /// units a coin's fill executed, as its statement row writes them to 10 places;
    /// the feed states them rounded to 6 (`feed_units`).
    fn scale(&self) -> u32 {
        if self.sort == Sort::Coin {
            10
        } else {
            0
        }
    }
    /// What `units` (in the listing's own scale) cost at `price` cents, in cents.
    fn cost(&self, units: i64, price: i64) -> i64 {
        match self.sort {
            Sort::Share => units * price,
            Sort::Contract => units * price * 100,
            Sort::Coin => ((units as i128 * price as i128) / 10_000_000_000) as i64,
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
    ty: String,
    role: Role,
    /// Stated rather than generated (`Generated::from_records`): the account's
    /// name and currency, each day's worth and net deposits in CAD cents, and
    /// what a margin account can borrow in CAD cents.
    nickname: Option<String>,
    currency: Cur,
    days: Option<Vec<(jiff::civil::Date, i64, i64)>>,
    buying_power: Option<i64>,
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
    /// The day it was closed, where it was.
    closed: Option<jiff::civil::Date>,
    /// The account Wealthsimple states it linked to.
    linked: Option<usize>,
    /// Each movement of its cash, in order, as a statement lists it.
    ledger: Vec<Line>,
}

/// A movement of cash as a monthly statement lists it.
#[derive(Clone, Debug)]
struct Line {
    day: jiff::civil::Date,
    cur: Cur,
    cents: i64,
    code: &'static str,
    description: String,
    /// What the row's `unit` names where it is not the cash's currency: the
    /// symbol of what moved.
    unit: Option<String>,
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
    /// Every holding of `from` sold and its cash moved to `to`, `keep` cents of
    /// CAD left behind; `from` closed (`Shape::LinkedIds`, `Shape::Closed`).
    Retire { from: usize, to: usize, keep: i64 },
    /// Calls sold to open on shares held for them (`Shape::Assigned`).
    WriteCalls { account: usize, listing: usize, units: i64 },
    /// The calls written assigned at expiry, the shares delivered.
    Assign { account: usize, listing: usize },
    /// A purchase or sale at its own stated cash, never funded first: a book
    /// stated whole (`Generated::from_records`).
    #[cfg(feature = "demo")]
    BuyAt { account: usize, listing: usize, units: i64, cents: i64 },
    #[cfg(feature = "demo")]
    SellAt { account: usize, listing: usize, units: i64, cents: i64 },
    #[cfg(feature = "demo")]
    DividendOf { account: usize, listing: usize, units: i64, cents: i64 },
    #[cfg(feature = "demo")]
    StakingOf { account: usize, listing: usize, units: i64 },
    #[cfg(feature = "demo")]
    Charge { account: usize, cur: Cur, cents: i64 },
}

/// An account stated whole, for `Generated::from_records`.
#[cfg(feature = "demo")]
#[derive(Clone, Debug)]
pub struct AccountRecord {
    pub id: String,
    pub nickname: String,
    /// Wealthsimple's `unifiedAccountType`.
    pub ty: String,
    /// `CAD` or `USD`.
    pub currency: String,
    pub opened: jiff::civil::Date,
    /// Each day its daily history states: the day, its worth and its net
    /// deposits, in CAD cents. The last is what it is worth now.
    pub days: Vec<(jiff::civil::Date, i64, i64)>,
    /// What a margin account can borrow, in CAD cents; else its cash.
    pub buying_power: Option<i64>,
}

/// What a listing stated whole is.
#[cfg(feature = "demo")]
#[derive(Clone, Debug)]
pub enum ListingSort {
    /// Shares, on an exchange (its name and MIC).
    Share { exchange: String, mic: String },
    Coin,
    /// An option contract on the listing of id `underlying`, its strike in cents.
    Contract { underlying: String, expiry: jiff::civil::Date, strike: i64, put: bool },
}

/// A listing stated whole, for `Generated::from_records`.
#[cfg(feature = "demo")]
#[derive(Clone, Debug)]
pub struct ListingRecord {
    pub id: String,
    pub symbol: String,
    pub name: String,
    /// `CAD` or `USD`.
    pub currency: String,
    pub sort: ListingSort,
    /// A unit's price now in cents (a coin's per whole coin; a contract's
    /// premium per share): what a holding of it is stated to be worth.
    pub price: i64,
}

/// One piece of activity stated whole, by account and listing id. Units are
/// whole for shares and contracts and millionths for a coin; cash in cents.
#[cfg(feature = "demo")]
#[derive(Clone, Debug)]
pub enum Record {
    Buy { account: String, listing: String, units: i64, cents: i64 },
    Sell { account: String, listing: String, units: i64, cents: i64 },
    /// Paid on `units` held, as Wealthsimple states them on the row.
    Dividend { account: String, listing: String, units: i64, cents: i64 },
    Staking { account: String, listing: String, units: i64 },
    /// CAD into the account.
    Deposit { account: String, cents: i64 },
    /// Margin interest charged, in `currency` (`CAD` or `USD`).
    InterestCharge { account: String, currency: String, cents: i64 },
}

#[cfg(feature = "demo")]
fn cur_of(code: &str) -> Cur {
    match code {
        "CAD" => Cur::Cad,
        "USD" => Cur::Usd,
        other => panic!("a made-up Wealthsimple holds CAD and USD, not {other}"),
    }
}

fn role_of(ty: &str) -> Role {
    match ty {
        "SELF_DIRECTED_CRYPTO" => Role::Crypto,
        "CASH" => Role::Spending,
        "CREDIT_CARD" => Role::Card,
        _ => Role::Invest,
    }
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
    /// Each account whose activity was asked, and from which day (the whole of
    /// it when `None`), in order.
    pub asked_activity: Vec<(String, Option<jiff::civil::Date>)>,
    shapes: BTreeSet<Shape>,
    /// From a day on, what names an account names another (one retired):
    /// `(from, to, day)`.
    redirect: Vec<(usize, usize, jiff::civil::Date)>,
    /// Moves of cash and conversions made, each kind counted, for the shapes
    /// that take every other one.
    moves: usize,
    funded: usize,
    /// The move of `LateRow`, not listed until `publish_late`: its day, from,
    /// to and cents.
    late: Option<(jiff::civil::Date, usize, usize, i64)>,
    /// Rows whose cash has moved, not listed until `publish_late`.
    held: Vec<Value>,
    /// Fills and purchases made, counted for `FeeFolded`, `FeedOmits` and `FxFunded`.
    fills: usize,
    buys: usize,
    us_buys: usize,
    /// Purchases the feed does not list (`FeedOmits`): the account, the listing,
    /// the day and the row, listed after all if the same day sells the listing.
    omitted: Vec<(usize, usize, jiff::civil::Date, Value)>,
    /// The share held for the calls written on it (`Assigned`), never moved.
    written_on: Option<usize>,
    /// The listing whose fills the statement states in two rows (`SplitFill`).
    split_on: Option<usize>,
    /// The first month the coin account's statements take their later form (`Coin2025`).
    coin_lists_from: Option<jiff::civil::Date>,
    /// Coin sales in months a statement covers, counted for `LimitOrder`.
    coin_sales: usize,
    /// Coin purchases made, each one's units past the feed's six places drawn from
    /// this count, apart from the book's draws.
    coin_buys: usize,
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
    funding: Value,
    statement: Value,
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
        let funding = parse(FUNDING);
        let funding = data(&funding).obj("searchFundingIntents").and_then(|f| f.list("edges")).expect("funding intents")[0].obj("node").expect("a funding intent").value().clone();
        let statement = parse(STATEMENT);
        let statement = data(&statement).obj("monthlyStatement").expect("a statement").value().clone();
        Templates { rows, account, securities, balances, history, position, conversion, card, funding, statement }
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

/// Units as the feed states them: a coin's rounded to six places, as every
/// recorded coin row is; a share's or a contract's whole.
fn feed_units(units: i64, l: &Listing) -> String {
    if l.sort == Sort::Coin {
        Dec::new(units, 10).expect("units fit").round(6, bagholder_core::Rounding::HalfEven).to_text()
    } else {
        units_text(units, l.scale())
    }
}

fn at(day: jiff::civil::Date, n: u32) -> String {
    // a second a row from 15:00 UTC, so `n` keeps a day's rows in order; never
    // past 23:59:59 UTC, which is the same day in Toronto all year
    let s = n.min(9 * 3600 - 1);
    format!("{day}T{:02}:{:02}:{:02}.000000+00:00", 15 + s / 3600, (s / 60) % 60, s % 60)
}

impl Generated {
    pub fn new(size: Size, today: jiff::civil::Date) -> Generated {
        Generated::shaped(size, today, &[])
    }

    /// A made-up Wealthsimple with these shapes of the real feed (`Shape`).
    /// `LinkedIds` and `Closed` need three investment accounts (seven accounts
    /// or more), `LateRow` needs `Closed`.
    pub fn shaped(size: Size, today: jiff::civil::Date, shapes: &[Shape]) -> Generated {
        let templates = Templates::read();
        let mut r = Rng(size.seed);
        let start = today.checked_sub(jiff::Span::new().months(size.months as i64)).expect("a start");
        let accounts: Vec<Acct> = (0..size.accounts)
            .map(|i| {
                let ty = TYPES[i % TYPES.len()];
                let role = role_of(ty);
                Acct { id: format!("gen-account-{i}"), custodian: format!("gen-custodian-{i}"), ty: ty.to_string(), role, nickname: None, currency: Cur::Cad, days: None, buying_power: None, cash: BTreeMap::new(), held: BTreeMap::new(), timeline: BTreeMap::new(), deposits: Vec::new(), opened: start, closed: None, linked: None, ledger: Vec::new() }
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
            listings.push(Listing { id, symbol, sort: if coin { Sort::Coin } else { Sort::Share }, cur, price, contract: None, put: false, name: None, venue: None, status: "TRADING" });
        }
        // about one share in sixteen retired by a corporate action and trading on
        // under a successor id (11 of 173 ids in the reference capture), so a book
        // carries successions as a real one does
        let retired: Vec<usize> = (0..listings.len()).filter(|i| listings[*i].sort == Sort::Share && i % 16 == 3).collect();
        for i in retired {
            let mut next = listings[i].clone();
            next.id = format!("sec-s-{}", r.hex());
            listings[i].status = "CORPORATE_ACTION";
            listings.push(next);
        }
        let shapes: BTreeSet<Shape> = shapes.iter().copied().collect();
        assert!(!shapes.contains(&Shape::LateRow) || shapes.contains(&Shape::Closed), "a late row is in the closed account");
        let mut g = Generated { today, accounts, listings, rows: Vec::new(), conversions: BTreeMap::new(), templates, asked: 0, asked_activity: Vec::new(), shapes, redirect: Vec::new(), moves: 0, funded: 0, late: None, held: Vec::new(), fills: 0, buys: 0, us_buys: 0, omitted: Vec::new(), written_on: None, split_on: None, coin_lists_from: None, coin_sales: 0, coin_buys: 0 };
        let mut plans = g.plan(&mut r, size, start);
        let span = (today - start).get_days().max(2) as i64;
        let on = |n: i64| start.checked_add(jiff::Span::new().days(n)).expect("a day");
        let invest = g.of_role(Role::Invest);
        let needs = |what: &str| -> usize { panic!("{what} needs three investment accounts, the book has {}", invest.len()) };
        if g.shapes.contains(&Shape::LinkedIds) {
            // the second investment account's id retired halfway, for a new one it links to
            let old = invest.get(1).copied().unwrap_or_else(|| needs("LinkedIds"));
            let day = on(span / 2);
            let n = g.accounts.len();
            let mut new = g.accounts[old].clone();
            new.id = format!("gen-account-{n}");
            new.custodian = format!("gen-custodian-{n}");
            new.opened = day;
            new.linked = Some(old);
            g.accounts.push(new);
            g.accounts[old].linked = Some(n);
            g.redirect.push((old, n, day));
            plans.entry(day).or_default().insert(0, Plan::Retire { from: old, to: n, keep: 0 });
        }
        if g.shapes.contains(&Shape::Closed) {
            let (closing, to) = (invest.get(2).copied().unwrap_or_else(|| needs("Closed")), invest[0]);
            let day = on(span * 2 / 3);
            let keep = if g.shapes.contains(&Shape::LateRow) { 1_234 } else { 0 };
            g.redirect.push((closing, to, day));
            plans.entry(day).or_default().insert(0, Plan::Retire { from: closing, to, keep });
            if keep > 0 {
                g.late = Some((today, closing, to, keep));
            }
        }
        if g.shapes.contains(&Shape::Assigned) {
            // a US share of its own, bought in the first investment account, which
            // nothing else trades or moves, and calls written on all of it
            let account = *invest.first().unwrap_or_else(|| panic!("Assigned needs an investment account"));
            let symbol = name(g.listings.len());
            let price = r.range(2_000, 9_000);
            g.listings.push(Listing { id: format!("sec-s-{}", r.hex()), symbol: symbol.clone(), sort: Sort::Share, cur: Cur::Usd, price, contract: None, put: false, name: None, venue: None, status: "TRADING" });
            let under = g.listings.len() - 1;
            g.written_on = Some(under);
            let (bought, expiry) = (on(span / 4), on(span / 4 + 30));
            assert!(expiry < today.first_of_month(), "the calls are assigned in a month a statement covers");
            let strike = (price / 100 + 1) * 100;
            g.listings.push(Listing { id: format!("sec-o-{}", r.hex()), symbol, sort: Sort::Contract, cur: Cur::Usd, price: r.range(50, 400), contract: Some((g.listings[under].id.clone(), expiry, strike)), put: false, name: None, venue: None, status: "TRADING" });
            let contract = g.listings.len() - 1;
            let contracts = r.range(1, 3);
            plans.entry(bought).or_default().push(Plan::Buy { account, listing: under, units: contracts * SHARES_A_CONTRACT });
            plans.entry(on(span / 4 + 1)).or_default().push(Plan::WriteCalls { account, listing: contract, units: contracts });
            plans.entry(expiry).or_default().push(Plan::Assign { account, listing: contract });
        }
        if g.shapes.contains(&Shape::Interlisted) {
            // drawn apart, so the listings and plans the book has without it stay as they are
            let mut own = Rng(size.seed ^ 0x2E54_4F2E_544F);
            let account = *invest.first().unwrap_or_else(|| panic!("Interlisted needs an investment account"));
            let symbol = name(g.listings.len());
            let company = format!("{symbol} Interlisted Inc");
            let price = own.range(500, 9_000);
            let mut listed = |g: &mut Generated, symbol: String, cur: Cur, price: i64, venue: (&str, &str)| {
                g.listings.push(Listing { id: format!("sec-s-{}", own.hex()), symbol, sort: Sort::Share, cur, price, contract: None, put: false, name: Some(company.clone()), venue: Some((venue.0.to_string(), venue.1.to_string())), status: "TRADING" });
                g.listings.len() - 1
            };
            let us = listed(&mut g, symbol.clone(), Cur::Usd, price, ("NASDAQ", "XNAS"));
            let canada = listed(&mut g, format!("{symbol}.TO"), Cur::Cad, price * 135 / 100, ("TSX", "XTSE"));
            // the same units of each, the same day: only the listing tells one fill from the other
            let units = 10 + size.seed as i64 % 90;
            for listing in [us, canada] {
                plans.entry(on(span / 3)).or_default().push(Plan::Buy { account, listing, units });
                plans.entry(on(span / 3 + 10)).or_default().push(Plan::Sell { account, listing, all: true });
            }
        }
        if g.shapes.contains(&Shape::Coin2025) {
            g.coin_lists_from = Some(on(span / 2).first_of_month());
        }
        if g.shapes.contains(&Shape::SplitFill) {
            // a US share of its own, which nothing else trades or moves: a round trip, then a holding
            let mut own = Rng(size.seed ^ 0x5350_4C49_54);
            let account = *invest.first().unwrap_or_else(|| panic!("SplitFill needs an investment account"));
            g.listings.push(Listing { id: format!("sec-s-{}", own.hex()), symbol: name(g.listings.len()), sort: Sort::Share, cur: Cur::Usd, price: own.range(200, 6_000), contract: None, put: false, name: None, venue: None, status: "TRADING" });
            let listing = g.listings.len() - 1;
            g.split_on = Some(listing);
            // in the newest month a statement covers
            let newest = today.first_of_month().checked_sub(jiff::Span::new().months(1)).expect("a month");
            let day = |n: i64| newest.checked_add(jiff::Span::new().days(n)).expect("a day");
            let units = 100 * own.range(1, 20);
            plans.entry(day(2)).or_default().push(Plan::Buy { account, listing, units });
            plans.entry(day(6)).or_default().push(Plan::Sell { account, listing, all: true });
            plans.entry(day(10)).or_default().push(Plan::Buy { account, listing, units: units / 2 });
        }
        g.run(&mut r, plans);
        g
    }

    /// The move of `Shape::LateRow` listed: Wealthsimple lists a row in the
    /// closed account, dated after the pull's one full read of it.
    pub fn publish_late(&mut self) {
        let held = std::mem::take(&mut self.held);
        self.rows.extend(held);
        let Some((day, from, to, cents)) = self.late.take() else { return };
        let mut n = 10_000;
        self.transfer(from, to, Cur::Cad, cents, day, &mut n, true);
    }

    /// The ids of the accounts the shapes made: the account retired for the id
    /// it links to (`LinkedIds`), and the one closed (`Closed`), each with the
    /// account that took its place.
    pub fn retired(&self) -> Vec<(String, String)> {
        self.redirect.iter().map(|(from, to, _)| (self.accounts[*from].id.clone(), self.accounts[*to].id.clone())).collect()
    }

    #[cfg(feature = "demo")]
    /// A made-up Wealthsimple stated whole rather than generated: these
    /// accounts, listings and activity, each purchase, sale and payment at its
    /// own cash and nothing funded but what `records` deposits. Each account's
    /// daily history is the days it states; its worth now the last of them.
    /// `records` is in the order it happens; a day's rows keep their order.
    pub fn from_records(today: jiff::civil::Date, accounts: &[AccountRecord], listings: &[ListingRecord], records: &[(jiff::civil::Date, Record)]) -> Generated {
        let accounts: Vec<Acct> = accounts
            .iter()
            .enumerate()
            .map(|(i, a)| Acct {
                id: a.id.clone(),
                custodian: format!("{}-custodian-{i}", a.id),
                ty: a.ty.clone(),
                role: role_of(&a.ty),
                nickname: Some(a.nickname.clone()),
                currency: cur_of(&a.currency),
                days: Some(a.days.clone()),
                buying_power: a.buying_power,
                cash: BTreeMap::new(),
                held: BTreeMap::new(),
                timeline: BTreeMap::new(),
                deposits: Vec::new(),
                opened: a.opened,
                closed: None,
                linked: None,
                ledger: Vec::new(),
            })
            .collect();
        let listings: Vec<Listing> = listings
            .iter()
            .map(|l| {
                let (sort, contract, put, venue) = match &l.sort {
                    ListingSort::Share { exchange, mic } => (Sort::Share, None, false, Some((exchange.clone(), mic.clone()))),
                    ListingSort::Coin => (Sort::Coin, None, false, None),
                    ListingSort::Contract { underlying, expiry, strike, put } => (Sort::Contract, Some((underlying.clone(), *expiry, *strike)), *put, None),
                };
                Listing { id: l.id.clone(), symbol: l.symbol.clone(), sort, cur: cur_of(&l.currency), price: l.price, contract, put, name: Some(l.name.clone()), venue, status: "TRADING" }
            })
            .collect();
        let account = |id: &str| accounts.iter().position(|a| a.id == id).unwrap_or_else(|| panic!("no account {id} among the records"));
        let listing = |id: &str| listings.iter().position(|l| l.id == id).unwrap_or_else(|| panic!("no listing {id} among the records"));
        // a record's coin units are millionths; the made-up broker holds ten-billionths
        let units_of = |l: &str, units: i64| if listings[listing(l)].sort == Sort::Coin { units * 10_000 } else { units };
        let mut plans: BTreeMap<jiff::civil::Date, Vec<Plan>> = BTreeMap::new();
        for (day, r) in records {
            let p = match r {
                Record::Buy { account: a, listing: l, units, cents } => Plan::BuyAt { account: account(a), listing: listing(l), units: units_of(l, *units), cents: *cents },
                Record::Sell { account: a, listing: l, units, cents } => Plan::SellAt { account: account(a), listing: listing(l), units: units_of(l, *units), cents: *cents },
                Record::Dividend { account: a, listing: l, units, cents } => Plan::DividendOf { account: account(a), listing: listing(l), units: units_of(l, *units), cents: *cents },
                Record::Staking { account: a, listing: l, units } => Plan::StakingOf { account: account(a), listing: listing(l), units: units_of(l, *units) },
                Record::Deposit { account: a, cents } => Plan::Deposit { account: account(a), cents: *cents },
                Record::InterestCharge { account: a, currency, cents } => Plan::Charge { account: account(a), cur: cur_of(currency), cents: *cents },
            };
            plans.entry(*day).or_default().push(p);
        }
        let mut g = Generated { today, accounts, listings, rows: Vec::new(), conversions: BTreeMap::new(), templates: Templates::read(), asked: 0, asked_activity: Vec::new(), shapes: BTreeSet::new(), redirect: Vec::new(), moves: 0, funded: 0, late: None, held: Vec::new(), fills: 0, buys: 0, us_buys: 0, omitted: Vec::new(), written_on: None, split_on: None, coin_lists_from: None, coin_sales: 0, coin_buys: 0 };
        // nothing stated whole is drawn
        g.run(&mut Rng(0), plans);
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
                    self.listings.push(Listing { id: format!("sec-o-{}", r.hex()), symbol: u.symbol.clone(), sort: Sort::Contract, cur: Cur::Usd, price: r.range(5, 900), contract: Some((u.id.clone(), expiry, strike)), put: false, name: None, venue: None, status: "TRADING" });
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
                Sort::Coin => r.range(1_000, 5_000_000) * 10_000,
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
        // one listing's history made deep: round trips one after another, each
        // closed before the next opens
        if let (Some(&account), Some(&listing)) = (invest.first(), shares.first()) {
            for k in 0..size.deep {
                let opened = 1 + (k as i64 * (days - 3)) / size.deep as i64;
                let closed = (1 + ((k as i64 + 1) * (days - 3)) / size.deep as i64 - 1).max(opened);
                out.entry(day(opened)).or_default().push(Plan::Buy { account, listing, units: r.range(1, 200) });
                // a distribution in the middle of the history, whatever its depth: the
                // capacity test's back-dated record (`capacity`, `Op::BackdatedRecord`)
                if k == size.deep / 2 {
                    out.entry(day(opened)).or_default().push(Plan::Dividend { account, listing });
                }
                // the last stays open: the listing is held today
                if k + 1 < size.deep {
                    out.entry(day(closed)).or_default().push(Plan::Sell { account, listing, all: true });
                }
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
        for (day, list) in plans {
            // counted from each day's first row: a count over the whole book ran past
            // the hour and timed a day's sale before its buy (#618)
            let mut n: u32 = 0;
            for p in list {
                n += 1;
                let p = self.redirected(p, day);
                self.apply(r, day, p, &mut n);
            }
        }
    }

    /// A plan naming an account retired by `day` names the one that took its place.
    fn redirected(&self, p: Plan, day: jiff::civil::Date) -> Plan {
        let to = |a: usize| self.redirect.iter().filter(|(from, _, d)| *from == a && *d <= day).map(|(_, to, _)| *to).next().unwrap_or(a);
        match p {
            Plan::Buy { account, listing, units } => Plan::Buy { account: to(account), listing, units },
            Plan::Sell { account, listing, all } => Plan::Sell { account: to(account), listing, all },
            Plan::Expire { account, listing } => Plan::Expire { account: to(account), listing },
            Plan::Dividend { account, listing } => Plan::Dividend { account: to(account), listing },
            Plan::Staking { account, listing } => Plan::Staking { account: to(account), listing },
            Plan::Move { from, to: t } => Plan::Move { from: to(from), to: to(t) },
            Plan::MoveHolding { from, to: t } => Plan::MoveHolding { from: to(from), to: to(t) },
            Plan::Deposit { account, cents } => Plan::Deposit { account: to(account), cents },
            Plan::Withdraw { account } => Plan::Withdraw { account: to(account) },
            Plan::Interest { account } => Plan::Interest { account: to(account) },
            other => other,
        }
    }

    /// The fee every third fill pays beyond what the feed's amount states
    /// (`FeeFolded`), else none.
    fn folded_fee(&mut self, cents: i64, day: jiff::civil::Date) -> i64 {
        if !self.shapes.contains(&Shape::FeeFolded) || day >= self.today.first_of_month() {
            return 0;
        }
        self.fills += 1;
        if self.fills % 3 == 0 {
            (cents / 250).max(1)
        } else {
            0
        }
    }

    fn next_id(&self, kind: &str) -> String {
        format!("gen-{kind}-{}", self.rows.len() + self.conversions.len())
    }

    fn cash(&self, a: usize, c: Cur) -> i64 {
        self.accounts[a].cash.get(&c).copied().unwrap_or(0)
    }

    /// Cash moved, and the line a statement lists it by: its code and description.
    fn move_cash(&mut self, a: usize, c: Cur, cents: i64, day: jiff::civil::Date, code: &'static str, description: String) {
        self.cash_moved(a, c, cents, day);
        self.accounts[a].ledger.push(Line { day, cur: c, cents, code, description, unit: None });
    }

    /// Cash moved on a day, with no line of its own.
    fn cash_moved(&mut self, a: usize, c: Cur, cents: i64, day: jiff::civil::Date) {
        let acct = &mut self.accounts[a];
        let now = acct.cash.get(&c).copied().unwrap_or(0) + cents;
        acct.cash.insert(c, now);
        acct.timeline.entry(c.cash_id().to_string()).or_default().push((day, now));
    }

    /// Whether a brokerage statement is issued for the account (an investment or
    /// a crypto account's), whose forms its lines take; a spending account's take
    /// a cash statement's.
    fn brokerage(&self, a: usize) -> bool {
        matches!(self.accounts[a].role, Role::Invest | Role::Crypto)
    }

    /// What a fill's line states besides its units: whether the account's
    /// statement has lists per currency (a coin account's has none), the fee it
    /// states (a contract's folded fee, a coin account's fee inside the cash
    /// with the folded one), and the price a share paid from another currency.
    fn said(&self, a: usize, l: &Listing, cents: i64, folded: i64, paid_at: Option<i64>) -> Said {
        let lists = self.accounts[a].role != Role::Crypto;
        let fee = match l.sort {
            Sort::Contract => folded,
            Sort::Coin if !lists => cents * COIN_FEE_PER_THOUSAND / 1000 + folded,
            _ => 0,
        };
        Said { lists, fee, paid_at }
    }

    /// A fill's cash moved, `moved` in its currency, and the lines a statement
    /// states it by: two for `SplitFill`'s listing (the cash at no units, the
    /// units at no cash), two for a coin in `Coin2025`'s months (the fill at its
    /// cash before the fee, `amount` being the feed's, and the fee), else one.
    #[allow(clippy::too_many_arguments)]
    fn stated_fill(&mut self, a: usize, listing: usize, buy: bool, units: i64, price: i64, amount: i64, moved: (Cur, i64), said: Said, day: jiff::civil::Date) {
        let l = self.listings[listing].clone();
        let (cur, cents) = moved;
        let code = if buy { "BUY" } else { "SELL" };
        if self.split_on == Some(listing) {
            let verb = if buy { "Bought" } else { "Sold" };
            let head = format!("{} - {}: {verb}", l.symbol, listing_name(&l));
            self.move_cash(a, cur, cents, day, code, format!("{head} 0.0000 shares at ${} per share (executed at {day}), FX Rate: {}", fixed(price, 2), fixed(13_500, 4)));
            self.accounts[a].ledger.push(Line { day, cur, cents: 0, code, description: format!("{head} {} shares at ${} per share (executed at {day})", fixed(units * 10_000, 4), fixed(price, 2)), unit: Some(l.symbol.clone()) });
            return;
        }
        if l.sort == Sort::Coin && self.coin_lists_from.is_some_and(|f| day >= f) {
            let fee = amount * COIN_FEE_PER_THOUSAND / 1000;
            let under = i64::from(buy && self.coin_lists_from == Some(day.first_of_month()));
            let what = format!("{} {} (executed at {day}), FX Rate: {}", fixed(units, 10), l.symbol, fixed(10_000, 4));
            self.cash_moved(a, cur, cents, day);
            let (fill, said, charged) = if buy { (-(amount - fee - under), format!("Purchase of {what}"), format!("Fee for purchase of {what}")) } else { (amount + fee, format!("Sale of {what}"), format!("Trading fee for sale of {what}")) };
            self.accounts[a].ledger.push(Line { day, cur, cents: fill, code, description: said, unit: None });
            self.accounts[a].ledger.push(Line { day, cur, cents: -fee, code: "FEE", description: charged, unit: None });
            return;
        }
        let (code, said) = fill_line(&l, buy, units, day, said);
        self.move_cash(a, cur, cents, day, code, said);
    }

    /// A coin purchase's units past the six places the feed states, in ten-billionths:
    /// nonzero, under half a millionth either way, drawn from a count of their own.
    fn past_the_feed_s_places(&mut self) -> i64 {
        self.coin_buys += 1;
        match (self.coin_buys as i64 * 6_151) % 9_999 - 4_999 {
            0 => 1,
            tail => tail,
        }
    }

    /// A deposit's line: a contribution in a brokerage account.
    fn deposit_line(&self, a: usize, day: jiff::civil::Date) -> (&'static str, String) {
        if self.brokerage(a) {
            ("CONT", format!("Contribution (executed at {day})"))
        } else {
            ("EFT", "Deposit".into())
        }
    }

    /// Whether the next move is listed on both sides: with
    /// `Shape::ArrivalOnly`, every other one is listed by its arrival alone.
    fn listed_whole(&mut self) -> bool {
        self.moves += 1;
        !(self.shapes.contains(&Shape::ArrivalOnly) && self.moves % 2 == 0)
    }

    /// A move of cash between two of the person's accounts: its two rows (the
    /// arrival alone where `both` is false), the cash and the net deposits.
    #[allow(clippy::too_many_arguments)]
    fn transfer(&mut self, from: usize, to: usize, c: Cur, cents: i64, day: jiff::civil::Date, n: &mut u32, both: bool) {
        *n += 1;
        let id = self.next_id("move");
        let group = format!("{id}-x");
        let out = set(self.row("INTERNAL_TRANSFER", "SOURCE", from, at(day, *n), &format!("{id}-out")), &[(&["amount"], text(money(cents))), (&["currency"], text(c.code())), (&["amountSign"], text("negative")), (&["opposingAccountId"], text(&self.accounts[to].id)), (&["externalCanonicalId"], text(&group)), (&["groupId"], text(&group))]);
        let inn = set(self.row("INTERNAL_TRANSFER", "DESTINATION", to, at(day, *n), &format!("{id}-in")), &[(&["amount"], text(money(cents))), (&["currency"], text(c.code())), (&["amountSign"], text("positive")), (&["opposingAccountId"], text(&self.accounts[from].id)), (&["externalCanonicalId"], text(&group)), (&["groupId"], text(&group))]);
        if both {
            self.push(out);
        }
        self.push(inn);
        let (out_code, out_said) = if self.brokerage(from) { ("TRFOUT", format!("Money transfer out of the account (executed at {day})")) } else { ("TRFOUTTF", "Transfer out".into()) };
        let (in_code, in_said) = if self.brokerage(to) { ("TRFIN", format!("Money transfer into the account (executed at {day})")) } else { ("TRFINTF", "Transfer in".into()) };
        self.move_cash(from, c, -cents, day, out_code, out_said);
        self.move_cash(to, c, cents, day, in_code, in_said);
        let cad = if c == Cur::Usd { cents * 135 / 100 } else { cents };
        self.net_deposit(from, -cad, day);
        self.net_deposit(to, cad, day);
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
            fields.push((&["contractType"], text(if l.put { "put" } else { "call" })));
        }
        set(row, &fields)
    }

    /// Cash enough for `cents` of `c` in account `a`: a deposit of CAD, and
    /// for another currency a conversion from it, before what needs it.
    fn fund(&mut self, a: usize, c: Cur, cents: i64, day: jiff::civil::Date, n: &mut u32) {
        self.fund_counted(a, c, cents, day, n, true);
    }

    /// The same, its conversion counted among those `FundingIntent` takes every
    /// other one of where `counted`, else never a funding intent.
    fn fund_counted(&mut self, a: usize, c: Cur, cents: i64, day: jiff::civil::Date, n: &mut u32, counted: bool) {
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
        let (code, said) = self.deposit_line(a, day);
        self.move_cash(a, Cur::Cad, cad, day, code, said);
        self.net_deposit(a, cad, day);
        if c == Cur::Usd {
            // every other one a funding intent, whose detail states the side received alone
            self.funded += usize::from(counted);
            let funding = counted && self.shapes.contains(&Shape::FundingIntent) && self.funded % 2 == 0;
            self.conversion(a, day, n, want, cad, funding);
        }
    }

    /// A conversion in account `a` of `cad` cents for `want` USD cents, its
    /// detail a funding intent where `funding`, else an internal transfer
    /// stating both sides.
    fn conversion(&mut self, a: usize, day: jiff::civil::Date, n: &mut u32, want: i64, cad: i64, funding: bool) {
        *n += 1;
        let id = self.next_id("conversion");
        let detail_id = format!("{id}-x");
        let row = set(self.row("FUNDS_CONVERSION", "", a, at(day, *n), &id), &[(&["amount"], text(money(want))), (&["currency"], text("USD")), (&["amountSign"], text("positive"))]);
        let detail = if funding {
            set(
                self.templates.funding.clone(),
                &[
                    (&["id"], text(&detail_id)),
                    (&["createdAt"], text(at(day, *n))),
                    (&["updatedAt"], text(at(day, *n))),
                    (&["fundableDetails", "fxAdjustedAmount"], text(money(want))),
                    (&["fundableDetails", "targetCurrency"], text("USD")),
                    (&["source", "id"], text(&self.accounts[a].id)),
                    (&["destination", "id"], text(&self.accounts[a].id)),
                ],
            )
        } else {
            set(
                self.templates.conversion.clone(),
                &[
                    (&["id"], text(&detail_id)),
                    (&["amount"], text(money(cad))),
                    (&["currency"], text("CAD")),
                    (&["fxAdjustedAmount"], text(money(want))),
                    (&["fxRate"], text("0.740740")),
                    (&["source_account", "id"], text(&self.accounts[a].id)),
                    (&["source_account", "unifiedAccountType"], text(&self.accounts[a].ty)),
                ],
            )
        };
        self.conversions.insert(detail_id, detail);
        self.push(row);
        // each list names its own currency, with the conversion's own amounts
        let said = |c: Cur| format!("Convert {} (executed at {day}) - ${}USD = ${}CAD", c.code(), money(want), money(cad));
        self.move_cash(a, Cur::Cad, -cad, day, "FXCONVERSION", said(Cur::Cad));
        self.move_cash(a, Cur::Usd, want, day, "FXCONVERSION", said(Cur::Usd));
    }

    /// A conversion made today in `account`, `usd` cents received for CAD at the
    /// made-up broker's rate, whose detail is a funding intent: the CAD deposited
    /// first, both rows listed.
    pub fn convert_today(&mut self, account: &str, usd: i64) {
        self.convert_on_today(account, usd, true);
    }

    /// As `convert_today`, its detail an internal transfer stating both sides.
    pub fn convert_today_stated(&mut self, account: &str, usd: i64) {
        self.convert_on_today(account, usd, false);
    }

    fn convert_on_today(&mut self, account: &str, usd: i64, funding: bool) {
        let a = self.account_index(account).unwrap_or_else(|| panic!("no account {account}"));
        let (day, mut n) = (self.today, 20_000 + self.rows.len() as u32);
        let cad = usd * 135 / 100 + 1;
        n += 1;
        let id = self.next_id("deposit");
        let row = set(self.row("DEPOSIT", "EFT", a, at(day, n), &id), &[(&["amount"], text(money(cad))), (&["currency"], text("CAD")), (&["amountSign"], text("positive"))]);
        self.push(row);
        let (code, said) = self.deposit_line(a, day);
        self.move_cash(a, Cur::Cad, cad, day, code, said);
        self.net_deposit(a, cad, day);
        self.conversion(a, day, &mut n, usd, cad, funding);
    }

    /// A deposit of `cents` CAD into `account` today, the cash there already, its
    /// row listed only once `publish_late` is called.
    pub fn deposit_listed_later(&mut self, account: &str, cents: i64) {
        let a = self.account_index(account).unwrap_or_else(|| panic!("no account {account}"));
        let (day, n) = (self.today, 30_000 + self.rows.len() as u32);
        let id = format!("{}-later", self.next_id("deposit"));
        let row = set(self.row("DEPOSIT", "EFT", a, at(day, n), &id), &[(&["amount"], text(money(cents))), (&["currency"], text("CAD")), (&["amountSign"], text("positive"))]);
        self.held.push(row);
        let (code, said) = self.deposit_line(a, day);
        self.move_cash(a, Cur::Cad, cents, day, code, said);
        self.net_deposit(a, cents, day);
    }

    fn apply(&mut self, r: &mut Rng, day: jiff::civil::Date, p: Plan, n: &mut u32) {
        match p {
            Plan::Buy { account, listing, units } => {
                let l = self.listings[listing].clone();
                // a coin's units executed, past the six places the feed states them to
                let units = if l.sort == Sort::Coin { units + self.past_the_feed_s_places() } else { units };
                // `SplitFill`'s listing at its own price, drawing nothing: the book without it stays as it is
                let own = self.split_on == Some(listing);
                let price = if own { l.price } else { (l.price + r.range(-l.price / 20, l.price / 20)).max(1) };
                let cost = l.cost(units, price).max(1);
                // a US share paid out of CAD at the broker's rate, 135 cents a dollar, half a cent up
                let fx_funded = self.shapes.contains(&Shape::FxFunded) && l.sort == Sort::Share && l.cur == Cur::Usd && day < self.today.first_of_month() && {
                    self.us_buys += 1;
                    self.us_buys % 4 == 0
                };
                let fee = if fx_funded { 0 } else { self.folded_fee(cost, day) };
                let paid = if fx_funded { (cost * 135 + 50) / 100 } else { cost + fee };
                let paid_in = if fx_funded { Cur::Cad } else { l.cur };
                self.fund_counted(account, paid_in, paid, day, n, !own);
                *n += 1;
                let id = self.next_id("buy");
                let (ty, sub) = match l.sort {
                    Sort::Share => ("DIY_BUY", "LIMIT_ORDER"),
                    Sort::Coin => ("CRYPTO_BUY", "MARKET_ORDER"),
                    Sort::Contract => ("OPTIONS_BUY", "LIMIT_ORDER"),
                };
                let row = self.row(ty, sub, account, at(day, *n), &id);
                let row = set(self.with_security(row, listing), &[(&["amount"], text(money(cost))), (&["currency"], text(l.cur.code())), (&["assetQuantity"], text(feed_units(units, &l))), (&["status"], text("FILLED"))]);
                self.buys += 1;
                if self.shapes.contains(&Shape::FeedOmits) && self.buys % 7 == 0 && day < self.today.first_of_month() {
                    self.omitted.push((account, listing, day, row));
                } else {
                    self.push(row);
                }
                let said = self.said(account, &l, cost, fee, fx_funded.then_some(price));
                self.stated_fill(account, listing, true, units, price, cost, (paid_in, -paid), said, day);
                self.move_units(account, listing, units, day);
            }
            Plan::Sell { account, listing, all } => {
                let l = self.listings[listing].clone();
                let held = self.accounts[account].held.get(&l.id).copied().unwrap_or(0);
                if held <= 0 {
                    return;
                }
                let units = if all { held } else { (held / 2).max(1) };
                // in the coin account's later form a sale is of units to the feed's six places,
                // which both state alike, what is past them left held
                let units = if l.sort == Sort::Coin && self.coin_lists_from.is_some_and(|f| day >= f) { units - units % 10_000 } else { units };
                if units <= 0 {
                    return;
                }
                let price = if self.split_on == Some(listing) { l.price + l.price / 10 } else { (l.price + r.range(-l.price / 5, l.price / 5)).max(1) };
                let proceeds = l.cost(units, price);
                let fee = if proceeds > 1 { self.folded_fee(proceeds, day) } else { 0 };
                // a purchase the feed left out, sold the same day: no position states it, so it is listed
                if let Some(i) = self.omitted.iter().position(|(a, li, d, _)| *a == account && *li == listing && *d == day) {
                    let (_, _, _, row) = self.omitted.remove(i);
                    self.push(row);
                }
                *n += 1;
                let id = self.next_id("sell");
                // a coin's limit sell the feed dates the day it was placed, days before it executed
                let placed = if self.shapes.contains(&Shape::LimitOrder) && l.sort == Sort::Coin && day < self.today.first_of_month() {
                    self.coin_sales += 1;
                    let placed = day.checked_sub(jiff::Span::new().days(9 + (self.coin_sales as i64 / 3) % 4)).expect("a day");
                    // only a holding that never fell from before the order to its execution: the
                    // feed's early date then sells nothing the account did not hold at that moment
                    let before = placed.yesterday().expect("a day");
                    let mut held = self.held_on(account, before).get(&l.id).copied().unwrap_or(0);
                    let enough = held >= units;
                    let never_fell = self.accounts[account].timeline.get(&l.id).is_none_or(|c| {
                        c.iter().filter(|(d, _)| *d >= placed && *d <= day).all(|(_, q)| {
                            let up = *q >= held;
                            held = *q;
                            up
                        })
                    });
                    (self.coin_sales % 3 == 0 && enough && never_fell).then_some(placed)
                } else {
                    None
                };
                let (ty, sub) = match l.sort {
                    Sort::Share => ("DIY_SELL", "LIMIT_ORDER"),
                    Sort::Coin if placed.is_some() => ("CRYPTO_SELL", "LIMIT_ORDER"),
                    Sort::Coin => ("CRYPTO_SELL", "MARKET_ORDER"),
                    Sort::Contract => ("OPTIONS_SELL", "LIMIT_ORDER"),
                };
                let row = self.row(ty, sub, account, at(placed.unwrap_or(day), *n), &id);
                let row = set(self.with_security(row, listing), &[(&["amount"], text(money(proceeds))), (&["currency"], text(l.cur.code())), (&["assetQuantity"], text(feed_units(units, &l))), (&["status"], text("FILLED"))]);
                self.push(row);
                let said = self.said(account, &l, proceeds, fee, None);
                self.stated_fill(account, listing, false, units, price, proceeds, (l.cur, proceeds - fee), said, day);
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
                self.move_cash(account, l.cur, cents, day, "DIV", dividend_line(&l, day));
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
                let row = set(self.with_security(row, listing), &[(&["assetQuantity"], text(feed_units(units, &self.listings[listing])))]);
                self.push(row);
                self.move_units(account, listing, units, day);
            }
            Plan::Move { from, to } => {
                let cents = self.cash(from, Cur::Cad) / 3;
                if cents < 100 || from == to {
                    return;
                }
                let both = self.listed_whole();
                self.transfer(from, to, Cur::Cad, cents, day, n, both);
            }
            Plan::MoveHolding { from, to } => {
                if from == to {
                    return;
                }
                // one share holding moved whole, on a day no other move touches these accounts
                let kept: Vec<String> = [self.written_on, self.split_on].into_iter().flatten().map(|w| self.listings[w].id.clone()).collect();
                let Some((lid, units)) = self.accounts[from].held.iter().find(|(id, _)| id.starts_with("sec-s-") && !kept.contains(id)).map(|(i, u)| (i.clone(), *u)) else { return };
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
                if self.listed_whole() {
                    self.push(out);
                }
                self.push(inn);
                self.move_units(from, listing, -units, day);
                self.move_units(to, listing, units, day);
            }
            Plan::Deposit { account, cents } => {
                *n += 1;
                let id = self.next_id("deposit");
                let row = set(self.row("DEPOSIT", "EFT", account, at(day, *n), &id), &[(&["amount"], text(money(cents))), (&["currency"], text("CAD")), (&["amountSign"], text("positive"))]);
                self.push(row);
                let (code, said) = self.deposit_line(account, day);
                self.move_cash(account, Cur::Cad, cents, day, code, said);
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
                let (code, said) = if self.brokerage(account) { ("WD", format!("Withdrawal (executed at {day})")) } else { ("EFTOUT", "Withdrawal".into()) };
                self.move_cash(account, Cur::Cad, -cents, day, code, said);
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
                let (code, said) = if self.brokerage(account) { ("FPLINT", "Stock lending monthly interest payment".into()) } else { ("INT", "Interest earned".into()) };
                self.move_cash(account, Cur::Cad, cents, day, code, said);
            }
            Plan::CardPurchase { account, cents } => {
                *n += 1;
                let id = self.next_id("card");
                let row = set(self.row("CREDIT_CARD", "PURCHASE", account, at(day, *n), &id), &[(&["amount"], text(money(cents))), (&["currency"], text("CAD")), (&["amountSign"], text("negative")), (&["spendMerchant"], text(format!("gen-merchant-{}", r.below(40))))]);
                self.push(row);
                self.move_cash(account, Cur::Cad, -cents, day, "PURCHASE", "Purchase".into());
            }
            #[cfg(feature = "demo")]
            Plan::BuyAt { account, listing, units, cents } | Plan::SellAt { account, listing, units, cents } => {
                let buy = matches!(p, Plan::BuyAt { .. });
                let l = self.listings[listing].clone();
                *n += 1;
                let id = self.next_id(if buy { "buy" } else { "sell" });
                let (ty, sub) = match (l.sort, buy) {
                    (Sort::Share, true) => ("DIY_BUY", "LIMIT_ORDER"),
                    (Sort::Coin, true) => ("CRYPTO_BUY", "MARKET_ORDER"),
                    (Sort::Contract, true) => ("OPTIONS_BUY", "LIMIT_ORDER"),
                    (Sort::Share, false) => ("DIY_SELL", "LIMIT_ORDER"),
                    (Sort::Coin, false) => ("CRYPTO_SELL", "MARKET_ORDER"),
                    (Sort::Contract, false) => ("OPTIONS_SELL", "LIMIT_ORDER"),
                };
                let row = self.row(ty, sub, account, at(day, *n), &id);
                let row = set(self.with_security(row, listing), &[(&["amount"], text(money(cents))), (&["currency"], text(l.cur.code())), (&["assetQuantity"], text(feed_units(units, &l))), (&["status"], text("FILLED"))]);
                self.push(row);
                let said = self.said(account, &l, cents, 0, None);
                let (code, said) = fill_line(&l, buy, units, day, said);
                self.move_cash(account, l.cur, if buy { -cents } else { cents }, day, code, said);
                self.move_units(account, listing, if buy { units } else { -units }, day);
            }
            #[cfg(feature = "demo")]
            Plan::DividendOf { account, listing, units, cents } => {
                let l = self.listings[listing].clone();
                *n += 1;
                let id = self.next_id("dividend");
                let row = self.row("DIVIDEND", "DIY_DIVIDEND", account, at(day, *n), &id);
                let row = set(self.with_security(row, listing), &[(&["amount"], text(money(cents))), (&["currency"], text(l.cur.code())), (&["assetQuantity"], text(feed_units(units, &l)))]);
                self.push(row);
                self.move_cash(account, l.cur, cents, day, "DIV", dividend_line(&l, day));
            }
            #[cfg(feature = "demo")]
            Plan::StakingOf { account, listing, units } => {
                *n += 1;
                let id = self.next_id("staking");
                let row = self.row("CRYPTO_STAKING_REWARD", "", account, at(day, *n), &id);
                let row = set(self.with_security(row, listing), &[(&["assetQuantity"], text(feed_units(units, &self.listings[listing])))]);
                self.push(row);
                self.move_units(account, listing, units, day);
            }
            #[cfg(feature = "demo")]
            Plan::Charge { account, cur, cents } => {
                *n += 1;
                let id = self.next_id("charge");
                let row = set(
                    self.row("INTEREST_CHARGE", "MARGIN_INTEREST", account, at(day, *n), &id),
                    &[(&["amount"], text(money(cents))), (&["currency"], text(cur.code())), (&["amountSign"], text("negative")), (&["securityId"], text(cur.cash_id())), (&["security", "id"], text(cur.cash_id())), (&["security", "stock", "symbol"], text(cur.code())), (&["assetSymbol"], text(cur.code()))],
                );
                self.push(row);
                self.move_cash(account, cur, -cents, day, "INTCHARGED", "Margin Interest Charges for Example Period".into());
            }
            Plan::Retire { from, to, keep } => {
                // every holding sold
                let held: Vec<usize> = self.accounts[from].held.iter().filter(|(_, q)| **q > 0).filter_map(|(id, _)| self.listings.iter().position(|l| &l.id == id)).collect();
                for listing in held {
                    *n += 1;
                    self.apply(r, day, Plan::Sell { account: from, listing, all: true }, n);
                }
                // its cash moved, `keep` of its CAD left behind
                for c in [Cur::Cad, Cur::Usd] {
                    let cents = self.cash(from, c) - if c == Cur::Cad { keep } else { 0 };
                    if cents > 0 {
                        let both = self.listed_whole();
                        self.transfer(from, to, c, cents, day, n, both);
                    }
                }
                self.accounts[from].closed = Some(day);
            }
            Plan::WriteCalls { account, listing, units } => {
                let l = self.listings[listing].clone();
                let price = (l.price + r.range(-l.price / 20, l.price / 20)).max(1);
                let fee = CONTRACT_FEE_CENTS * units;
                let net = (l.cost(units, price) - fee).max(1);
                *n += 1;
                let id = self.next_id("sell");
                let row = self.row("OPTIONS_SELL", "LIMIT_ORDER", account, at(day, *n), &id);
                let row = set(self.with_security(row, listing), &[(&["amount"], text(money(net))), (&["currency"], text(l.cur.code())), (&["assetQuantity"], text(feed_units(units, &l))), (&["status"], text("FILLED"))]);
                self.push(row);
                let (_, said) = fill_line(&l, false, units, day, Said { lists: true, fee, paid_at: None });
                self.move_cash(account, l.cur, net, day, "SELLTOOPEN", said);
                self.move_units(account, listing, -units, day);
            }
            Plan::Assign { account, listing } => {
                let l = self.listings[listing].clone();
                let contracts = -self.accounts[account].held.get(&l.id).copied().unwrap_or(0);
                let (Some((under, expiry, strike)), true) = (l.contract.clone(), contracts > 0) else { return };
                let under = self.listings.iter().position(|x| x.id == under).expect("a contract's underlying");
                let shares = contracts * SHARES_A_CONTRACT;
                let cents = strike * shares;
                *n += 1;
                let id = self.next_id("assignment");
                let row = self.row("OPTIONS_ASSIGN", "AUTO_ASSIGN", account, format!("{day}T21:30:00.000000+00:00"), &id);
                let row = set(self.with_security(row, listing), &[(&["amount"], text(money(cents))), (&["amountSign"], text("positive")), (&["currency"], text("USD")), (&["assetQuantity"], text(contracts.to_string()))]);
                self.push(row);
                // the contracts and the shares leave at the assignment, the cash with them
                self.move_units(account, listing, contracts, day);
                self.move_units(account, under, -shares, day);
                self.cash_moved(account, l.cur, cents, day);
                // the statement: the contracts assigned at no cash, the shares sold at the strike days after
                let symbol = l.symbol.clone();
                let right = if l.put { "PUT" } else { "CALL" };
                self.accounts[account].ledger.push(Line { day, cur: l.cur, cents: 0, code: "ASSIGN", description: format!("{symbol} {} USD {right} {expiry}: Assigned -{contracts} contract (executed at {day})", fixed(strike, 2)), unit: Some(symbol) });
                let settled = day.checked_add(jiff::Span::new().days(r.range(1, 4))).expect("a day");
                let (_, said) = fill_line(&self.listings[under], false, shares, day, Said { lists: true, fee: 0, paid_at: None });
                self.accounts[account].ledger.push(Line { day: settled, cur: l.cur, cents, code: "SELL", description: said, unit: None });
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
                self.move_cash(from, Cur::Cad, -owed, day, "AFT_OUT", "Credit card payment".into());
                self.move_cash(card, Cur::Cad, owed, day, "PAYMENT", "Payment".into());
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
        let name = listing_name(l);
        let mut fields: Vec<(&[&str], Value)> = vec![(&["id"], text(&l.id)), (&["currency"], text(l.cur.code())), (&["securityType"], text(ty)), (&["stock", "symbol"], text(&l.symbol)), (&["stock", "name"], text(name))];
        if l.sort == Sort::Share {
            let (exchange, mic) = match &l.venue {
                Some((e, m)) => (e.clone(), m.clone()),
                None if l.cur == Cur::Cad => ("TSX".into(), "XTSE".into()),
                None => ("NASDAQ".into(), "XNAS".into()),
            };
            fields.push((&["stock", "primaryExchange"], text(exchange)));
            fields.push((&["stock", "primaryMic"], text(mic)));
            fields.push((&["status"], text(l.status)));
        }
        if let Some((under, expiry, strike)) = &l.contract {
            fields.push((&["optionDetails", "expiryDate"], text(expiry.to_string())));
            fields.push((&["optionDetails", "strikePrice"], text(money(*strike))));
            let (right, letter) = if l.put { ("PUT", 'P') } else { ("CALL", 'C') };
            fields.push((&["optionDetails", "optionType"], text(right)));
            fields.push((&["optionDetails", "multiplier"], Value::Number(SHARES_A_CONTRACT.to_string())));
            fields.push((&["optionDetails", "osiSymbol"], text(format!("{:<6}{}{letter}{:08}", l.symbol, expiry.strftime("%y%m%d"), strike * 10))));
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
                // a contract sold to open is held short, its units below nothing
                (&["direction"], text(if q < 0 { "SHORT" } else { "LONG" })),
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

    /// An account's cash now, per currency, as its balances state it.
    pub fn cash_of(&self, account: &str) -> BTreeMap<String, Dec> {
        let Some(a) = self.account_index(account) else { return BTreeMap::new() };
        self.accounts[a].cash.iter().filter(|(_, q)| **q != 0).map(|(c, q)| (c.code().to_string(), Dec::new(*q, 2).expect("cents fit"))).collect()
    }

    /// The months an account's monthly statement is issued for, with the
    /// `Statements` shape: each month it was open that has ended. None for an
    /// account no brokerage statement covers (spending, a card).
    pub fn statement_months(&self, account: &str) -> Vec<jiff::civil::Date> {
        let Some(a) = self.account_index(account) else { return vec![] };
        let acct = &self.accounts[a];
        if !self.shapes.contains(&Shape::Statements) || !matches!(acct.role, Role::Invest | Role::Crypto) {
            return vec![];
        }
        let mut out = vec![];
        let mut m = acct.opened.first_of_month();
        while m < self.today.first_of_month() && acct.closed.is_none_or(|c| m <= c.first_of_month()) {
            out.push(m);
            m = m.checked_add(jiff::Span::new().months(1)).expect("a month");
        }
        out
    }

    /// Where an account's statement closes a month in a currency: every movement
    /// its statements list to the month's end.
    pub fn statement_closing(&self, account: &str, currency: &str, month: jiff::civil::Date) -> Dec {
        let Some(a) = self.account_index(account) else { return Dec::ZERO };
        let end = month.last_of_month();
        let cents: i64 = self.accounts[a].ledger.iter().filter(|l| l.cur.code() == currency && l.day <= end).map(|l| l.cents).sum();
        Dec::new(cents, 2).expect("cents fit")
    }

    /// An account's units now, per symbol (a listing retired by a corporate
    /// action trades on under another id, the same symbol), in each listing's
    /// own scale.
    pub fn units_of(&self, account: &str) -> BTreeMap<String, Dec> {
        let Some(a) = self.account_index(account) else { return BTreeMap::new() };
        let mut out: BTreeMap<String, Dec> = BTreeMap::new();
        for (id, q) in &self.accounts[a].held {
            let Some(l) = self.listings.iter().find(|l| &l.id == id) else { continue };
            // a contract by its terms, as the book names it
            let key = match &l.contract {
                Some((_, expiry, strike)) => format!("{} {expiry} {} {}", l.symbol, money(*strike), if l.put { "P" } else { "C" }),
                None => l.symbol.clone(),
            };
            let e = out.entry(key).or_insert(Dec::ZERO);
            *e = e.checked_add(Dec::new(*q, l.scale()).expect("units fit")).expect("units add");
        }
        out
    }

    /// Every account's id.
    pub fn account_ids(&self) -> Vec<String> {
        self.accounts.iter().map(|a| a.id.clone()).collect()
    }

    /// The conversions whose detail is a funding intent: each one's row id.
    pub fn funding_intents(&self) -> Vec<String> {
        self.rows.iter().filter(|r| Node::root(r).text("type").ok() == Some("FUNDS_CONVERSION")).filter(|r| Node::root(r).text("externalCanonicalId").ok().and_then(|x| self.conversions.get(x)).is_some_and(|d| Node::root(d).field("fundableType").is_ok())).filter_map(|r| Node::root(r).text("canonicalId").ok().map(str::to_string)).collect()
    }
}

/// A listing's name, as its security record states it.
fn listing_name(l: &Listing) -> String {
    l.name.clone().unwrap_or_else(|| format!("{} Generated Listing", l.symbol))
}

/// A dividend's statement line, its record date a week before it is paid.
fn dividend_line(l: &Listing, day: jiff::civil::Date) -> String {
    let record = day.checked_sub(jiff::Span::new().days(7)).expect("a day");
    format!("{} - {}: Cash dividend distribution, received on {day}, record date of {record}", l.symbol, listing_name(l))
}

/// `mantissa` with `places` decimals, every one written: a statement writes a
/// share's units to 4, a contract's fee to 4 and a rate to 4, a coin's units to
/// 10, a strike and a price to 2.
fn fixed(mantissa: i64, places: u32) -> String {
    let unit = 10_i64.pow(places);
    let sign = if mantissa < 0 { "-" } else { "" };
    let m = mantissa.abs();
    format!("{sign}{}.{:0width$}", m / unit, m % unit, width = places as usize)
}

/// What a fill's line states besides its units (`Generated::said`).
struct Said {
    lists: bool,
    fee: i64,
    paid_at: Option<i64>,
}

/// A fill's statement line: its code and description, in the forms the
/// recorded statements write (`tests/replies/wealthsimple-statements`, read by
/// `broker::codes::fill_of`): `SYM - NAME: Bought 4.0000 shares (executed at
/// D)`, with `at $P per share` and `, FX Rate: R` where it was paid from another
/// currency; `Purchase of N SYM (executed at D), FX Rate: R`, with `, Fee charged
/// $F` in a coin account; `SYM K USD CALL D: Sold 1 contract (executed at D)`,
/// with `, Fee: $F` where it states one.
fn fill_line(l: &Listing, buy: bool, units: i64, day: jiff::civil::Date, said: Said) -> (&'static str, String) {
    let verb = if buy { "Bought" } else { "Sold" };
    match l.sort {
        Sort::Share => {
            let at = said.paid_at.map(|p| format!(" at ${} per share", fixed(p, 2))).unwrap_or_default();
            // the rate as the statement writes it, the made-up broker's 135 cents a dollar
            let rate = if said.paid_at.is_some() { format!(", FX Rate: {}", fixed(13_500, 4)) } else { String::new() };
            // a Canadian listing the feed calls `SYM.TO` the statement calls `SYM` (`Shape::Interlisted`)
            let symbol = l.symbol.strip_suffix(".TO").unwrap_or(&l.symbol);
            (if buy { "BUY" } else { "SELL" }, format!("{symbol} - {}: {verb} {} shares{at} (executed at {day}){rate}", listing_name(l), fixed(units * 10_000, 4)))
        }
        // a coin's units to ten places; its rate is to CAD, which every made-up coin is priced in
        Sort::Coin => {
            let fee = if said.lists { String::new() } else { format!(", Fee charged ${}", fixed(said.fee, 2)) };
            (if buy { "BUY" } else { "SELL" }, format!("{} of {} {} (executed at {day}), FX Rate: {}{fee}", if buy { "Purchase" } else { "Sale" }, fixed(units, 10), l.symbol, fixed(10_000, 4)))
        }
        Sort::Contract => {
            let (expiry, strike) = l.contract.as_ref().map(|(_, e, s)| (*e, *s)).expect("a contract's terms");
            let fee = if said.fee > 0 { format!(", Fee: ${}", fixed(said.fee * 100, 4)) } else { String::new() };
            (if buy { "BUYTOOPEN" } else { "SELLTOCLOSE" }, format!("{} {} USD {} {expiry}: {verb} {units} contract (executed at {day}){fee}", l.symbol, fixed(strike, 2), if l.put { "PUT" } else { "CALL" }))
        }
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
                // what it is worth now: the last day it states, else its holdings at their price
                let worth = match a.days.as_ref().and_then(|d| d.last()) {
                    Some((_, value, _)) => *value,
                    None => self.worth(i, self.today),
                };
                set(
                    self.templates.account.clone(),
                    &[
                        (&["id"], text(&a.id)),
                        (&["unifiedAccountType"], text(&a.ty)),
                        (&["nickname"], text(a.nickname.clone().unwrap_or_else(|| format!("Generated {i}")))),
                        (&["currency"], text(a.currency.code())),
                        (&["createdAt"], text(format!("{}T12:00:00.000000Z", a.opened))),
                        (&["status"], text(if a.closed.is_some() { "closed" } else { "open" })),
                        (&["linkedAccount"], match a.linked {
                            Some(l) => Value::Object([("id".to_string(), text(&self.accounts[l].id))].into()),
                            None => Value::Null,
                        }),
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
        self.asked_activity.push((account.to_string(), from));
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

    fn net_worth_history(&mut self, _accounts: &[String], _outside: &[String], _start: jiff::civil::Date, _end: jiff::civil::Date) -> Answer<Option<Vec<Value>>> {
        // the made-up person holds nothing outside: no history of it to read
        self.asked += 1;
        Ok(None)
    }

    fn net_worth(&mut self) -> Answer<Option<Value>> {
        // the made-up person holds nothing outside Wealthsimple
        self.asked += 1;
        json::parse(r#"{"id":"identity-generated","netWorth":{"externalFinancialEntities":[]}}"#).map(Some).map_err(|e| Failure::Mismatch(e.to_string()))
    }

    fn buying_power(&mut self, account: &str) -> Answer<Value> {
        self.asked += 1;
        let a = self.account_index(account).ok_or_else(|| Failure::Refused(format!("no account {account}")))?;
        let cad = self.accounts[a].buying_power.unwrap_or_else(|| self.cash(a, Cur::Cad) + self.cash(a, Cur::Usd) * 135 / 100);
        let reply = format!(r#"{{"id":"{account}","financials":{{"current":{{"id":"c","marginV3":{{"trading":{{"buyingPower":{{"__typename":"BuyingPowerMetricAvailable","total":{{"amount":"{}","currency":"CAD"}}}},"__typename":"MarginTrading"}},"__typename":"MarginV3"}},"__typename":"Current"}},"__typename":"Financials"}},"__typename":"Account"}}"#, money(cad));
        json::parse(&reply).map_err(|e| Failure::Mismatch(e.to_string()))
    }

    fn margin_rates(&mut self, account: &str) -> Answer<Value> {
        self.asked += 1;
        self.account_index(account).ok_or_else(|| Failure::Refused(format!("no account {account}")))?;
        // the made-up person's rates, the same for every margin account
        let reply = format!(r#"{{"id":"{account}","appliedRates":{{"__typename":"AppliedMarginInterestRates","cadPositionRate":"0.0545","usdPositionRate":"0.0745"}},"__typename":"Account"}}"#);
        json::parse(&reply).map_err(|e| Failure::Mismatch(e.to_string()))
    }

    fn history(&mut self, account: &str, from: Option<jiff::civil::Date>) -> Answer<Vec<Value>> {
        self.asked += 1;
        let a = self.account_index(account).ok_or_else(|| Failure::Refused(format!("no account {account}")))?;
        let acct = &self.accounts[a];
        let m = |cents: i64| Value::Object([("amount".to_string(), text(money(cents))), ("cents".to_string(), Value::Number(cents.to_string())), ("currency".to_string(), text("CAD"))].into());
        // the days stated, where they are
        if let Some(days) = &acct.days {
            return Ok(days
                .iter()
                .filter(|(d, _, _)| from.is_none_or(|f| *d >= f) && *d <= self.today)
                .map(|(d, value, deposits)| set(self.templates.history.clone(), &[(&["date"], text(d.to_string())), (&["netDepositsV2"], m(*deposits)), (&["netLiquidationValueV2"], m(*value))]))
                .collect());
        }
        let mut day = from.unwrap_or(acct.opened).max(acct.opened);
        let mut out = Vec::new();
        while day <= self.today {
            let deposits: i64 = acct.deposits.iter().filter(|(d, _)| *d <= day).map(|(_, c)| c).sum();
            let worth = self.worth(a, day);
            out.push(set(self.templates.history.clone(), &[(&["date"], text(day.to_string())), (&["netDepositsV2"], m(deposits)), (&["netLiquidationValueV2"], m(worth))]));
            day = day.tomorrow().map_err(|e| Failure::Mismatch(e.to_string()))?;
        }
        Ok(out)
    }

    fn statement(&mut self, account: &str, month: jiff::civil::Date, kind: &str) -> Answer<Option<Value>> {
        self.asked += 1;
        // a spending account's cash statement is not made up: no shape moves its cash
        if !self.shapes.contains(&Shape::Statements) || kind != crate::statement::BROKERAGE {
            return Ok(None);
        }
        let a = self.account_index(account).ok_or_else(|| Failure::Refused(format!("no account {account}")))?;
        let acct = &self.accounts[a];
        // issued for each month it was open that has ended
        if month >= self.today.first_of_month() || month < acct.opened.first_of_month() || acct.closed.is_some_and(|c| month > c.first_of_month()) {
            return Ok(None);
        }
        let end = month.last_of_month();
        // a coin account's statement states its rows in its own currency, with no lists per
        // currency, until its later form (`Coin2025`)
        let lists = acct.role != Role::Crypto || self.coin_lists_from.is_some_and(|f| month >= f);
        let row = |l: &Line, balance: i64| -> Value {
            Value::Object(
                [
                    ("transactionDate".to_string(), text(l.day.to_string())),
                    ("transactionType".to_string(), text(l.code)),
                    ("description".to_string(), text(&l.description)),
                    ("cashMovement".to_string(), text(money(l.cents))),
                    ("balance".to_string(), text(money(balance))),
                    // a coin account's statement names no unit; another's names the
                    // currency for cash, and a symbol for a row of units at no cash
                    ("unit".to_string(), match (&l.unit, lists) {
                        (_, false) => Value::Null,
                        (Some(symbol), true) => text(symbol),
                        (None, true) => text(format!("${}", l.cur.code())),
                    }),
                    ("__typename".to_string(), text("BrokerageMonthlyStatementTransactions")),
                ]
                .into(),
            )
        };
        // each currency's movements in the month, with the running balance
        let mut per_currency: Vec<(Cur, Vec<Value>)> = Vec::new();
        for c in [Cur::Cad, Cur::Usd] {
            let mut balance = 0;
            let mut list = Vec::new();
            for l in acct.ledger.iter().filter(|l| l.cur == c && l.day <= end) {
                balance += l.cents;
                if l.day >= month {
                    list.push(row(l, balance));
                }
            }
            per_currency.push((c, list));
        }
        let own = per_currency.iter().find(|(c, _)| *c == acct.currency).map(|(_, l)| l.clone()).unwrap_or_default();
        if !lists {
            assert!(per_currency.iter().all(|(c, l)| *c == acct.currency || l.is_empty()), "a coin account moves its own currency alone");
        }
        let per = if lists {
            Value::Array(
                per_currency
                    .into_iter()
                    .map(|(c, list)| Value::Object([("currency".to_string(), text(c.code())), ("currentTransactions".to_string(), Value::Array(list)), ("__typename".to_string(), text("BrokerageMonthlyStatementActivitiesPerCurrency"))].into()))
                    .collect(),
            )
        } else {
            Value::Null
        };
        Ok(Some(set(
            self.templates.statement.clone(),
            &[
                (&["id"], text(format!("{}-{month}", acct.custodian))),
                (&["data", "custodianAccountId"], text(&acct.custodian)),
                (&["data", "isMultiCurrency"], Value::Bool(lists)),
                (&["data", "currentTransactions"], Value::Array(own)),
                (&["data", "activitiesPerCurrency"], per),
            ],
        )))
    }

    fn requests(&self) -> usize {
        self.asked
    }
}
