//! `bagholder demo <data folder>`: a made-up book for the browser tests and the
//! README screenshots, made the way the app makes one: a made-up Wealthsimple
//! (`bagholder_wealthsimple::generated::Generated::from_records`) pulled into a
//! fresh book through the real adapter and pull, the journal written through the
//! figures as a page writes it, and the facts and prices the engine reads written
//! beside it (`demo_facts`). Four accounts, shares on both sides of the border,
//! covered calls, long options, dividends, crypto with staking, and two and a half
//! years of daily equity. Nothing in it is anyone's; every value is made up here.
//! A developer's command: built only with the `demo` feature, never in a release.

use std::collections::BTreeMap;
use std::path::Path;

use bagholder_book::Book;
use bagholder_broker::pull;
use bagholder_core::account::AccountRef;
use bagholder_core::instrument::{RefScheme, Reference};
use bagholder_core::jiff::civil::{Date, Weekday};
use bagholder_core::jiff::Timestamp;
use bagholder_core::journal::{Grade, JournalEntry};
use bagholder_core::Broker;
use bagholder_engine::trades::TradeKey;
use bagholder_wealthsimple::adapter::Wealthsimple;
use bagholder_wealthsimple::generated::{AccountRecord, Generated, ListingRecord, ListingSort, Record};

/// The demo's today and the moment it was read.
pub const TODAY: &str = "2026-09-08";
pub const SYNCED: &str = "2026-09-08T20:05:00Z";

/// nickname, id, currency, Wealthsimple's account type, and the account's share
/// of the book's daily value and net deposits
const ACCOUNTS: [(&str, &str, &str, &str, f64); 4] = [
    ("TFSA", "acct-tfsa", "CAD", "SELF_DIRECTED_TFSA", 0.38),
    ("RRSP", "acct-rrsp", "CAD", "SELF_DIRECTED_RRSP", 0.41),
    ("Trading", "acct-trading", "USD", "SELF_DIRECTED_NON_REGISTERED_MARGIN", 0.08),
    ("Crypto", "acct-crypto", "CAD", "SELF_DIRECTED_CRYPTO", 0.13),
];

/// What the margin account can borrow, in CAD.
const BUYING_POWER: (&str, &str) = ("acct-trading", "41250.80");

/// symbol, name, exchange, mic, currency
const LISTINGS: [(&str, &str, &str, &str, &str); 23] = [
    ("XEQT", "iShares Core Equity ETF Portfolio", "TSX", "XTSE", "CAD"),
    ("VFV", "Vanguard S&P 500 Index ETF", "TSX", "XTSE", "CAD"),
    ("ENB", "Enbridge Inc.", "TSX", "XTSE", "CAD"),
    ("TD", "Toronto-Dominion Bank", "TSX", "XTSE", "CAD"),
    ("SHOP", "Shopify Inc.", "TSX", "XTSE", "CAD"),
    ("BCE", "BCE Inc.", "TSX", "XTSE", "CAD"),
    ("CNQ", "Canadian Natural Resources", "TSX", "XTSE", "CAD"),
    ("AAPL", "Apple Inc.", "NASDAQ", "XNAS", "USD"),
    ("MSFT", "Microsoft Corp.", "NASDAQ", "XNAS", "USD"),
    ("NVDA", "NVIDIA Corp.", "NASDAQ", "XNAS", "USD"),
    ("INTC", "Intel Corp.", "NASDAQ", "XNAS", "USD"),
    ("SOFI", "SoFi Technologies", "NASDAQ", "XNAS", "USD"),
    ("RIVN", "Rivian Automotive", "NASDAQ", "XNAS", "USD"),
    ("HOOD", "Robinhood Markets", "NASDAQ", "XNAS", "USD"),
    ("COIN", "Coinbase Global", "NASDAQ", "XNAS", "USD"),
    ("UBER", "Uber Technologies", "NYSE", "XNYS", "USD"),
    ("TSLA", "Tesla Inc.", "NASDAQ", "XNAS", "USD"),
    ("PLTR", "Palantir Technologies", "NASDAQ", "XNAS", "USD"),
    ("AMD", "Advanced Micro Devices", "NASDAQ", "XNAS", "USD"),
    ("BTC", "Bitcoin", "", "", "CAD"),
    ("ETH", "Ether", "", "", "CAD"),
    ("SOL", "Solana", "", "", "CAD"),
    ("DOGE", "Dogecoin", "", "", "CAD"),
];

