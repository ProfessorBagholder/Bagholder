//! `bagholder capacity`: the server held to its cost budget at the owner's size
//! (`docs/architecture.md` §16; `docs/plans/stage-p-capacity-and-process.md`).
//!
//! - `capacity build <folder> [--times N] [--seed S]` makes a data folder the
//!   way the app makes one: a made-up Wealthsimple of the owner's size (times N)
//!   pulled through the real adapter and pull, a year of balances reads at the
//!   owner's rate, the market facts through the cache's own writers, and the
//!   figures settled as a start settles them.
//! - `capacity run <work folder>` builds (or reuses) the owner's size and four
//!   times it under the work folder, measures each operation in a process of
//!   its own, prints the table, and fails when the budget or the growth rule is
//!   broken (or when an operation listed over budget is now within it).
//! - `capacity probe <operation> <folder>` is one measured run, printed as one
//!   line; `run` starts it.
//!
//! A subcommand of the server, as `demo-facts` and `pull-broker` are: what it
//! measures is the server's own code, which only this binary holds.

use std::alloc::{GlobalAlloc, Layout, System};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use bagholder_book::Book;
use bagholder_broker::pull;
use bagholder_core::jiff::civil::Date;
use bagholder_core::jiff::{SignedDuration, Timestamp};
use bagholder_core::{Broker, Currency, Dec, Money, SourceName};
use bagholder_sources::cache::{MarketCache, StoredQuote};
use bagholder_wealthsimple::adapter::Wealthsimple;
use bagholder_wealthsimple::generated::{Generated, Size};

// ---- the budget (docs/architecture.md §16; a test holds the two equal) ----

/// RAIL: load "in 5 seconds or less on mid-range mobile devices" (web.dev/articles/rail).
pub const STARTUP_MS: u64 = 5_000;
/// RAIL: "complete a transition initiated by user input within 100 ms".
pub const ANSWER_MS: u64 = 100;
/// RAIL: past a second the person loses the thread of the task.
pub const TO_FIGURES_MS: u64 = 1_000;
/// How much slower a Raspberry Pi 5 is than the runner, on one core: Raspberry
/// Pi's own Geekbench 6 single-core average for the Pi 5 (764) against published
/// runs of the runner's CPU, Azure Cobalt 100 (1,620 and 1,639). CPU only, not
/// measured on hardware.
pub const PI_SLOWDOWN: f64 = 2.1;
/// Work through an index grows by a B-tree's depth when the book is four times
/// the size; work proportional to the book grows about four times. Under twice
/// tells the two apart.
pub const GROWTH_LIMIT: f64 = 2.0;
/// The balances cadence (`broker_reads::BALANCES_EVERY`), in milliseconds.
pub const BALANCES_EVERY_MS: u64 = crate::broker_reads::BALANCES_EVERY.as_millis() as u64;
/// The quotes cadence (`due::QUOTES_EVERY`), in milliseconds.
pub const QUOTES_EVERY_MS: u64 = crate::due::QUOTES_EVERY.as_millis() as u64;
/// How many times the median is taken over.
pub const RUNS: usize = 7;

/// The owner's size, counted on a copy of the owner's book on 2026-10-08: 29
/// accounts, 210 instruments of which 53 option contracts (each option round
/// trip here makes its own), 646 trades, three years of activity.
pub const OWNER: Size = Size { accounts: 29, instruments: 157, trades: 646, months: 36, seed: 1 };
/// The rows the balances reads add to the book a day at the owner's size:
/// 9,761 over the 11 days counted on the same copy.
pub const STATEMENT_ROWS_A_DAY: u64 = 890;
/// A year of balances reads: the size the book reaches in a year of use.
pub const BALANCES_DAYS: u32 = 365;
/// The day the made-up activity ends on (fixed, so the same seed makes the same book).
pub const MADE_UP_TODAY: &str = "2026-10-08";

/// Each operation measured, whether a person waits on it, and whether the
/// growth rule holds it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Startup,
    AddTrade,
    Import,
    Brokers,
    PriceChanged,
    FeedStep,
}

impl Op {
    pub const ALL: [Op; 6] = [Op::Startup, Op::AddTrade, Op::Import, Op::Brokers, Op::PriceChanged, Op::FeedStep];
    pub fn name(self) -> &'static str {
        match self {
            Op::Startup => "startup",
            Op::AddTrade => "add-trade",
            Op::Import => "import",
            Op::Brokers => "brokers",
            Op::PriceChanged => "price-changed",
            Op::FeedStep => "feed-step",
        }
    }
    fn parse(s: &str) -> Option<Op> {
        Op::ALL.into_iter().find(|o| o.name() == s)
    }
    /// A person waits on it: its wall time fails the run.
    fn waited_on(self) -> bool {
        matches!(self, Op::Startup | Op::AddTrade | Op::Import)
    }
    /// It answers one change: the growth rule holds it.
    fn per_change(self) -> bool {
        self != Op::Startup
    }
}

/// What is over budget today, each with its issue: the run fails when one off
/// this list misses, and when one on it meets its budget and the growth rule,
/// so the list only shrinks (`docs/architecture.md` §16).
pub const OVER_BUDGET: &[(&str, &str)] = &[
    // the balances history read whole by every figures pass (`engine_inputs::brokers`)
    ("startup", "#417"),
    ("brokers", "#417"),
    ("price-changed", "#418"),
    ("import", "#419"),
    ("add-trade", "#420"),
    // the whole view built and serialised to learn whether it changed
    ("feed-step", "#426"),
];

