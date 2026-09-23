//! A made-up book for the README screenshots: four accounts, shares on both
//! sides of the border, covered calls, long options, dividends, crypto with
//! staking, and two and a half years of daily equity. Nothing in it is anyone's.
//!
//! ```text
//! cargo run -p bagholder-store --bin demo-book -- --home /tmp/bh-demo        # a desktop data directory
//! cargo run -p bagholder-store --bin demo-book -- --pull /tmp/bh-demo-phone  # last-pull.json + journal.json for the apps
//! ```
//!
//! It carries what the Portfolio tab needs too: each account's net liquidation
//! value, a margin balance and buying power. Market data (FX, indexes, quotes,
//! declared distributions) is fetched by the app itself. The phone files are
//! seeded as MOBILE.md describes. The screenshots in this folder were taken
//! that way.

use bagholder_model::cases::round_half_even;
use bagholder_model::dates::{fmt, from_days, parse_iso, to_days};
use rusqlite::Connection;
use serde::Serialize;
use serde_json::{json, Value};
use std::cell::Cell;
use std::collections::BTreeMap;
use std::path::Path;

const TODAY: &str = "2026-09-08";
const SYNCED: &str = "2026-09-08T20:05:00Z";

/// nickname, id, currency, type
const ACCOUNTS: [(&str, &str, &str, &str); 4] = [
    ("TFSA", "acct-tfsa", "CAD", "SELF_DIRECTED_TFSA"),
    ("RRSP", "acct-rrsp", "CAD", "SELF_DIRECTED_RRSP"),
    ("Trading", "acct-trading", "USD", "SELF_DIRECTED_NON_REGISTERED_MARGIN"),
    ("Crypto", "acct-crypto", "CAD", "SELF_DIRECTED_CRYPTO"),
];

/// symbol, name, exchange, mic, currency
const LISTINGS: [(&str, &str, &str, &str, &str); 22] = [
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
];

/// Net liquidation value per account at the snapshot: positions plus cash, less margin.
const NAV_BY_ACCOUNT: [(&str, f64); 4] = [("TFSA", 90714.35), ("RRSP", 97220.10), ("Trading", 18157.40), ("Crypto", 27787.60)];

/// A number that stays an integer until it meets a float.
#[derive(Clone, Copy)]
enum N { I(i64), F(f64) }

impl N {
    fn f(self) -> f64 { match self { N::I(i) => i as f64, N::F(f) => f } }
    fn mul(self, o: N) -> N { match (self, o) { (N::I(a), N::I(b)) => N::I(a * b), _ => N::F(self.f() * o.f()) } }
    fn neg(self) -> N { match self { N::I(i) => N::I(-i), N::F(f) => N::F(-f) } }
    fn round2(self) -> N { match self { N::I(i) => N::I(i), N::F(f) => N::F(round_half_even(f, 2)) } }
    fn json(self) -> Value { match self { N::I(i) => json!(i), N::F(f) => json!(f) } }
}
fn i(v: i64) -> N { N::I(v) }
fn f(v: f64) -> N { N::F(v) }

/// Mersenne Twister MT19937 seeded from an integer key, with 53-bit floats and
/// Box-Muller normals that cache the second draw, so the equity path is the
/// same sequence on every run.
struct Mt { mt: [u32; 624], idx: usize, gauss_next: Option<f64> }