/// Listings whose ticker another listing above already has, each by a key of its own
/// (`SYMBOL@VENUE`): a listing is its own security, with its own Wealthsimple id,
/// never found by its ticker (brief 36, 7).
/// key, symbol, name, exchange, mic, currency
const MORE_LISTINGS: [(&str, &str, &str, &str, &str, &str); 1] = [("TD@NYSE", "TD", "Toronto-Dominion Bank", "NYSE", "XNYS", "USD")];

/// The coins: their units are millionths.
const COINS: [&str; 4] = ["BTC", "ETH", "SOL", "DOGE"];

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn day(s: &str) -> Date {
    s.parse().expect("a made-up day")
}

fn ymd(y: i16, m: i8, d: i8) -> Date {
    Date::new(y, m, d).expect("a made-up day")
}

/// A listing's Wealthsimple id, by its key: the ticker, or `SYMBOL@VENUE` for a
/// second listing of one ticker, or an option's name.
fn sec_id(key: &str) -> String {
    format!("sec-{}", key.to_lowercase().replace(' ', "-").replace('.', "_").replace('@', "-"))
}

fn account_id(nick: &str) -> &'static str {
    ACCOUNTS.iter().find(|a| a.0 == nick).expect("a made-up account").1
}

/// A price to the cent, as cents.
fn cents(v: f64) -> i64 {
    (v * 100.0).round() as i64
}

/// A value to the cent as `{:.2}` writes it, as cents.
fn to_cents(v: f64) -> i64 {
    format!("{v:.2}").replace('.', "").parse().expect("a value to the cent")
}

/// Units of a coin as millionths.
fn millionths(v: f64) -> i64 {
    (v * 1_000_000.0).round() as i64
}

/// `a * b / d`, rounded half to even.
fn mul_div(a: i64, b: i64, d: i64) -> i64 {
    let n = a as i128 * b as i128;
    let (q, r) = (n / d as i128, n % d as i128);
    let twice = 2 * r.abs();
    let up = twice > d as i128 || (twice == d as i128 && q % 2 != 0);
    (if up { q + n.signum() } else { q }) as i64
}

/// A note on the trade an account's fill of a listing opened on a day.
struct Note {
    account: &'static str,
    listing: String,
    day: Date,
    entry: JournalEntry,
}

#[derive(Default)]
struct Demo {
    records: Vec<(Date, Record)>,
    /// Each option contract traded: its name and its underlying's symbol.
    options: Vec<String>,
    /// The last price each listing traded at, in cents per unit (per whole coin).
    last_price: BTreeMap<String, i64>,
    notes: Vec<Note>,
}