// ---- work counted ----

static COUNT_ALLOCS: AtomicBool = AtomicBool::new(false);
static ALLOCS: AtomicU64 = AtomicU64::new(0);

/// The process's allocator, counting allocations only once a capacity probe
/// has turned counting on; otherwise the system's, with one flag read.
pub struct Counting;

// SAFETY: every call is passed straight to the system allocator with the same
// arguments; the count is a relaxed atomic add, which allocates nothing.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if COUNT_ALLOCS.load(Ordering::Relaxed) {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
        }
        // SAFETY: as the caller's contract for `alloc`
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if COUNT_ALLOCS.load(Ordering::Relaxed) {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
        }
        // SAFETY: as the caller's contract for `alloc_zeroed`
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: as the caller's contract for `dealloc`
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if COUNT_ALLOCS.load(Ordering::Relaxed) {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
        }
        // SAFETY: as the caller's contract for `realloc`
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

/// Bytes the process has asked the system to read (Linux's `rchar`): SQLite's
/// page reads that its own cache did not hold. None where the system does not say.
fn read_bytes() -> Option<u64> {
    let io = std::fs::read_to_string("/proc/self/io").ok()?;
    io.lines().find_map(|l| l.strip_prefix("rchar:")).and_then(|v| v.trim().parse().ok())
}

/// One measured run.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Measured {
    pub wall_ns: u64,
    pub steps: u64,
    pub allocs: u64,
    pub read: Option<u64>,
    /// What the budget divides by at this size: accounts for `brokers`, priced
    /// holdings for a price change and a stream step.
    pub divisor: u64,
}

struct Mark {
    at: std::time::Instant,
    steps: u64,
    allocs: u64,
    read: Option<u64>,
}

fn mark() -> Mark {
    Mark { at: std::time::Instant::now(), steps: bagholder_sqlite::work::steps(), allocs: ALLOCS.load(Ordering::SeqCst), read: read_bytes() }
}