impl Mt {
    fn new(seed: u32) -> Mt {
        let mut m = Mt { mt: [0; 624], idx: 624, gauss_next: None };
        m.init_genrand(19650218);
        let key = [seed];
        let (mut i, mut j) = (1usize, 0usize);
        for _ in 0..624usize.max(key.len()) {
            let prev = m.mt[i - 1];
            m.mt[i] = (m.mt[i] ^ (prev ^ (prev >> 30)).wrapping_mul(1664525)).wrapping_add(key[j]).wrapping_add(j as u32);
            i += 1; j += 1;
            if i >= 624 { m.mt[0] = m.mt[623]; i = 1; }
            if j >= key.len() { j = 0; }
        }
        for _ in 0..623 {
            let prev = m.mt[i - 1];
            m.mt[i] = (m.mt[i] ^ (prev ^ (prev >> 30)).wrapping_mul(1566083941)).wrapping_sub(i as u32);
            i += 1;
            if i >= 624 { m.mt[0] = m.mt[623]; i = 1; }
        }
        m.mt[0] = 0x8000_0000;
        m
    }
    fn init_genrand(&mut self, s: u32) {
        self.mt[0] = s;
        for i in 1..624 {
            let prev = self.mt[i - 1];
            self.mt[i] = 1812433253u32.wrapping_mul(prev ^ (prev >> 30)).wrapping_add(i as u32);
        }
        self.idx = 624;
    }
    fn next_u32(&mut self) -> u32 {
        if self.idx >= 624 {
            for k in 0..624 {
                let y = (self.mt[k] & 0x8000_0000) | (self.mt[(k + 1) % 624] & 0x7fff_ffff);
                let mut v = self.mt[(k + 397) % 624] ^ (y >> 1);
                if y & 1 != 0 { v ^= 0x9908_b0df; }
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

fn sec_id(symbol: &str) -> String {
    format!("sec-{}", symbol.to_lowercase().replace(' ', "-").replace('.', "_"))
}

fn account(nick: &str) -> (&'static str, &'static str, &'static str, &'static str) {
    *ACCOUNTS.iter().find(|a| a.0 == nick).unwrap()
}

#[derive(Default)]
struct Book {
    n: usize,
    acts: Vec<Value>,
    option_ids: Vec<(String, String)>,
    journal: Vec<(String, Value)>,
}

impl Book {
    fn row(&mut self, kind: &str, acct: &str, day: &str, symbol: &str, qty: N, px: N, cash: N, currency: &str) -> String {
        self.n += 1;
        let id = format!("demo-{:04}", self.n);
        let (_, aid, _, _) = account(acct);
        let mut base = json!({
            "id": id, "canonicalId": id,
            "occurredAt": format!("{}T15:30:00.000000-04:00", day),
            "transactionDate": day, "accountId": aid, "fifoId": aid, "accountType": acct,
            "description": "", "direction": "", "symbol": symbol,
            "name": symbol.split(' ').next().unwrap_or(""),
            "currency": "CAD", "aftType": "", "counterSymbol": "",
            "securityId": if symbol.is_empty() { String::new() } else { sec_id(symbol) },
            "quantity": qty.json(), "unitPrice": px.json(), "commission": 0.0,
            "netCashAmount": cash.round2().json(),
        });
        let (category, at, sub, raw, dir) = match kind {
            "buy" => ("trade", "Trade", "BUY", "DIY_BUY", "debit"),
            "sell" => ("trade", "Trade", "SELL", "DIY_SELL", "credit"),
            "sto" => ("trade", "OPTIONS_SELL", "SELLTOOPEN", "OPTIONS_SELL", "credit"),
            "stc" => ("trade", "OPTIONS_SELL", "SELLTOCLOSE", "OPTIONS_SELL", "credit"),
            "bto" => ("trade", "OPTIONS_BUY", "BUYTOOPEN", "OPTIONS_BUY", "debit"),
            "btc" => ("trade", "OPTIONS_BUY", "BUYTOCLOSE", "OPTIONS_BUY", "debit"),
            "div" => ("dividend", "Dividend", "dividend", "DIVIDEND", "credit"),
            "dep" => ("deposit", "Deposit", "deposit", "DEPOSIT", "credit"),
            "cbuy" => ("other", "CRYPTO_BUY", "MARKET_ORDER", "CRYPTO_BUY", "credit"),
            "csell" => ("other", "CRYPTO_SELL", "MARKET_ORDER", "CRYPTO_SELL", "credit"),
            "reward" => ("other", "CRYPTO_STAKING_REWARD", "other", "CRYPTO_STAKING_REWARD", "credit"),
            "charge" => ("other", "INTEREST_CHARGE", "MARGIN_INTEREST", "INTEREST_CHARGE", "debit"),
            _ => unreachable!(),
        };
        let o = base.as_object_mut().unwrap();
        o.insert("category".into(), json!(category));
        o.insert("activityType".into(), json!(at));
        o.insert("activitySubType".into(), json!(sub));
        o.insert("rawType".into(), json!(raw));
        o.insert("direction".into(), json!(dir));
        if kind == "dep" { o.insert("aftType".into(), json!("misc_payments")); }
        let description = match kind {
            "div" => format!("Dividend: {}", symbol),
            "dep" => "Deposit".to_string(),
            "cbuy" => format!("CRYPTO_BUY: {}", symbol),
            "csell" => format!("CRYPTO_SELL: {}", symbol),
            "reward" => format!("CRYPTO_STAKING_REWARD: {}", symbol),
            _ => format!("{}: {}", at, symbol),
        };
        o.insert("description".into(), json!(description));
        o.insert("currency".into(), json!(currency));
        self.acts.push(base);
        id
    }
    fn buy(&mut self, a: &str, day: &str, sym: &str, qty: i64, px: f64, cur: &str) -> String {
        self.row("buy", a, day, sym, i(qty), f(px), i(qty).neg().mul(f(px)), cur)
    }
    fn sell(&mut self, a: &str, day: &str, sym: &str, qty: i64, px: f64, cur: &str) -> String {
        self.row("sell", a, day, sym, i(-qty), f(px), i(qty).mul(f(px)), cur)
    }
    fn option(&mut self, kind: &str, a: &str, day: &str, sym: &str, qty: i64, px: f64) -> String {
        let under = sym.split(' ').next().unwrap().to_string();
        if !self.option_ids.iter().any(|(s, _)| s == sym) { self.option_ids.push((sym.to_string(), under)); }
        let sign = if kind == "sto" || kind == "stc" { 1 } else { -1 };
        let q = if kind == "bto" || kind == "btc" { qty } else { -qty };
        let cash = i(sign * qty).mul(f(px)).mul(i(100));
        self.row(kind, a, day, sym, i(q), f(px), cash, "USD")
    }
    fn dividend(&mut self, a: &str, day: &str, sym: &str, qty: i64, per: f64) {
        self.row("div", a, day, sym, i(qty), f(per), i(qty).mul(f(per)), "CAD");
    }
    fn deposit(&mut self, a: &str, day: &str, amount: i64) {
        self.row("dep", a, day, "", i(0), i(0), i(amount), "CAD");
    }
    fn crypto(&mut self, kind: &str, day: &str, sym: &str, qty: N, px: N) -> String {
        self.row(kind, "Crypto", day, sym, qty, px, qty.mul(px), "CAD")
    }
    fn note(&mut self, id: &str, grade: &str, tags: &[&str], thesis: &str) {
        self.journal.push((format!("rt:{}", id), json!({"grade": grade, "tags": tags, "thesis": thesis})));
    }
}

fn build() -> Book {
    let mut b = Book::default();
    let (c, u) = ("CAD", "USD");
    b.deposit("TFSA", "2024-01-03", 30000); b.deposit("RRSP", "2024-01-03", 30000); b.deposit("Trading", "2024-02-01", 20000);
    b.deposit("Crypto", "2024-02-20", 15000); b.deposit("TFSA", "2024-07-02", 20000); b.deposit("RRSP", "2025-01-06", 20000); b.deposit("TFSA", "2025-09-02", 15000);

    // TFSA: Canadian core and a few swings
    b.buy("TFSA", "2024-01-08", "XEQT", 300, 28.10, c); b.buy("TFSA", "2024-07-08", "XEQT", 200, 31.40, c); b.buy("TFSA", "2025-09-03", "XEQT", 250, 36.20, c);
    let enb = b.buy("TFSA", "2024-02-12", "ENB", 400, 46.30, c);
    b.buy("TFSA", "2024-03-11", "TD", 150, 80.20, c);
    let shop = b.buy("TFSA", "2024-04-15", "SHOP", 60, 95.40, c); b.sell("TFSA", "2025-02-20", "SHOP", 60, 165.30, c);
    let bce = b.buy("TFSA", "2024-05-06", "BCE", 200, 45.10, c); b.sell("TFSA", "2025-03-12", "BCE", 200, 32.40, c);
    let cnq = b.buy("TFSA", "2025-04-08", "CNQ", 150, 40.20, c); b.sell("TFSA", "2025-08-19", "CNQ", 150, 45.10, c);
    b.note(&shop, "A", &["momentum", "earnings"], "Merchant growth re-accelerating after the logistics sale; held through two earnings beats and sold into the run.");
    b.note(&bce, "D", &["yield-trap"], "Bought for the dividend; the payout was cut and the thesis was gone. Should have sold on the cut, not four months later.");
    b.note(&cnq, "B", &["energy", "swing"], "Oil oversold into tariff noise; took the bounce and left.");
    b.note(&enb, "", &["income"], "Core income holding. Add on weakness below $45.");

    // RRSP: index core plus US names
    b.buy("RRSP", "2024-01-10", "VFV", 250, 118.50, c); b.buy("RRSP", "2025-01-08", "VFV", 120, 145.20, c);
    let aapl = b.buy("RRSP", "2024-02-05", "AAPL", 40, 186.50, u); b.sell("RRSP", "2024-12-10", "AAPL", 40, 246.80, u);
    b.buy("RRSP", "2024-06-03", "MSFT", 20, 410.20, u);
    b.buy("RRSP", "2024-08-07", "NVDA", 100, 98.90, u); b.sell("RRSP", "2025-01-27", "NVDA", 60, 118.40, u);
    let intc = b.buy("RRSP", "2024-04-29", "INTC", 200, 31.20, u); b.sell("RRSP", "2024-11-05", "INTC", 200, 22.90, u);
    b.note(&aapl, "B", &["core", "trim"], "Services margin story intact; trimmed the whole lot at a stretched multiple to fund the index core.");
    b.note(&intc, "C", &["turnaround"], "Foundry turnaround was a story, not a number. Cut after the dividend suspension.");

    // Trading (USD): covered calls on AAPL, long options, US swings
    let t = "Trading";
    b.buy(t, "2024-03-04", "AAPL", 100, 172.30, u);
    let cc1 = b.option("sto", t, "2024-03-18", "AAPL 19APR24 190.00 CALL", 1, 2.10); b.option("btc", t, "2024-04-12", "AAPL 19APR24 190.00 CALL", 1, 0.45);
    let cc2 = b.option("sto", t, "2024-05-20", "AAPL 21JUN24 200.00 CALL", 1, 2.85); b.option("btc", t, "2024-06-14", "AAPL 21JUN24 200.00 CALL", 1, 9.20);
    let cc3 = b.option("sto", t, "2024-10-21", "AAPL 15NOV24 240.00 CALL", 1, 3.40); b.option("btc", t, "2024-11-13", "AAPL 15NOV24 240.00 CALL", 1, 0.30);
    let cc4 = b.option("sto", t, "2025-01-27", "AAPL 21FEB25 260.00 CALL", 1, 2.60); b.option("btc", t, "2025-02-19", "AAPL 21FEB25 260.00 CALL", 1, 0.20);
    b.option("sto", t, "2026-08-24", "AAPL 16OCT26 260.00 CALL", 1, 4.10);
    let tsla = b.option("bto", t, "2024-07-15", "TSLA 20SEP24 200.00 PUT", 2, 6.30); b.option("stc", t, "2024-08-21", "TSLA 20SEP24 200.00 PUT", 2, 2.10);
    let nvc = b.option("bto", t, "2024-09-16", "NVDA 17JAN25 120.00 CALL", 2, 4.80); b.option("stc", t, "2024-12-02", "NVDA 17JAN25 120.00 CALL", 2, 16.40);
    let pltr = b.option("bto", t, "2024-10-14", "PLTR 21MAR25 40.00 CALL", 3, 2.15); b.option("stc", t, "2025-02-10", "PLTR 21MAR25 40.00 CALL", 3, 34.50);
    let amd = b.option("bto", t, "2025-03-10", "AMD 20JUN25 130.00 CALL", 2, 5.40); b.option("stc", t, "2025-06-13", "AMD 20JUN25 130.00 CALL", 2, 0.55);
    let sofi = b.buy(t, "2024-06-24", "SOFI", 300, 7.15, u); b.sell(t, "2025-01-21", "SOFI", 300, 15.80, u);
    let rivn = b.buy(t, "2024-09-09", "RIVN", 200, 14.20, u); b.sell(t, "2025-04-07", "RIVN", 200, 11.60, u);
    let hood = b.buy(t, "2026-01-12", "HOOD", 100, 42.10, u); b.sell(t, "2026-03-25", "HOOD", 100, 55.30, u);
    let coin = b.buy(t, "2026-02-09", "COIN", 30, 250.40, u); b.sell(t, "2026-05-14", "COIN", 30, 198.20, u);
    let uber = b.buy(t, "2026-04-20", "UBER", 80, 78.30, u); b.sell(t, "2026-07-28", "UBER", 80, 88.10, u);
    b.buy(t, "2026-06-15", "NVDA", 25, 165.40, u);
    b.note(&cc1, "A", &["covered-call"], "Monthly call against the 100 shares; closed at 80% of max profit as planned.");
    b.note(&cc2, "C", &["covered-call", "capped"], "Sold the 200 strike two weeks before WWDC. Bought back for a loss rather than lose the shares.");
    b.note(&cc3, "A", &["covered-call"], "Post-earnings IV crush; closed early.");
    b.note(&cc4, "A", &["covered-call"], "Same setup as November.");
    b.note(&tsla, "D", &["hedge", "theta"], "Bought puts after the run-up expecting a fade. Fade came late; theta ate it.");
    b.note(&nvc, "B", &["earnings", "long-call"], "Blackwell ramp priced too low into Q3; sold into the December high.");
    b.note(&pltr, "A", &["long-call", "momentum"], "Commercial revenue inflection; sized small, let it run through two earnings.");
    b.note(&amd, "C", &["long-call"], "Bet on an MI350 re-rate. Export controls in April killed the timing.");
    b.note(&sofi, "B", &["fintech", "swing"], "Bank charter economics finally showing in NIM; rode it to the January high.");
    for id in [&rivn, &hood, &coin, &uber] { b.note(id, "", &[], ""); }

    // Crypto
    b.crypto("cbuy", "2024-02-26", "BTC", f(0.25), i(58200)); b.crypto("cbuy", "2024-08-05", "BTC", f(0.15), i(82500)); b.crypto("csell", "2024-12-16", "BTC", f(0.20), i(132400));
    let eth = b.crypto("cbuy", "2024-03-11", "ETH", i(3), i(4150)); b.crypto("csell", "2025-04-02", "ETH", i(3), i(3480));
    b.crypto("cbuy", "2024-11-11", "SOL", i(40), i(195));
    let (mut y, mut m) = (2024i64, 12u32);
    while fmt(y, m, 5).as_str() <= TODAY {
        b.crypto("reward", &fmt(y, m, 5), "SOL", f(0.22), i(200 + (m as i64 * 7) % 60));
        if m == 12 { y += 1; m = 1 } else { m += 1 }
    }
    b.note(&eth, "C", &["crypto"], "Bought the ETF-approval news; sold a year later below cost.");

    // Dividends and distributions
    for (y, per) in [(2024, 0.915), (2025, 0.9425), (2026, 0.975)] {
        for m in [3, 6, 9, 12] {
            let day = fmt(y, m, 1);
            if "2024-02-12" < day.as_str() && day.as_str() <= TODAY { b.dividend("TFSA", &day, "ENB", 400, per); }
        }
    }
    for (y, per) in [(2024, 1.02), (2025, 1.05), (2026, 1.05)] {
        for m in [1, 4, 7, 10] {
            let day = if m != 1 { fmt(y, m, 30) } else { fmt(y, 1, 31) };
            if "2024-03-11" < day.as_str() && day.as_str() <= TODAY { b.dividend("TFSA", &day, "TD", 150, per); }
        }
    }
    for day in ["2024-07-15", "2024-10-15", "2025-01-15"] { b.dividend("TFSA", day, "BCE", 200, 0.9975); }
    for y in [2024, 2025, 2026] {
        for (m, per) in [(3, 0.17), (6, 0.19), (9, 0.18), (12, 0.20)] {
            let day = fmt(y, m, 28);
            let d = day.as_str();
            if "2024-01-08" < d && d <= TODAY {
                let qty = 300 + if d >= "2024-07-08" { 200 } else { 0 } + if d >= "2025-09-03" { 250 } else { 0 };
                b.dividend("TFSA", d, "XEQT", qty, per);
            }
            if "2024-01-10" < d && d <= TODAY {
                b.dividend("RRSP", d, "VFV", 250 + if d >= "2025-01-08" { 120 } else { 0 }, per * 2.0);
            }
        }
    }
    // Margin interest on the Trading account, billed on the first of the month in USD
    for (k, amount) in [64.10, 71.85, 88.20, 93.40, 97.15, 102.60, 109.35, 118.90].into_iter().enumerate() {
        let day = fmt(2026, 2 + k as u32, 1);
        if day.as_str() <= TODAY { b.row("charge", "Trading", &day, "", i(0), i(0), f(amount).neg(), "USD"); }
    }
    b.acts.sort_by(|x, y| {
        (x["transactionDate"].as_str(), x["id"].as_str()).cmp(&(y["transactionDate"].as_str(), y["id"].as_str()))
    });
    b
}

fn cash_securities() -> Vec<Value> {
    vec![
        json!({"id": "sec-c-cad", "symbol": "CAD", "name": "Canadian dollar", "primaryExchange": "", "primaryMic": "", "currency": "CAD", "underlyingId": ""}),
        json!({"id": "sec-c-usd", "symbol": "USD", "name": "US dollar", "primaryExchange": "", "primaryMic": "", "currency": "USD", "underlyingId": ""}),
    ]
}

fn balances() -> Vec<Value> {
    vec![
        json!({"accountId": "acct-tfsa", "securityId": "sec-c-cad", "quantity": 4210.35}),
        json!({"accountId": "acct-rrsp", "securityId": "sec-c-cad", "quantity": 1875.00}),
        json!({"accountId": "acct-trading", "securityId": "sec-c-usd", "quantity": -18240.60}),
        json!({"accountId": "acct-crypto", "securityId": "sec-c-cad", "quantity": 312.40}),
    ]
}

fn margin() -> Vec<Value> {
    vec![json!({"accountId": "acct-trading", "buyingPower": 12680.45, "currency": "CAD", "unavailable": ""})]
}

fn accounts() -> Vec<Value> {
    ACCOUNTS.iter().map(|(nick, aid, cur, typ)| {
        let nav = NAV_BY_ACCOUNT.iter().find(|n| n.0 == *nick).unwrap().1;
        json!({"id": aid, "nickname": nick, "unifiedAccountType": typ, "currency": cur, "status": "open", "type": "self_directed", "netLiquidationValue": nav})
    }).collect()
}

fn listings(b: &Book) -> Vec<Value> {
    let mut out: Vec<Value> = LISTINGS.iter().map(|(sym, name, ex, mic, cur)| {
        json!({"id": sec_id(sym), "symbol": sym, "name": name, "primaryExchange": ex, "primaryMic": mic, "currency": cur, "underlyingId": ""})
    }).collect();
    for (osym, under) in &b.option_ids {
        out.push(json!({"id": sec_id(osym), "symbol": osym, "name": osym, "primaryExchange": "OPRA", "primaryMic": "OPRA", "currency": "USD", "underlyingId": sec_id(under)}));
    }
    out
}

/// Business days from 2024-01-02: deposits as a step, equity as deposits times
/// a drifting, noisy path.
fn nav_series() -> Vec<Value> {
    let mut rng = Mt::new(9);
    let steps = [("2024-01-03", 60000i64), ("2024-02-01", 80000), ("2024-02-20", 95000), ("2024-07-02", 115000), ("2025-01-06", 135000), ("2025-09-02", 150000)];
    let mut out = Vec::new();
    let (ty, tm, td) = parse_iso(TODAY).unwrap();
    let end = to_days(ty, tm, td);
    let mut growth = 1.0f64;
    for days in to_days(2024, 1, 2)..=end {
        if (days + 3).rem_euclid(7) >= 5 { continue; }
        let (y, m, d) = from_days(days);
        let iso = fmt(y, m, d);
        let deposits = steps.iter().filter(|s| s.0 <= iso.as_str()).map(|s| s.1).last().unwrap_or(0);
        growth *= 1.0 + rng.gauss(0.00055, 0.009);
        if ["2024-08-05", "2025-04-03", "2025-04-04"].contains(&iso.as_str()) { growth *= 0.955; }
        let equity = if deposits != 0 { round_half_even(deposits as f64 * growth, 2) } else { 0.0 };
        out.push(json!({"date": iso, "equity": equity, "netDeposits": deposits as f64, "currency": "CAD"}));
    }
    out
}

fn kept_journal(b: &Book) -> BTreeMap<String, Value> {
    b.journal.iter().filter(|(_, v)| {
        v["grade"] != "" || !v["tags"].as_array().unwrap().is_empty() || v["thesis"] != ""
    }).cloned().collect()
}

fn write_pull(dir: &Path, b: &Book, lst: &[Value], nav: &[Value]) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    // the iOS decoder wants every field of the pull, the derived ones included; the apps compute them from the rows
    let metrics = json!({"realizedPnlCad": 0.0, "tradeCount": 0, "winCount": 0, "lossCount": 0, "evenCount": 0, "grossProfit": 0.0, "grossLoss": 0.0, "winRate": 0.0,
        "profitFactor": 0.0, "avgWin": 0.0, "avgLoss": 0.0, "expectancy": 0.0, "maxWinPnl": 0.0, "maxWinSymbol": "", "maxLossPnl": 0.0, "maxLossSymbol": "", "avgHoldDays": 0.0});
    let mut all = lst.to_vec();
    all.extend(cash_securities());
    let pull = json!({"activities": b.acts, "listings": all, "nav": nav, "navByAccount": {}, "syncedAt": SYNCED,
        "accounts": accounts(), "balances": balances(), "margin": margin(),
        "closed": [], "metrics": metrics, "monthly": [], "years": [], "avgAnnualized": "", "avgAnnualizedSubtitle": ""});
    std::fs::write(dir.join("last-pull.json"), serde_json::to_string(&pull).unwrap())?;
    let mut text = Vec::new();
    let mut ser = serde_json::Serializer::with_formatter(&mut text, serde_json::ser::PrettyFormatter::with_indent(b" "));
    kept_journal(b).serialize(&mut ser).unwrap();
    std::fs::write(dir.join("journal.json"), text)?;
    println!("phone seed: {} activities, {} listings, {} nav days", b.acts.len(), lst.len(), nav.len());
    Ok(())
}

/// Every row here comes from a `json!` literal built above; reading it back
/// as the typed row the store now takes is exactly what a lenient reader is
/// for, and it never fails on a shape this file itself just wrote.
fn typed<T: serde::de::DeserializeOwned>(rows: &[Value]) -> Vec<T> {
    rows.iter().map(|v| serde_json::from_value(v.clone()).unwrap()).collect()
}

fn write_home(dir: &Path, b: &Book, lst: &[Value], nav: &[Value]) -> rusqlite::Result<()> {
    std::fs::create_dir_all(dir).expect("create the data directory");
    let path = dir.join("bagholder.db");
    let conn = Connection::open(&path)?;
    bagholder_store::relabel::ensure(&conn)?;
    let n = Cell::new(0u64);
    let new_id = || { n.set(n.get() + 1); format!("00000000-0000-4000-8000-{:012}", n.get()) };
    bagholder_store::merge::apply_wealthsimple_mapped(&conn, &b.acts, &new_id)?;
    bagholder_store::admin::upsert_securities(&conn, &typed(lst), SYNCED)?;
    bagholder_store::tables::replace_accounts(&conn, &typed(&accounts()))?;
    bagholder_store::admin::upsert_securities(&conn, &typed(&cash_securities()), SYNCED)?;
    bagholder_store::tables::replace_balances(&conn, &typed(&balances()))?;
    bagholder_store::tables::replace_margin(&conn, &typed(&margin()), SYNCED)?;
    bagholder_store::tables::replace_nav(&conn, &typed(nav))?;
    bagholder_store::tables::set_meta(&conn, "synced_at", SYNCED)?;
    for (k, v) in kept_journal(b) {
        bagholder_store::admin::save_journal_entry(&conn, &k, Some(&v))?;
    }
    let count = bagholder_store::activities::activity_count(&conn)?;
    println!("desktop: {} activities in {}", count, path.display());
    Ok(())
}

/// Daily bars for every listing the book traded, so a chart has something to draw with
/// no source to ask (the browser tests run offline): each listing's closes pass through
/// the prices it was traded at on the days it was traded, with a small fixed ripple
/// between them, and are marked as read from the first day they cover.
fn write_bars(dir: &Path, b: &Book) -> rusqlite::Result<()> {
    let conn = Connection::open(dir.join("bagholder.db"))?;
    let (ty, tm, td) = parse_iso(TODAY).unwrap();
    let end = to_days(ty, tm, td);
    let mut written = 0usize;
    for (sym, ..) in LISTINGS.iter() {
        let mut anchors: Vec<(i64, f64)> = b.acts.iter()
            .filter(|a| a["category"] == "trade" && a["symbol"] == *sym)
            .filter_map(|a| {
                let (y, m, d) = parse_iso(a["transactionDate"].as_str()?)?;
                Some((to_days(y, m, d), a["unitPrice"].as_f64()?))
            })
            .collect();
        anchors.sort_by(|x, y| x.0.cmp(&y.0));
        anchors.dedup_by_key(|x| x.0);
        let Some(&(first, _)) = anchors.first() else { continue };
        let start = first - 30;
        let at = |day: i64| -> f64 {
            let next = anchors.iter().position(|x| x.0 >= day);
            match next {
                Some(0) => anchors[0].1,
                None => anchors[anchors.len() - 1].1,
                Some(k) => {
                    let (d0, p0) = anchors[k - 1];
                    let (d1, p1) = anchors[k];
                    p0 + (p1 - p0) * (day - d0) as f64 / (d1 - d0) as f64
                }
            }
        };
        let traded = |day: i64| anchors.iter().any(|x| x.0 == day);
        let mut bars = Vec::new();
        let mut prev = at(start);
        for day in start..=end {
            if (day + 3).rem_euclid(7) >= 5 { continue; }
            let ripple = if traded(day) { 0.0 } else { 0.012 * ((day as f64) * 0.9).sin() + 0.007 * ((day as f64) * 0.23).cos() };
            let close = round_half_even(at(day) * (1.0 + ripple), 2);
            let (open, hi, lo) = (prev, prev.max(close) * 1.004, prev.min(close) * 0.996);
            let (y, m, d) = from_days(day);
            bars.push(bagholder_store::bars::DayBar {
                date: fmt(y, m, d),
                px: bagholder_store::bars::Ohlcv {
                    open: Some(open),
                    high: Some(round_half_even(hi, 2)),
                    low: Some(round_half_even(lo, 2)),
                    close,
                    volume: Some(100000.0 + ((day % 17) as f64) * 5000.0),
                },
            });
            prev = close;
        }
        let (y, m, d) = from_days(start);
        written += bagholder_store::market::upsert_price_history(&conn, sym, &bars, "demo")?;
        bagholder_store::market::mark_history_fetched(&conn, sym, &fmt(y, m, d), SYNCED)?;
    }
    println!("desktop: {} daily bars", written);
    Ok(())
}

fn main() {
    let mut home = None;
    let mut pull = None;
    let mut bars = false;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--home" => home = args.next(),
            "--pull" => pull = args.next(),
            "--bars" => bars = true,
            _ => {
                eprintln!("usage: demo-book [--home DIR [--bars]] [--pull DIR]\n  --home DIR  write a desktop data directory (bagholder.db) here\n  --bars      with --home: daily bars for the listings traded, for a chart with no source to ask\n  --pull DIR  write last-pull.json and journal.json for the phone apps here");
                std::process::exit(2);
            }
        }
    }
    let book = build();
    let lst = listings(&book);
    let nav = nav_series();
    if let Some(dir) = pull {
        write_pull(Path::new(&dir), &book, &lst, &nav).expect("write the phone seed");
    }
    if let Some(dir) = home {
        write_home(Path::new(&dir), &book, &lst, &nav).expect("write the desktop data directory");
        if bars {
            write_bars(Path::new(&dir), &book).expect("write the daily bars");
        }
    }
}