/// What a fill opened, for a note: the account, the listing and the day.
type Opened = (&'static str, String, Date);

impl Demo {
    fn trade(&mut self, buy: bool, account: &str, on: &str, key: &str, units: i64, price: i64, cash: i64) -> Opened {
        let (account, listing, on) = (account_id(account), sec_id(key), day(on));
        self.last_price.insert(listing.clone(), price);
        let r = if buy { Record::Buy { account: account.into(), listing: listing.clone(), units, cents: cash } } else { Record::Sell { account: account.into(), listing: listing.clone(), units, cents: cash } };
        self.records.push((on, r));
        (account, listing, on)
    }
    fn buy(&mut self, a: &str, on: &str, key: &str, qty: i64, px: f64) -> Opened {
        self.trade(true, a, on, key, qty, cents(px), qty * cents(px))
    }
    fn sell(&mut self, a: &str, on: &str, key: &str, qty: i64, px: f64) -> Opened {
        self.trade(false, a, on, key, qty, cents(px), qty * cents(px))
    }
    /// `sto`, `stc`, `bto`, `btc`: a contract's premium per share, a hundred shares a contract.
    fn option(&mut self, kind: &str, a: &str, on: &str, name: &str, qty: i64, px: f64) -> Opened {
        if !self.options.iter().any(|o| o == name) {
            self.options.push(name.to_string());
        }
        let buy = kind == "bto" || kind == "btc";
        self.trade(buy, a, on, name, qty, cents(px), qty * cents(px) * 100)
    }
    fn crypto(&mut self, buy: bool, on: &str, sym: &str, qty: f64, px: f64) -> Opened {
        let units = millionths(qty);
        self.trade(buy, "Crypto", on, sym, units, cents(px), mul_div(units, cents(px), 1_000_000))
    }
    fn reward(&mut self, on: &str, sym: &str, qty: f64) {
        self.records.push((day(on), Record::Staking { account: account_id("Crypto").into(), listing: sec_id(sym), units: millionths(qty) }));
    }
    /// A distribution of `per` a unit on `qty` units.
    fn dividend(&mut self, a: &str, on: Date, sym: &str, qty: i64, per: f64) {
        let per = (per * 10_000.0).round() as i64;
        self.records.push((on, Record::Dividend { account: account_id(a).into(), listing: sec_id(sym), units: qty, cents: mul_div(qty, per, 100) }));
    }
    fn deposit(&mut self, a: &str, on: &str, amount: i64) {
        self.records.push((day(on), Record::Deposit { account: account_id(a).into(), cents: amount * 100 }));
    }
    fn charge(&mut self, a: &str, on: Date, amount: f64) {
        self.records.push((on, Record::InterestCharge { account: account_id(a).into(), currency: "USD".into(), cents: cents(amount) }));
    }
    fn note(&mut self, on: &Opened, grade: &str, tags: &[&str], thesis: &str) {
        let grade = (!grade.is_empty()).then(|| Grade::parse(grade).expect("a grade the journal keeps"));
        let entry = JournalEntry { thesis: thesis.to_string(), grade, tags: tags.iter().map(|t| t.to_string()).collect() };
        if !entry.is_empty() {
            self.notes.push(Note { account: on.0, listing: on.1.clone(), day: on.2, entry });
        }
    }
}

fn build() -> Demo {
    let mut b = Demo::default();
    let today = day(TODAY);
    b.deposit("TFSA", "2024-01-03", 30000);
    b.deposit("RRSP", "2024-01-03", 30000);
    b.deposit("Trading", "2024-02-01", 20000);
    b.deposit("Crypto", "2024-02-20", 15000);
    b.deposit("TFSA", "2024-07-02", 20000);
    b.deposit("RRSP", "2025-01-06", 20000);
    b.deposit("TFSA", "2025-09-02", 15000);

    // TFSA: Canadian core and a few swings
    b.buy("TFSA", "2024-01-08", "XEQT", 300, 28.10);
    b.buy("TFSA", "2024-07-08", "XEQT", 200, 31.40);
    b.buy("TFSA", "2025-09-03", "XEQT", 250, 36.20);
    let enb = b.buy("TFSA", "2024-02-12", "ENB", 400, 46.30);
    b.buy("TFSA", "2024-03-11", "TD", 150, 80.20);
    let shop = b.buy("TFSA", "2024-04-15", "SHOP", 60, 95.40);
    b.sell("TFSA", "2025-02-20", "SHOP", 60, 165.30);
    let bce = b.buy("TFSA", "2024-05-06", "BCE", 200, 45.10);
    b.sell("TFSA", "2025-03-12", "BCE", 200, 32.40);
    let cnq = b.buy("TFSA", "2025-04-08", "CNQ", 150, 40.20);
    b.sell("TFSA", "2025-08-19", "CNQ", 150, 45.10);
    b.note(&shop, "A", &["momentum", "earnings"], "Merchant growth re-accelerating after the logistics sale; held through two earnings beats and sold into the run.");
    // ungraded: the journal grades A, B, C and F, and the earlier demo's D was never kept
    b.note(&bce, "", &["yield-trap"], "Bought for the dividend; the payout was cut and the thesis was gone. Should have sold on the cut, not four months later.");
    b.note(&cnq, "B", &["energy", "swing"], "Oil oversold into tariff noise; took the bounce and left.");
    b.note(&enb, "", &["income"], "Core income holding. Add on weakness below $45.");

    // RRSP: index core plus US names
    b.buy("RRSP", "2024-01-10", "VFV", 250, 118.50);
    b.buy("RRSP", "2025-01-08", "VFV", 120, 145.20);
    let aapl = b.buy("RRSP", "2024-02-05", "AAPL", 40, 186.50);
    b.sell("RRSP", "2024-12-10", "AAPL", 40, 246.80);
    b.buy("RRSP", "2024-06-03", "MSFT", 20, 410.20);
    // TD on the NYSE, beside the TFSA's TD on the TSX: one ticker, two listings
    b.buy("RRSP", "2025-03-12", "TD@NYSE", 40, 58.10);
    b.buy("RRSP", "2024-08-07", "NVDA", 100, 98.90);
    b.sell("RRSP", "2025-01-27", "NVDA", 60, 118.40);
    let intc = b.buy("RRSP", "2024-04-29", "INTC", 200, 31.20);
    b.sell("RRSP", "2024-11-05", "INTC", 200, 22.90);
    b.note(&aapl, "B", &["core", "trim"], "Services margin story intact; trimmed the whole lot at a stretched multiple to fund the index core.");
    b.note(&intc, "C", &["turnaround"], "Foundry turnaround was a story, not a number. Cut after the dividend suspension.");

    // Trading (USD): covered calls on AAPL, long options, US swings
    let t = "Trading";
    b.buy(t, "2024-03-04", "AAPL", 100, 172.30);
    let cc1 = b.option("sto", t, "2024-03-18", "AAPL 19APR24 190.00 CALL", 1, 2.10);
    b.option("btc", t, "2024-04-12", "AAPL 19APR24 190.00 CALL", 1, 0.45);
    let cc2 = b.option("sto", t, "2024-05-20", "AAPL 21JUN24 200.00 CALL", 1, 2.85);
    b.option("btc", t, "2024-06-14", "AAPL 21JUN24 200.00 CALL", 1, 9.20);
    let cc3 = b.option("sto", t, "2024-10-21", "AAPL 15NOV24 240.00 CALL", 1, 3.40);
    b.option("btc", t, "2024-11-13", "AAPL 15NOV24 240.00 CALL", 1, 0.30);
    let cc4 = b.option("sto", t, "2025-01-27", "AAPL 21FEB25 260.00 CALL", 1, 2.60);
    b.option("btc", t, "2025-02-19", "AAPL 21FEB25 260.00 CALL", 1, 0.20);
    b.option("sto", t, "2026-08-24", "AAPL 16OCT26 260.00 CALL", 1, 4.10);
    let tsla = b.option("bto", t, "2024-07-15", "TSLA 20SEP24 200.00 PUT", 2, 6.30);
    b.option("stc", t, "2024-08-21", "TSLA 20SEP24 200.00 PUT", 2, 2.10);
    let nvc = b.option("bto", t, "2024-09-16", "NVDA 17JAN25 120.00 CALL", 2, 4.80);
    b.option("stc", t, "2024-12-02", "NVDA 17JAN25 120.00 CALL", 2, 16.40);
    let pltr = b.option("bto", t, "2024-10-14", "PLTR 21MAR25 40.00 CALL", 3, 2.15);
    b.option("stc", t, "2025-02-10", "PLTR 21MAR25 40.00 CALL", 3, 34.50);
    let amd = b.option("bto", t, "2025-03-10", "AMD 20JUN25 130.00 CALL", 2, 5.40);
    b.option("stc", t, "2025-06-13", "AMD 20JUN25 130.00 CALL", 2, 0.55);
    let sofi = b.buy(t, "2024-06-24", "SOFI", 300, 7.15);
    b.sell(t, "2025-01-21", "SOFI", 300, 15.80);
    b.buy(t, "2024-09-09", "RIVN", 200, 14.20);
    b.sell(t, "2025-04-07", "RIVN", 200, 11.60);
    b.buy(t, "2026-01-12", "HOOD", 100, 42.10);
    b.sell(t, "2026-03-25", "HOOD", 100, 55.30);
    b.buy(t, "2026-02-09", "COIN", 30, 250.40);
    b.sell(t, "2026-05-14", "COIN", 30, 198.20);
    b.buy(t, "2026-04-20", "UBER", 80, 78.30);
    b.sell(t, "2026-07-28", "UBER", 80, 88.10);
    b.buy(t, "2026-06-15", "NVDA", 25, 165.40);
    b.note(&cc1, "A", &["covered-call"], "Monthly call against the 100 shares; closed at 80% of max profit as planned.");
    b.note(&cc2, "C", &["covered-call", "capped"], "Sold the 200 strike two weeks before WWDC. Bought back for a loss rather than lose the shares.");
    b.note(&cc3, "A", &["covered-call"], "Post-earnings IV crush; closed early.");
    b.note(&cc4, "A", &["covered-call"], "Same setup as November.");
    b.note(&tsla, "", &["hedge", "theta"], "Bought puts after the run-up expecting a fade. Fade came late; theta ate it.");
    b.note(&nvc, "B", &["earnings", "long-call"], "Blackwell ramp priced too low into Q3; sold into the December high.");
    b.note(&pltr, "A", &["long-call", "momentum"], "Commercial revenue inflection; sized small, let it run through two earnings.");
    b.note(&amd, "C", &["long-call"], "Bet on an MI350 re-rate. Export controls in April killed the timing.");
    b.note(&sofi, "B", &["fintech", "swing"], "Bank charter economics finally showing in NIM; rode it to the January high.");

    // Crypto
    b.crypto(true, "2024-02-26", "BTC", 0.25, 58200.0);
    b.crypto(true, "2024-08-05", "BTC", 0.15, 82500.0);
    b.crypto(false, "2024-12-16", "BTC", 0.20, 132400.0);
    let eth = b.crypto(true, "2024-03-11", "ETH", 3.0, 4150.0);
    b.crypto(false, "2025-04-02", "ETH", 3.0, 3480.0);
    b.crypto(true, "2024-11-11", "SOL", 40.0, 195.0);
    // sold down to dust: 0.04 left, worth a cent, under the smallest order the broker takes
    b.crypto(true, "2025-01-06", "DOGE", 1000.0, 0.25);
    b.crypto(false, "2025-03-10", "DOGE", 999.96, 0.25);
    let (mut y, mut m) = (2024i16, 12i8);
    while ymd(y, m, 5) <= today {
        b.reward(&ymd(y, m, 5).to_string(), "SOL", 0.22);
        if m == 12 {
            y += 1;
            m = 1
        } else {
            m += 1
        }
    }
    b.note(&eth, "C", &["crypto"], "Bought the ETF-approval news; sold a year later below cost.");

    // Dividends and distributions
    for (y, per) in [(2024, 0.915), (2025, 0.9425), (2026, 0.975)] {
        for m in [3, 6, 9, 12] {
            let d = ymd(y, m, 1);
            if day("2024-02-12") < d && d <= today {
                b.dividend("TFSA", d, "ENB", 400, per);
            }
        }
    }
    for (y, per) in [(2024, 1.02), (2025, 1.05), (2026, 1.05)] {
        for m in [1, 4, 7, 10] {
            let d = if m != 1 { ymd(y, m, 30) } else { ymd(y, 1, 31) };
            if day("2024-03-11") < d && d <= today {
                b.dividend("TFSA", d, "TD", 150, per);
            }
        }
    }
    for d in ["2024-07-15", "2024-10-15", "2025-01-15"] {
        b.dividend("TFSA", day(d), "BCE", 200, 0.9975);
    }
    for y in [2024, 2025, 2026] {
        for (m, per) in [(3, 0.17), (6, 0.19), (9, 0.18), (12, 0.20)] {
            let d = ymd(y, m, 28);
            if day("2024-01-08") < d && d <= today {
                let qty = 300 + if d >= day("2024-07-08") { 200 } else { 0 } + if d >= day("2025-09-03") { 250 } else { 0 };
                b.dividend("TFSA", d, "XEQT", qty, per);
            }
            if day("2024-01-10") < d && d <= today {
                b.dividend("RRSP", d, "VFV", 250 + if d >= day("2025-01-08") { 120 } else { 0 }, per * 2.0);
            }
        }
    }
    // Margin interest on the Trading account, billed on the first of the month in USD
    for (k, amount) in [64.10, 71.85, 88.20, 93.40, 97.15, 102.60, 109.35, 118.90].into_iter().enumerate() {
        let d = ymd(2026, 2 + k as i8, 1);
        if d <= today {
            b.charge("Trading", d, amount);
        }
    }
    // in the order they happened; a day's rows in the order they were made
    b.records.sort_by_key(|(d, _)| *d);
    b
}

/// Every listing traded: the shares and coins, then each option contract.
fn listings(b: &Demo) -> Vec<ListingRecord> {
    let price = |id: &str| b.last_price.get(id).copied().unwrap_or(0);
    let mut out: Vec<ListingRecord> = LISTINGS
        .iter()
        .map(|(sym, name, ex, mic, cur)| (*sym, *sym, *name, *ex, *mic, *cur))
        .chain(MORE_LISTINGS.iter().copied())
        .map(|(key, sym, name, ex, mic, cur)| {
            let id = sec_id(key);
            let sort = if COINS.contains(&key) { ListingSort::Coin } else { ListingSort::Share { exchange: ex.into(), mic: mic.into() } };
            ListingRecord { price: price(&id), id, symbol: sym.into(), name: name.into(), currency: cur.into(), sort }
        })
        .collect();
    for name in &b.options {
        // `UNDER DDMMMYY STRIKE CALL|PUT`
        let parts: Vec<&str> = name.split(' ').collect();
        let [under, expiry, strike, right] = parts[..] else { panic!("a made-up contract's name: {name}") };
        let expiry = Date::strptime("%d%b%y", expiry).expect("a made-up expiry");
        let strike = cents(strike.parse::<f64>().expect("a made-up strike"));
        let id = sec_id(name);
        // Wealthsimple names a contract by its underlying's symbol and no name of its own
        out.push(ListingRecord { price: price(&id), id, symbol: under.into(), name: String::new(), currency: "USD".into(), sort: ListingSort::Contract { underlying: sec_id(under), expiry, strike, put: right == "PUT" } });
    }
    out
}

/// Mersenne Twister MT19937 seeded from an integer key, with 53-bit floats and
/// Box-Muller normals that cache the second draw, so the equity path is the
/// same sequence on every run.
struct Mt {
    mt: [u32; 624],
    idx: usize,
    gauss_next: Option<f64>,
}

impl Mt {
    fn new(seed: u32) -> Mt {
        let mut m = Mt { mt: [0; 624], idx: 624, gauss_next: None };
        m.mt[0] = 19650218;
        for i in 1..624 {
            let prev = m.mt[i - 1];
            m.mt[i] = 1812433253u32.wrapping_mul(prev ^ (prev >> 30)).wrapping_add(i as u32);
        }
        let key = [seed];
        let (mut i, mut j) = (1usize, 0usize);
        for _ in 0..624usize.max(key.len()) {
            let prev = m.mt[i - 1];
            m.mt[i] = (m.mt[i] ^ (prev ^ (prev >> 30)).wrapping_mul(1664525)).wrapping_add(key[j]).wrapping_add(j as u32);
            i += 1;
            j += 1;
            if i >= 624 {
                m.mt[0] = m.mt[623];
                i = 1;
            }
            if j >= key.len() {
                j = 0;
            }
        }
        for _ in 0..623 {
            let prev = m.mt[i - 1];
            m.mt[i] = (m.mt[i] ^ (prev ^ (prev >> 30)).wrapping_mul(1566083941)).wrapping_sub(i as u32);
            i += 1;
            if i >= 624 {
                m.mt[0] = m.mt[623];
                i = 1;
            }
        }
        m.mt[0] = 0x8000_0000;
        m
    }
    fn next_u32(&mut self) -> u32 {
        if self.idx >= 624 {
            for k in 0..624 {
                let y = (self.mt[k] & 0x8000_0000) | (self.mt[(k + 1) % 624] & 0x7fff_ffff);
                let mut v = self.mt[(k + 397) % 624] ^ (y >> 1);
                if y & 1 != 0 {
                    v ^= 0x9908_b0df;
                }
                self.mt[k] = v;
            }
            self.idx = 0;
        }
        let mut y = self.mt[self.idx];
        self.idx += 1;
        y ^= y >> 11;
        y ^= (y << 7) & 0x9d2c_5680;
        y ^= (y << 15) & 0xefc6_0000;
        y ^ (y >> 18)
    }
    fn random(&mut self) -> f64 {
        let a = (self.next_u32() >> 5) as f64;
        let b = (self.next_u32() >> 6) as f64;
        (a * 67108864.0 + b) / 9007199254740992.0
    }
    fn gauss(&mut self, mu: f64, sigma: f64) -> f64 {
        let z = match self.gauss_next.take() {
            Some(z) => z,
            None => {
                let x2pi = self.random() * std::f64::consts::TAU;
                let g2rad = (-2.0 * (1.0 - self.random()).ln()).sqrt();
                self.gauss_next = Some(x2pi.sin() * g2rad);
                x2pi.cos() * g2rad
            }
        };
        mu + z * sigma
    }
}

/// The book's value and net deposits each business day from 2024-01-02: deposits
/// as a step, the value as deposits times a drifting, noisy path.
fn nav_series() -> Vec<(Date, f64, f64)> {
    let mut rng = Mt::new(9);
    let steps = [("2024-01-03", 60000i64), ("2024-02-01", 80000), ("2024-02-20", 95000), ("2024-07-02", 115000), ("2025-01-06", 135000), ("2025-09-02", 150000)];
    let mut out = Vec::new();
    let mut growth = 1.0f64;
    let mut d = ymd(2024, 1, 2);
    while d <= day(TODAY) {
        if !matches!(d.weekday(), Weekday::Saturday | Weekday::Sunday) {
            let iso = d.to_string();
            let deposits = steps.iter().filter(|s| s.0 <= iso.as_str()).map(|s| s.1).last().unwrap_or(0);
            growth *= 1.0 + rng.gauss(0.00055, 0.009);
            if ["2024-08-05", "2025-04-03", "2025-04-04"].contains(&iso.as_str()) {
                growth *= 0.955;
            }
            let equity = if deposits != 0 { format!("{:.2}", deposits as f64 * growth).parse().expect("a value to the cent") } else { 0.0 };
            out.push((d, equity, deposits as f64));
        }
        d = d.tomorrow().expect("a day after");
    }
    out
}

/// The made-up accounts, each stating its share of the book's daily value.
fn accounts() -> Vec<AccountRecord> {
    let nav = nav_series();
    ACCOUNTS
        .iter()
        .map(|(nick, id, cur, ty, share)| AccountRecord {
            id: id.to_string(),
            nickname: nick.to_string(),
            ty: ty.to_string(),
            currency: cur.to_string(),
            opened: nav[0].0,
            days: nav.iter().map(|(d, v, dep)| (*d, to_cents(v * share), to_cents(dep * share))).collect(),
            buying_power: (*id == BUYING_POWER.0).then(|| to_cents(BUYING_POWER.1.parse().expect("an amount"))),
        })
        .collect()
}

/// Write the made-up book into `home`, a folder that holds no book yet. What
/// it wrote, in a line.
pub fn write(home: &Path) -> Result<String, String> {
    if home.join(bagholder_book::BOOK_FILE).exists() {
        return Err(format!("{} already holds a book; the made-up one is written only into a folder that holds none", home.display()));
    }
    std::fs::create_dir_all(home).map_err(err)?;
    let at: Timestamp = SYNCED.parse().map_err(err)?;
    let today = day(TODAY);
    let demo = build();
    let (book, _) = Book::open_in(home, crate::app::APP_VERSION, at).map_err(err)?;
    // the zone the browser tests' pages run in, as a page states it: the figures are
    // built from the first start, before any page has opened
    book.state_zone("America/Toronto", at).map_err(err)?;
    let ws = Broker::named("wealthsimple");
    let connection = book.add_connection(&ws, "Wealthsimple", at).map_err(err)?;
    let mut adapter = Wealthsimple::new(Generated::from_records(today, &accounts(), &listings(&demo), &demo.records));
    let report = pull::pull(&book, &mut adapter, connection, today, at, &mut |_| {}).map_err(err)?;
    if !report.failures.is_empty() {
        return Err(format!("the pull of the made-up Wealthsimple failed: {:?}", report.failures));
    }
    drop(book);
    let facts = crate::demo_facts::write(home)?;

    // the journal, written as a page writes it: on the trade each note's fill opened
    let f = crate::figures::Figures::open(home, at)?;
    let book = f.book()?;
    let opened: BTreeMap<bagholder_core::TransactionId, (bagholder_core::AccountId, Option<bagholder_core::InstrumentId>, Date)> = book.transactions().map_err(err)?.into_iter().map(|t| (t.id, (t.account, t.instrument, t.trade_date))).collect();
    let trades: Vec<(bagholder_core::TransactionId, bagholder_core::TradeId)> = f
        .read(|e| e.figures().trades.iter().filter_map(|t| match (&t.key, t.trade) {
            (TradeKey::Trip(k), Some(id)) => Some((k.opening.clone(), id)),
            _ => None,
        }).collect())
        .ok_or("the figures of the made-up book were not built")?;
    for note in &demo.notes {
        let account = book.account_by_ref(&AccountRef::new(ws.clone(), note.account)).map_err(err)?.ok_or_else(|| format!("no account {}", note.account))?;
        let instrument = book.instrument_by_ref(&Reference::new(RefScheme::BrokerSecurity(ws.clone()), &note.listing)).map_err(err)?.ok_or_else(|| format!("no listing {}", note.listing))?;
        let trade = trades
            .iter()
            .find(|(opening, _)| opened.get(opening) == Some(&(account, Some(instrument), note.day)))
            .map(|(_, id)| *id)
            .ok_or_else(|| format!("no trade of {} in {} opened {}", note.listing, note.account, note.day))?;
        f.write_journal(&trade.to_string(), &note.entry, at).map_err(|e| format!("the note on {} could not be written: {e:?}", note.listing))?;
    }
    Ok(format!("demo: {} rows pulled into {}, {} notes; {facts}", report.rows_read, home.display(), demo.notes.len()))
}

pub fn cli(args: &[String]) -> i32 {
    let [home] = args else {
        eprintln!("usage: bagholder demo <data folder>  (a made-up book, written into a folder that holds none)");
        return 2;
    };
    match write(Path::new(home)) {
        Ok(report) => {
            print!("{report}");
            0
        }
        Err(e) => {
            eprintln!("the made-up book could not be written: {e}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_made_up_book_is_pulled_with_its_journal_and_facts() {
        let home = tempfile::tempdir().unwrap();
        write(home.path()).unwrap();
        let (book, _) = Book::open_in(home.path(), crate::app::APP_VERSION, SYNCED.parse().unwrap()).unwrap();
        assert_eq!(book.accounts().unwrap().len(), 4);
        assert_eq!(book.journal_entries().unwrap().len(), build().notes.len());
        assert!(book.orphaned_journal().unwrap().is_empty());
        // a second run never writes over a book
        assert!(write(home.path()).unwrap_err().contains("already holds a book"));
    }

    #[test]
    fn the_equity_path_is_the_same_on_every_run() {
        let nav = nav_series();
        // the path the earlier demo drew, seeded the same, to the cent: 701 business days
        assert_eq!(nav.len(), 701);
        assert_eq!(nav[0], (ymd(2024, 1, 2), 0.0, 0.0));
        assert_eq!(nav[1], (ymd(2024, 1, 3), 59677.03, 60000.0));
        assert_eq!(nav[700], (day(TODAY), 223248.64, 150000.0));
        assert_eq!(nav_series(), nav);
    }
}