fn since(m: &Mark, divisor: u64) -> Measured {
    Measured {
        wall_ns: m.at.elapsed().as_nanos() as u64,
        steps: bagholder_sqlite::work::steps() - m.steps,
        allocs: ALLOCS.load(Ordering::SeqCst) - m.allocs,
        read: match (m.read, read_bytes()) {
            (Some(a), Some(b)) => Some(b - a),
            _ => None,
        },
        divisor,
    }
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn made_up_today() -> Date {
    MADE_UP_TODAY.parse().expect("a day")
}

// ---- build ----

/// The owner's size times `times`.
pub fn sized(times: usize, seed: u64) -> Size {
    Size { accounts: OWNER.accounts * times, instruments: OWNER.instruments * times, trades: OWNER.trades * times, seed, ..OWNER }
}

/// Make the data folder `home` for `size`, with `balances_days` of balances reads.
pub fn build(home: &Path, size: Size, balances_days: u32) -> Result<String, String> {
    std::fs::create_dir_all(home).map_err(err)?;
    let today = made_up_today();
    let at: Timestamp = format!("{today}T20:00:00Z").parse().map_err(err)?;
    let (book, _) = Book::open_in(home, crate::app::APP_VERSION, at).map_err(err)?;
    book.state_zone("America/Toronto", at).map_err(err)?;
    let ws = Broker::named("wealthsimple");
    let connection = match book.connections().map_err(err)?.into_iter().find(|c| c.broker == ws) {
        Some(c) => c.id,
        None => book.add_connection(&ws, "Wealthsimple", at).map_err(err)?,
    };
    let mut adapter = Wealthsimple::new(Generated::new(size, today));
    let report = pull::pull(&book, &mut adapter, connection, today, at, &mut |_| {}).map_err(err)?;
    if !report.failures.is_empty() {
        return Err(format!("the pull failed: {:?}", report.failures));
    }
    // a year of balances reads at the owner's rate. One real pass, through the
    // pull, says what a pass writes; the rest write the same through the book's
    // own store calls, as the pull writes them (`pull::balances`), one
    // transaction a pass, without the pull's reads: what grows is what is measured,
    // and the build must not wait on it
    let rows = |b: &Book| -> Result<u64, String> { b.conn_for_tests().query_row("SELECT count(*) FROM statements", [], |r| r.get::<_, i64>(0)).map(|n| n as u64).map_err(err) };
    let start = at.checked_sub(SignedDuration::from_hours(24 * i64::from(balances_days))).map_err(err)?;
    let before = rows(&book)?;
    let read = pull::balances(&book, &mut adapter, connection, start).map_err(err)?;
    if !read.failures.is_empty() {
        return Err(format!("the balances read failed: {:?}", read.failures));
    }
    let per_pass = (rows(&book)? - before).max(1);
    let accounts_ratio = size.accounts as f64 / OWNER.accounts as f64;
    let passes_a_day = ((STATEMENT_ROWS_A_DAY as f64 * accounts_ratio / per_pass as f64).round() as u64).max(1);
    let every = SignedDuration::from_secs((86_400 / passes_a_day) as i64);
    // what each account states, read once: the made-up broker's balances do not move
    use bagholder_broker::BrokerAdapter as _;
    let stated = adapter.accounts().map_err(|f| f.to_string())?;
    let mut by_account = Vec::new();
    for a in &stated {
        let Some(id) = book.account_by_ref(&bagholder_core::account::AccountRef::new(ws.clone(), &a.key)).map_err(err)? else { continue };
        let margin = matches!(a.account_type, bagholder_core::account::AccountType::Known { kind: bagholder_core::account::AccountKind::Margin, .. });
        by_account.push((id, a.key.clone(), a.net_value, margin));
    }
    let keys: Vec<String> = by_account.iter().map(|(_, k, _, _)| k.clone()).collect();
    let cash = adapter.cash(&keys).map_err(|f| f.to_string())?;
    let margins: Vec<String> = by_account.iter().filter(|(_, _, _, m)| *m).map(|(_, k, _, _)| k.clone()).collect();
    let power = if margins.is_empty() { Default::default() } else { adapter.buying_power(&margins).map_err(|f| f.to_string())? };
    let mut when = start;
    for _ in 1..(u64::from(balances_days) * passes_a_day) {
        when = when.checked_add(every).map_err(err)?;
        book.atomically(|| {
            let read = book.broker_read(connection, "net-value", when)?;
            for (id, _, value, _) in &by_account {
                if let Some(v) = value {
                    book.store_net_value(*id, when, *v, &read)?;
                }
            }
            let read = book.broker_read(connection, "cash", when)?;
            for (id, key, _, _) in &by_account {
                if let Some(c) = cash.get(key) {
                    book.store_cash(*id, when, c, &read)?;
                }
            }
            let read = book.broker_read(connection, "buying-power", when)?;
            for (id, key, _, _) in by_account.iter().filter(|(_, _, _, m)| *m) {
                if let Some(Ok(p)) = power.get(key) {
                    book.store_buying_power(*id, when, &Ok(Money::new(*p, Currency::CAD)), &read)?;
                }
            }
            Ok(())
        })
        .map_err(err)?;
    }
    facts(home, &book, at)?;
    let first_account = by_account.first().map(|(id, _, _, _)| *id).ok_or("the made-up broker has no account")?;
    drop(book);
    // settled as a start settles it: the engine built, each round trip given its trade
    let f = crate::figures::Figures::open(home, at)?;
    // the older part of the activity imported again from Wealthsimple's own
    // activity export, through the import, as the owner's book holds it: what an
    // import is linked against
    let export = activity_export(adapter.source.rows())?;
    let open = || Ok(Box::new(std::io::Cursor::new(export.clone().into_bytes())) as Box<dyn std::io::BufRead>);
    crate::csv_import::import_from(&f, "activities.csv", &open, Some(first_account), at, &mut |_| crate::csv_import::Go::On).map_err(|e| format!("the activity export was not imported: {e:?}"))?;
    f.record_changed(at)?;
    Ok(format!("{} rows, {} balances passes ({} a day), {} statement rows\n", report.rows_read, u64::from(balances_days) * passes_a_day, passes_a_day, rows(&f.book()?)?))
}

/// The share of the feed's records the owner's book also holds from
/// Wealthsimple's activity export: 6,156 of 7,994, counted on the same copy.
pub const EXPORTED_SHARE: (usize, usize) = (6_156, 7_994);

/// Wealthsimple's activity export of the oldest `EXPORTED_SHARE` of the made-up
/// activity, written from the same replies the pull read, in the export's own
/// columns and words: its activity types and sub-types, a direction on every
/// row and a currency on every row (CAD on every coin's), as the owner's
/// imported export rows state them.
fn activity_export(rows: &[bagholder_core::json::Value]) -> Result<String, String> {
    use bagholder_core::json::Value;
    let field = |v: &Value, k: &str| match v {
        Value::Object(o) => match o.get(k) {
            Some(Value::String(t) | Value::Number(t)) => t.clone(),
            _ => String::new(),
        },
        _ => String::new(),
    };
    // the activity feed's type and sub-type, as the export writes them
    let words = |ty: &str, sub: &str| -> (String, String) {
        let (t, s) = match (ty, sub) {
            ("DIY_BUY", _) => ("Trade", "BUY"),
            ("DIY_SELL", _) => ("Trade", "SELL"),
            ("DIVIDEND", _) => ("Dividend", "dividend"),
            ("DEPOSIT", _) => ("Deposit", "deposit"),
            ("WITHDRAWAL", _) => ("Withdrawal", "withdrawal"),
            ("FUNDS_CONVERSION", _) => ("FxExchange", "fx"),
            ("INTEREST", _) => ("Interest", "interest"),
            ("INTERNAL_TRANSFER", _) => ("Transfer", "transfer"),
            ("OPTIONS_BUY", _) => ("OPTIONS_BUY", "BUYTOOPEN"),
            ("OPTIONS_SELL", _) => ("OPTIONS_SELL", "SELLTOCLOSE"),
            // the made-up options are bought, so one expiring leaves a long position
            ("OPTIONS_EXPIRY", _) => ("EXPIR", "SELL"),
            ("CRYPTO_STAKING_REWARD", _) | ("CREDIT_CARD_PAYMENT", _) => (ty, "other"),
            _ => (ty, sub),
        };
        (t.to_string(), s.to_string())
    };
    let toronto = bagholder_core::jiff::tz::TimeZone::get("America/Toronto").map_err(err)?;
    let mut rows: Vec<&Value> = rows.iter().collect();
    rows.sort_by_key(|r| field(r, "occurredAt"));
    let take = rows.len() * EXPORTED_SHARE.0 / EXPORTED_SHARE.1;
    let mut out = String::from("transaction_date,activity_type,activity_sub_type,direction,account_id,symbol,currency,quantity,net_cash_amount\n");
    for r in rows.into_iter().take(take) {
        let (ty, sub) = (field(r, "type"), field(r, "subType"));
        let amount = field(r, "amount");
        let out_of = field(r, "amountSign") == "negative" || matches!(ty.as_str(), "DIY_BUY" | "OPTIONS_BUY");
        let signed = if out_of && !amount.is_empty() { format!("-{amount}") } else { amount };
        let currency = match field(r, "currency") {
            c if c.is_empty() => "CAD".to_string(),
            c => c,
        };
        // the export states each day as Toronto's, where Wealthsimple states its days
        let day = field(r, "occurredAt").parse::<Timestamp>().map_err(|e| format!("a made-up row's time: {e}"))?.to_zoned(toronto.clone()).date().to_string();
        let (t, s) = words(&ty, &sub);
        // a contract named as the export names one: `UNDER 21NOV25 35.00 CALL`
        let symbol = match (field(r, "expiryDate").parse::<Date>(), field(r, "strikePrice")) {
            (Ok(expiry), strike) if !strike.is_empty() => format!("{} {} {strike} CALL", field(r, "assetSymbol"), expiry.strftime("%d%b%y").to_string().to_uppercase()),
            _ => field(r, "assetSymbol"),
        };
        let direction = if out_of { "DEBIT" } else { "CREDIT" };
        out.push_str(&[day, t, s, direction.to_string(), field(r, "accountId"), symbol, currency, field(r, "assetQuantity"), signed].join(","));
        out.push('\n');
    }
    Ok(out)
}

/// The market facts the figures read, through the stores' own writers: the
/// Bank's rate for every other currency, a quote for every listing, each
/// payer's record, and the benchmarks.
fn facts(home: &Path, book: &Book, at: Timestamp) -> Result<(), String> {
    let (cache, _) = MarketCache::open(&home.join(crate::figures::CACHE_FILE), crate::app::APP_VERSION, at).map_err(err)?;
    let source = SourceName::named("capacity");
    let bank = SourceName::named("bank-of-canada");
    let transactions = book.transactions().map_err(err)?;
    let first = transactions.iter().map(|t| t.trade_date).min().unwrap_or(made_up_today());
    let from = first.checked_sub(SignedDuration::from_hours(24 * 30)).map_err(err)?;
    // to the day the run is made on, so a value held today converts
    let to = Timestamp::now().to_zoned(bagholder_core::jiff::tz::TimeZone::get("America/Toronto").map_err(err)?).date().max(made_up_today());
    let days = |a: Date, b: Date| {
        let mut out = Vec::new();
        let mut d = a;
        while d <= b {
            if !matches!(d.weekday(), bagholder_core::jiff::civil::Weekday::Saturday | bagholder_core::jiff::civil::Weekday::Sunday) {
                out.push(d);
            }
            d = d.tomorrow().expect("a day after");
        }
        out
    };
    let instruments = book.instruments().map_err(err)?;
    for c in instruments.iter().map(|i| i.currency).filter(|c| *c != Currency::CAD).collect::<std::collections::BTreeSet<_>>() {
        let rates: Vec<(Date, Dec)> = days(from, to).into_iter().map(|d| (d, Dec::parse("1.35").expect("a rate"))).collect();
        book.store_rates(c, &rates, (from, to), &bank, at).map_err(err)?;
        book.store_rate_series(&[bagholder_book::facts::RateSeries { currency: c, source: bank.clone(), first_day: from, last_day: to, ended: false }], at).map_err(err)?;
    }
    // each listing quoted at the last price a fill of it stated, or its cost
    for i in &instruments {
        let price = transactions.iter().rev().find(|t| t.instrument == Some(i.id) && t.price.is_some()).and_then(|t| t.price).map(|p| p.amount).unwrap_or(Dec::ONE);
        cache.store_quote(&StoredQuote { instrument: i.id, source: source.clone(), price: Money::new(price, i.currency), change: None, change_pct: None, quoted_at: at, allowance: std::time::Duration::ZERO, received_at: at }).map_err(err)?;
    }
    // each payer quarterly, every payment it made declared a week before it was paid
    let mut paid: std::collections::BTreeMap<bagholder_core::InstrumentId, Vec<bagholder_book::facts::DeclaredRow>> = Default::default();
    for t in transactions.iter().filter(|t| t.kind == bagholder_core::transaction::Kind::Dividend) {
        let (Some(i), Some(cash)) = (t.instrument, t.cash) else { continue };
        let ex = t.trade_date.checked_sub(SignedDuration::from_hours(24 * 7)).map_err(err)?;
        paid.entry(i).or_default().push(bagholder_book::facts::DeclaredRow { form: bagholder_core::distribution::Form::Stated, ex_date: ex, record_date: Some(ex), pay_date: Some(t.trade_date), amount: Money::new(cash.amount.div_rounded(Dec::from_int(100), 6, bagholder_core::Rounding::HalfEven).map_err(err)?, cash.currency), reinvested: None });
    }
    for (i, rows) in &paid {
        book.store_declared(*i, rows, &source, at).map_err(err)?;
        book.store_frequency(*i, 4, &source, Some(made_up_today()), at).map_err(err)?;
    }
    for b in [bagholder_sources::contract::Benchmark::Sp500, bagholder_sources::contract::Benchmark::Tsx, bagholder_sources::contract::Benchmark::Tx60] {
        let closes: Vec<(Date, Dec)> = days(from, to).into_iter().enumerate().map(|(n, d)| (d, Dec::new(10_000 + n as i64, 2).expect("a close"))).collect();
        cache.store_benchmark_closes(b, &closes, &source, at).map_err(err)?;
    }
    Ok(())
}

// ---- probe: one measured run, in a process of its own ----

/// The app open on `home`, as a start opens it, with nothing running in the background.
fn opened(home: &Path) -> Result<Arc<crate::app::App>, String> {
    let app = crate::app::App::new(home.to_path_buf(), PathBuf::from("."), "127.0.0.1".into());
    let f = crate::figures::Figures::open(&app.home, Timestamp::now())?;
    app.set_figures(f);
    Ok(app)
}

fn figures(app: &crate::app::App) -> Result<&crate::figures::Figures, String> {
    app.figures.get().ok_or_else(|| "the figures are not open".to_string())
}

/// What a page showing every tab watches, as `web/src/lib/live.svelte.ts` says it.
const PAGE: [&str; 9] = ["status", "book", "notifications", "dashboard", "trades", "positions", "exposure", "markets", "cashflow"];

pub fn probe(op: Op, home: &Path) -> Result<Measured, String> {
    // every connection from here on is counted, and every allocation
    bagholder_sqlite::work::start_counting();
    COUNT_ALLOCS.store(true, Ordering::SeqCst);
    if op == Op::Startup {
        let m = mark();
        let app = opened(home)?;
        figures(&app)?.read(|_| ()).ok_or("the figures were not built")?;
        return Ok(since(&m, 1));
    }
    let app = opened(home)?;
    let f = figures(&app)?;
    let held: Vec<(bagholder_core::InstrumentId, String, Currency)> = f
        .read(|e| {
            // the shares held, each by the name it trades under now
            let ledger = &e.inputs().ledger;
            e.figures()
                .positions
                .iter()
                .filter_map(|p| {
                    let info = ledger.instruments.get(&p.instrument)?;
                    (info.instrument.kind == bagholder_core::instrument::InstrumentKind::Security).then_some(())?;
                    Some((p.instrument, info.names.last()?.symbol.clone(), info.instrument.currency))
                })
                .collect::<Vec<_>>()
        })
        .ok_or("the figures were not built")?;
    let priced = held.len().max(1) as u64;
    let accounts = f.book()?.accounts().map_err(err)?.len().max(1) as u64;
    match op {
        Op::Startup => unreachable!("measured above"),
        Op::Brokers => {
            let book = f.book()?;
            let m = mark();
            crate::engine_inputs::brokers(&book)?;
            Ok(since(&m, accounts))
        }
        Op::PriceChanged => {
            let (instrument, _, currency) = held.first().cloned().ok_or("nothing is held")?;
            quote_moved(f, instrument, currency)?;
            let m = mark();
            f.price_changed(instrument)?;
            Ok(since(&m, priced))
        }
        Op::FeedStep => {
            let mut feed = crate::events::Feed::open(app.clone());
            let want = |k: &str| (k.to_string(), crate::events::Want { params: serde_json::json!({}), have: None });
            app.events.watch(&app, feed.id(), PAGE.iter().map(|k| want(k)).collect());
            // the page's first state, sent whole, is not the step measured
            feed.step(&crate::status::status);
            let (instrument, _, currency) = held.first().cloned().ok_or("nothing is held")?;
            quote_moved(f, instrument, currency)?;
            f.price_changed(instrument)?;
            app.events.signal();
            let m = mark();
            feed.step(&crate::status::status);
            Ok(since(&m, priced))
        }
        Op::AddTrade => {
            let (_, symbol, currency) = held.first().cloned().ok_or("nothing is held")?;
            let req = crate::entries::EntryRequest::Trade { account: String::new(), instrument: None, symbol, currency: currency.to_string(), day: made_up_today().to_string(), side: "buy".into(), quantity: "1".into(), price: "10".into(), fee: "0".into() };
            let m = mark();
            crate::entries::enter(f, &req, Timestamp::now()).map_err(|e| format!("{e:?}"))?;
            Ok(since(&m, 1))
        }
        Op::Import => {
            let mut text = String::from("Date,Action,Symbol,Quantity,Price,Amount,Currency\n");
            for (n, (_, symbol, currency)) in held.iter().cycle().take(20).enumerate() {
                text.push_str(&format!("{},Buy,{symbol},1,{}.00,-{}.00,{currency}\n", made_up_today(), 10 + n, 10 + n));
            }
            let open = || Ok(Box::new(std::io::Cursor::new(text.clone().into_bytes())) as Box<dyn std::io::BufRead>);
            let m = mark();
            crate::csv_import::import_from(f, "capacity.csv", &open, None, Timestamp::now(), &mut |_| crate::csv_import::Go::On).map_err(|e| format!("{e:?}"))?;
            Ok(since(&m, 1))
        }
    }
}

/// A new quote for one listing, a cent above its last.
fn quote_moved(f: &crate::figures::Figures, instrument: bagholder_core::InstrumentId, currency: Currency) -> Result<(), String> {
    let cache = f.cache()?;
    let last = cache.quotes().map_err(err)?.into_iter().find(|q| q.instrument == instrument).map(|q| q.price.amount).unwrap_or(Dec::ONE);
    let now = Timestamp::now();
    let price = last.checked_add(Dec::new(1, 2).map_err(err)?).map_err(err)?;
    cache.store_quote(&StoredQuote { instrument, source: SourceName::named("capacity"), price: Money::new(price, currency), change: None, change_pct: None, quoted_at: now, allowance: std::time::Duration::ZERO, received_at: now }).map_err(err)
}

// ---- run: both sizes, every operation, the table and the verdict ----

/// The budget of an operation on a Pi, in milliseconds, at the size measured
/// (its divisor), with what it comes from.
pub fn budget_ms(op: Op, divisor: u64) -> (f64, &'static str) {
    match op {
        Op::Startup => (STARTUP_MS as f64, "RAIL load"),
        Op::AddTrade => (ANSWER_MS as f64, "RAIL response (answered with its figures)"),
        Op::Import => (TO_FIGURES_MS as f64, "RAIL flow"),
        Op::Brokers => (BALANCES_EVERY_MS as f64 / divisor as f64, "balances cadence / accounts"),
        Op::PriceChanged => (QUOTES_EVERY_MS as f64 / divisor as f64, "quotes cadence / priced holdings"),
        Op::FeedStep => (QUOTES_EVERY_MS as f64 / divisor as f64, "quotes cadence / signals a pass sends"),
    }
}

fn median(mut v: Vec<u64>) -> u64 {
    v.sort_unstable();
    v[v.len() / 2]
}

fn measure(exe: &Path, op: Op, home: &Path, times: usize) -> Result<Measured, String> {
    let mut runs = Vec::new();
    for _ in 0..times {
        // every probe writes (a quote, a trade, an import, what a start settles),
        // so each runs on a fresh copy and the built book stays as it was built
        let copy = home.with_extension("probe");
        copy_folder(home, &copy)?;
        let exec = std::time::Instant::now();
        let out = std::process::Command::new(exe).args(["capacity", "probe", op.name()]).arg(&copy).env("BAGHOLDER_OFFLINE", "1").env("BAGHOLDER_DRY_ORDERS", "1").output().map_err(err)?;
        std::fs::remove_dir_all(&copy).map_err(err)?;
        if !out.status.success() {
            return Err(format!("{} on {}: {}", op.name(), home.display(), String::from_utf8_lossy(&out.stderr)));
        }
        let line = String::from_utf8_lossy(&out.stdout);
        let v: serde_json::Value = serde_json::from_str(line.trim()).map_err(|e| format!("{}: {e}: {line}", op.name()))?;
        let n = |k: &str| v[k].as_u64();
        // startup is timed from the exec, as a person starting the app waits on it
        let wall_ns = if op == Op::Startup { exec.elapsed().as_nanos() as u64 } else { n("wall_ns").unwrap_or(0) };
        runs.push(Measured { wall_ns, steps: n("steps").unwrap_or(0), allocs: n("allocs").unwrap_or(0), read: n("read"), divisor: n("divisor").unwrap_or(1) });
    }
    Ok(Measured {
        wall_ns: median(runs.iter().map(|r| r.wall_ns).collect()),
        steps: median(runs.iter().map(|r| r.steps).collect()),
        allocs: median(runs.iter().map(|r| r.allocs).collect()),
        read: runs.iter().map(|r| r.read).collect::<Option<Vec<_>>>().map(median),
        divisor: runs[0].divisor,
    })
}

/// A data folder copied whole into `to`, replacing what was there.
fn copy_folder(from: &Path, to: &Path) -> Result<(), String> {
    if to.exists() {
        std::fs::remove_dir_all(to).map_err(err)?;
    }
    std::fs::create_dir_all(to).map_err(err)?;
    for entry in std::fs::read_dir(from).map_err(err)? {
        let entry = entry.map_err(err)?;
        let target = to.join(entry.file_name());
        if entry.file_type().map_err(err)?.is_dir() {
            copy_folder(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target).map_err(err)?;
        }
    }
    Ok(())
}

/// One operation's verdict.
#[derive(Debug, PartialEq)]
pub struct Verdict {
    pub op: Op,
    pub over: Vec<String>,
}

/// What breaks the budget or the growth rule, from the two sizes' measurements.
pub fn judge(op: Op, one: &Measured, four: &Measured) -> Verdict {
    let mut over = Vec::new();
    let pi_ms = four.wall_ns as f64 / 1e6 * PI_SLOWDOWN;
    let (budget, _) = budget_ms(op, four.divisor);
    if op.waited_on() && pi_ms > budget {
        over.push(format!("{:.0} ms on a Pi at four times the owner's size, over {budget:.0} ms", pi_ms));
    }
    if op.per_change() {
        let ratio = |a: u64, b: u64| if a == 0 { if b == 0 { 1.0 } else { f64::INFINITY } } else { b as f64 / a as f64 };
        for (what, a, b) in [("steps", Some(one.steps), Some(four.steps)), ("allocations", Some(one.allocs), Some(four.allocs)), ("bytes read", one.read, four.read)] {
            if let (Some(a), Some(b)) = (a, b) {
                let r = ratio(a, b);
                if r >= GROWTH_LIMIT {
                    over.push(format!("{what} grew {r:.2} times with the book"));
                }
            }
        }
    }
    Verdict { op, over }
}

/// The run fails on an operation off the list that is over, and on one on the
/// list that is not.
pub fn listed_disagree(verdicts: &[Verdict]) -> Vec<String> {
    let mut out = Vec::new();
    for v in verdicts {
        let listed = OVER_BUDGET.iter().any(|(name, _)| *name == v.op.name());
        match (listed, v.over.is_empty()) {
            (false, false) => out.push(format!("{} is over budget: {}", v.op.name(), v.over.join("; "))),
            (true, true) => out.push(format!("{} is within its budget now: take it off OVER_BUDGET and close its issue", v.op.name())),
            _ => {}
        }
    }
    out
}

pub fn run(work: &Path) -> Result<String, String> {
    let exe = std::env::current_exe().map_err(err)?;
    let (one, four) = (work.join("owner"), work.join("owner-x4"));
    for (dir, times) in [(&one, 1), (&four, 4)] {
        if !dir.join(bagholder_book::BOOK_FILE).exists() {
            build(dir, sized(times, OWNER.seed), BALANCES_DAYS)?;
        }
    }
    // each row printed as it is measured, so a long run shows where it is
    let header = "| Operation | Runner (ms) | Pi (ms) | Budget (ms) | From | VM steps ×1 → ×4 | Allocations ×1 → ×4 | Bytes read ×1 → ×4 |\n| --- | --- | --- | --- | --- | --- | --- | --- |\n";
    write_out(header)?;
    let mut verdicts = Vec::new();
    for op in Op::ALL {
        // an operation listed over budget is measured once: its counts are the
        // same every run, and one run over its budget keeps it over; only a run
        // within it is taken to the median of seven, to say it is off the list
        let listed = OVER_BUDGET.iter().any(|(name, _)| *name == op.name());
        let mut a = measure(&exe, op, &one, if listed { 1 } else { RUNS })?;
        let mut b = measure(&exe, op, &four, if listed { 1 } else { RUNS })?;
        if listed && judge(op, &a, &b).over.is_empty() {
            a = measure(&exe, op, &one, RUNS)?;
            b = measure(&exe, op, &four, RUNS)?;
        }
        let (budget, from) = budget_ms(op, b.divisor);
        let read = match (a.read, b.read) {
            (Some(x), Some(y)) => format!("{x} → {y}"),
            _ => "not counted here".into(),
        };
        write_out(&format!("| {} | {:.1} | {:.1} | {budget:.1} | {from} | {} → {} | {} → {} | {read} |\n", op.name(), b.wall_ns as f64 / 1e6, b.wall_ns as f64 / 1e6 * PI_SLOWDOWN, a.steps, b.steps, a.allocs, b.allocs))?;
        verdicts.push(judge(op, &a, &b));
    }
    let wrong = listed_disagree(&verdicts);
    if wrong.is_empty() {
        Ok(String::from("\nEvery operation is within its budget and the growth rule, or listed over it.\n"))
    } else {
        Err(format!("\n{}", wrong.join("\n")))
    }
}

fn write_out(text: &str) -> Result<(), String> {
    use std::io::Write as _;
    print!("{text}");
    std::io::stdout().flush().map_err(err)
}

pub fn cli(args: &[String]) -> i32 {
    let usage = "usage: bagholder capacity build <folder> [--times N] [--seed S] | run <work folder> | probe <operation> <folder>";
    let flag = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).and_then(|v| v.parse::<u64>().ok());
    let result = match args.first().map(String::as_str) {
        Some("build") => match args.get(1) {
            Some(dir) => build(Path::new(dir), sized(flag("--times").unwrap_or(1) as usize, flag("--seed").unwrap_or(OWNER.seed)), BALANCES_DAYS),
            None => Err(usage.into()),
        },
        Some("run") => match args.get(1) {
            Some(dir) => run(Path::new(dir)),
            None => Err(usage.into()),
        },
        Some("probe") => match (args.get(1).and_then(|o| Op::parse(o)), args.get(2)) {
            (Some(op), Some(dir)) => probe(op, Path::new(dir)).map(|m| {
                format!("{}\n", serde_json::json!({"wall_ns": m.wall_ns, "steps": m.steps, "allocs": m.allocs, "read": m.read, "divisor": m.divisor}))
            }),
            _ => Err(usage.into()),
        },
        _ => Err(usage.into()),
    };
    match result {
        Ok(text) => {
            print!("{text}");
            0
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SMALL: Size = Size { accounts: 12, instruments: 16, trades: 40, months: 4, seed: 3 };

    fn counts(home: &Path) -> Vec<(String, i64)> {
        let c = rusqlite::Connection::open(home.join(bagholder_book::BOOK_FILE)).unwrap();
        let tables: Vec<String> = c.prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name").unwrap().query_map([], |r| r.get(0)).unwrap().map(Result::unwrap).collect();
        tables.into_iter().map(|t| { let n = c.query_row(&format!("SELECT count(*) FROM \"{t}\""), [], |r| r.get(0)).unwrap(); (t, n) }).collect()
    }

    /// Every trade's and holding's figures, without the ids a build gives anew
    /// (each id is made from the moment it is made), in a fixed order.
    fn figures_digest(home: &Path) -> Vec<String> {
        let f = crate::figures::Figures::open(home, Timestamp::now()).unwrap();
        let mut out = f
            .read(|e| {
                let fig = e.figures();
                let trades = fig.trades.iter().map(|t| format!("trade {:?}", (t.opened_on, t.closed_on, t.currency, t.direction, &t.status, &t.qty, &t.entry, &t.exit, &t.basis, &t.pnl, &t.pnl_cad, &t.fees)));
                let held = fig.positions.iter().map(|p| format!("held {:?}", (p.currency, p.direction, &p.qty, &p.book, &p.avg, &p.market, &p.unrealized, &p.book_cad)));
                trades.chain(held).collect::<Vec<_>>()
            })
            .unwrap();
        out.sort();
        out
    }

    /// The same seed and size make the same book: what the CI's cached books rest on.
    #[test]
    fn test_the_same_seed_and_size_build_the_same_book() {
        let _g = crate::tests_common::guard();
        let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        build(a.path(), SMALL, 3).unwrap();
        build(b.path(), SMALL, 3).unwrap();
        let (ca, cb) = (counts(a.path()), counts(b.path()));
        assert!(ca.iter().any(|(t, n)| t == "transactions" && *n > 0), "{ca:?}");
        assert_eq!(ca, cb);
        // every row, the feed's and the export's, is read with no problem
        assert!(ca.iter().any(|(t, n)| t == "record_problems" && *n == 0), "{ca:?}");
        let (fa, fb) = (figures_digest(a.path()), figures_digest(b.path()));
        assert!(fa.iter().any(|l| l.starts_with("trade")) && fa.iter().any(|l| l.starts_with("held")), "{fa:?}");
        assert_eq!(fa, fb);
    }

    /// The activity export imported over the feed it repeats links row for row:
    /// a share's rows to the share although the broker names each contract on it
    /// by the share's symbol, and a contract's rows, written by its terms, to the
    /// contract the feed made. What stays unlinked is only the asset movements,
    /// which the import reads as cash and the feed as holdings (#405).
    #[test]
    fn test_the_activity_export_links_to_the_feed_it_repeats() {
        let _g = crate::tests_common::guard();
        let home = tempfile::tempdir().unwrap();
        build(home.path(), SMALL, 1).unwrap();
        let c = rusqlite::Connection::open(home.path().join(bagholder_book::BOOK_FILE)).unwrap();
        let n = |sql: &str| -> i64 { c.query_row(sql, [], |r| r.get(0)).unwrap() };
        // the case is there: a contract on a share the book also holds, both seen under the share's symbol
        assert!(n("SELECT count(*) FROM option_terms o JOIN instruments u ON u.id = o.underlying_id AND u.kind = 'security'") > 0);
        assert!(n("SELECT count(*) FROM source_records WHERE source = 'csv'") > 0);
        let unlinked: Vec<String> = c
            .prepare("SELECT json_extract(v.payload, '$.cells.activity_type') FROM source_records r JOIN record_revisions v ON v.record_id = r.id WHERE r.source = 'csv' AND r.state = 'live'")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert!(unlinked.iter().all(|t| t == "ASSET_MOVEMENT"), "{unlinked:?}");
        assert_eq!(n("SELECT count(*) FROM instruments i WHERE NOT EXISTS (SELECT 1 FROM instrument_sightings s JOIN source_records r ON r.id = s.record_id WHERE s.instrument_id = i.id AND r.source = 'wealthsimple')"), 0, "the import named an instrument the feed did not");
    }

    /// Every operation is measured on a small book and four times it, in this
    /// process, and none off `OVER_BUDGET` breaks the growth rule there; the
    /// budget itself, and whether a listed one now meets it, is the CI job's, at
    /// the owner's size, on the runner it is stated for.
    #[test]
    fn test_every_operation_is_measured_and_the_growth_rule_judged() {
        let _g = crate::tests_common::guard();
        let (one, four) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        build(one.path(), SMALL, 2).unwrap();
        build(four.path(), Size { accounts: SMALL.accounts * 4, instruments: SMALL.instruments * 4, trades: SMALL.trades * 4, ..SMALL }, 2).unwrap();
        let listed = |op: Op| OVER_BUDGET.iter().any(|(name, _)| *name == op.name());
        for op in Op::ALL.into_iter().filter(|o| o.per_change()) {
            let (a, b) = (probe(op, one.path()).unwrap(), probe(op, four.path()).unwrap());
            assert!(a.steps > 0 || op == Op::FeedStep, "{op:?} counted no work: {a:?}");
            let v = judge(op, &a, &b);
            let growth: Vec<&String> = v.over.iter().filter(|o| o.contains("grew")).collect();
            assert!(growth.is_empty() || listed(op), "{op:?}: {growth:?} ({a:?} then {b:?})");
        }
    }

    /// The verdict: a listed operation within its budget fails the run as an
    /// unlisted one over it does.
    #[test]
    fn test_an_operation_over_budget_fails_unless_listed_and_a_listed_one_within_fails_too() {
        let small = Measured { wall_ns: 1_000, steps: 100, allocs: 100, read: Some(1_000), divisor: 10 };
        let linear = Measured { wall_ns: 1_000, steps: 400, allocs: 100, read: Some(1_000), divisor: 40 };
        let v = judge(Op::PriceChanged, &small, &linear);
        assert!(v.over.iter().any(|o| o.contains("steps grew 4.00 times")), "{v:?}");
        assert!(judge(Op::PriceChanged, &small, &small).over.is_empty());
        let slow = Measured { wall_ns: 10_000_000_000, ..small };
        assert!(judge(Op::Startup, &small, &slow).over.iter().any(|o| o.contains("over 5000 ms")));
        // a background operation's wall time is reported, never judged
        assert!(judge(Op::Brokers, &small, &Measured { wall_ns: 10_000_000_000, ..small }).over.is_empty());
        let off_list = listed_disagree(&[Verdict { op: Op::PriceChanged, over: vec!["x".into()] }]);
        assert_eq!(off_list.len(), usize::from(!OVER_BUDGET.iter().any(|(n, _)| *n == "price-changed")));
    }
}
