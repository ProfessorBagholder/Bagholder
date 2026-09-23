//! The import of an earlier version's database, on a fixture built here from
//! hand-written rows: one per row shape the plan's table lists, and the identity
//! cases. Every id, name and amount is made up (the repository is public).

mod common;

use std::path::Path;

use bagholder_book::import::{copy_database, old, ImportedNote, NoteOn, Translated, TranslatedGroup};
use bagholder_book::Book;
use bagholder_core::account::{AccountRef, AccountType};
use bagholder_core::instrument::InstrumentKind;
use bagholder_core::journal::{Anchor, JournalEntry, JournalSubject};
use bagholder_core::transaction::{Effect, Kind, Transaction};
use bagholder_core::{Broker, Currency, Dec};
use common::*;

/// The tables of schema 13 the import reads, as the earlier builds made them.
const OLD_SCHEMA: &str = "
CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT);
CREATE TABLE activities (id TEXT PRIMARY KEY, canonical_id TEXT, occurred_at TEXT, transaction_date TEXT NOT NULL, settlement_date TEXT,
    account_id TEXT, book_id TEXT, fifo_id TEXT, account_type TEXT, activity_type TEXT, activity_sub_type TEXT, description TEXT,
    direction TEXT, symbol TEXT, name TEXT, currency TEXT, quantity REAL, unit_price REAL, commission REAL, net_cash_amount REAL,
    category TEXT, balance REAL, source TEXT, raw_type TEXT, aft_type TEXT, counter_symbol TEXT, security_id TEXT);
CREATE TABLE securities (id TEXT PRIMARY KEY, symbol TEXT, name TEXT, primary_exchange TEXT, primary_mic TEXT, currency TEXT, underlying_id TEXT, fetched_at TEXT);
CREATE TABLE accounts (id TEXT PRIMARY KEY, nickname TEXT, unified_account_type TEXT, currency TEXT, status TEXT, type TEXT, net_liquidation_value REAL, margin_account_id TEXT);
INSERT INTO meta VALUES ('schema_version', '13');
";

/// A row: id, type, sub-type, Wealthsimple type, symbol, quantity, cash, security.
struct Row {
    id: &'static str,
    account: &'static str,
    ty: &'static str,
    sub: &'static str,
    raw: &'static str,
    symbol: &'static str,
    currency: &'static str,
    qty: f64,
    cash: f64,
    security: &'static str,
    source: &'static str,
    when: &'static str,
    direction: &'static str,
    price: f64,
    commission: f64,
}

fn row(id: &'static str, ty: &'static str, sub: &'static str, raw: &'static str, symbol: &'static str, qty: f64, cash: f64, security: &'static str) -> Row {
    Row { id, account: "tfsa-cad", ty, sub, raw, symbol, currency: "CAD", qty, cash, security, source: "wealthsimple", when: "2026-03-02T15:00:00.123456+00:00", direction: if cash < 0.0 { "DEBIT" } else { "CREDIT" }, price: 0.0, commission: 0.0 }
}

fn rows() -> Vec<Row> {
    vec![
        row("trade-buy", "Trade", "BUY", "DIY_BUY", "QNC", 10.0, -100.0, "sec-s-qnc"),
        row("trade-sell", "Trade", "SELL", "DIY_SELL", "QNC", -10.0, 105.5, "sec-s-qnc"),
        Row { commission: 4.95, ..row("trade-fee", "Trade", "BUY", "DIY_BUY", "QNC", 1.0, -14.95, "sec-s-qnc") },
        Row { currency: "USD", ..row("opt-bto", "OPTIONS_BUY", "BUYTOOPEN", "OPTIONS_BUY", "LUNR 15JAN27 12.00 CALL", 2.0, -300.0, "sec-o-lunr") },
        Row { currency: "USD", ..row("opt-multileg", "OPTIONS_BUY", "BUYTOCLOSE", "OPTIONS_MULTILEG", "LUNR 15JAN27 12.00 CALL", 0.0, -50.0, "sec-o-lunr") },
        Row { currency: "USD", ..row("opt-sto", "OPTIONS_SELL", "SELLTOOPEN", "OPTIONS_SELL", "LUNR 15JAN27 12.00 CALL", -2.0, 120.0, "sec-o-lunr") },
        Row { currency: "USD", ..row("expir-short", "EXPIR", "BUY", "OPTIONS_SHORT_EXPIRY", "LUNR 15JAN27 12.00 CALL", 2.0, 0.0, "sec-o-lunr") },
        Row { currency: "USD", ..row("expir-long", "EXPIR", "SELL", "OPTIONS_EXPIRY", "LUNR 15JAN27 12.00 CALL", -1.0, 0.0, "sec-o-lunr") },
        Row { currency: "USD", ..row("assign", "ASSIGN", "BUYTOCLOSE", "OPTIONS_ASSIGN", "LUNR 15JAN27 12.00 CALL", 1.0, 0.0, "sec-o-lunr") },
        row("stkdis-marker", "STKDIS", "BUY", "CORPORATE_ACTION", "QNC", 0.0, 0.0, "sec-s-qnc"),
        row("stkdis-in", "STKDIS", "BUY", "DIVIDEND", "QNC", 4000.0, 0.0, "sec-s-qnc"),
        row("dividend", "Dividend", "dividend", "DIVIDEND", "QNC", 100.0, 12.5, "sec-s-qnc"),
        row("dividend-reversal", "Dividend", "dividend", "DIVIDEND", "QNC", 0.0, -12.5, "sec-s-qnc"),
        row("interest", "Interest", "interest", "INTEREST", "", 0.0, 1.1, ""),
        row("interest-charge", "INTEREST_CHARGE", "MARGIN_INTEREST", "INTEREST_CHARGE", "CAD", 0.0, -9.9, "sec-c-cad"),
        row("withholding", "WITHHOLDING_TAX", "other", "WITHHOLDING_TAX", "", 0.0, -4.47, ""),
        Row { currency: "USD", ..row("fx", "FxExchange", "fx", "FUNDS_CONVERSION", "", 0.0, 65.16, "") },
        row("deposit", "Deposit", "deposit", "DEPOSIT", "", 0.0, 100.0, ""),
        row("employer", "GROUP_CONTRIBUTION", "EMPLOYER_CONTRIBUTION", "GROUP_CONTRIBUTION", "", 0.0, 102.07, ""),
        row("employee", "GROUP_CONTRIBUTION", "EMPLOYEE_CONTRIBUTION", "GROUP_CONTRIBUTION", "", 0.0, 102.07, ""),
        row("grant", "RESP_GRANT", "CESG", "RESP_GRANT", "CAD", 0.0, 100.0, "sec-c-cad"),
        row("withdrawal", "Withdrawal", "withdrawal", "WITHDRAWAL", "", 0.0, -50.0, ""),
        row("transfer-in", "Transfer", "transfer", "INTERNAL_TRANSFER", "", 0.0, 10.0, ""),
        row("transfer-out", "Transfer", "transfer", "INTERNAL_TRANSFER", "", 0.0, -10.0, ""),
        Row { direction: "DEBIT", ..row("transfer-zero", "Transfer", "transfer", "INTERNAL_TRANSFER", "", 0.0, 0.0, "") },
        row("asset-movement", "ASSET_MOVEMENT", "SOURCE", "ASSET_MOVEMENT", "", 0.0, -56.91, ""),
        Row { account: "crypto", ..row("crypto-buy", "CRYPTO_BUY", "MARKET_ORDER", "CRYPTO_BUY", "DOGE", 154.699294, 49.77, "sec-z-doge") },
        Row { account: "crypto", ..row("crypto-sell", "CRYPTO_SELL", "LIMIT_ORDER", "CRYPTO_SELL", "DOGE", 573.350399, 295.34, "sec-z-doge") },
        Row { account: "crypto", ..row("crypto-in", "CRYPTO_TRANSFER", "TRANSFER_IN", "CRYPTO_TRANSFER", "DOGE", 0.36162, 1249.7, "sec-z-doge") },
        Row { account: "crypto", ..row("crypto-out", "CRYPTO_TRANSFER", "TRANSFER_OUT", "CRYPTO_TRANSFER", "DOGE", 99.899429, -138.59, "sec-z-doge") },
        Row { account: "crypto", ..row("staking-reward", "CRYPTO_STAKING_REWARD", "other", "CRYPTO_STAKING_REWARD", "DOGE", 0.165761, 0.0, "sec-z-doge") },
        Row { account: "crypto", ..row("staking-move", "CRYPTO_STAKING_ACTION", "STAKE", "CRYPTO_STAKING_ACTION", "DOGE", 9.471899, 0.0, "sec-z-doge") },
        Row { currency: "USD", ..row("pred-buy", "PREDICTIONS_BUY", "MARKET_ORDER", "PREDICTIONS_BUY", "FED-YES", 307.6, 67.67, "sec-r-fed") },
        Row { currency: "USD", ..row("pred-resolution", "PREDICTIONS_RESOLUTION", "PREDICTIONS_EXPIRY", "PREDICTIONS_RESOLUTION", "FED-YES", 307.6, 0.0, "sec-r-fed") },
        Row { account: "card", ..row("card-purchase", "CREDIT_CARD", "PURCHASE", "CREDIT_CARD", "", 0.0, -36.4, "") },
        Row { account: "card", ..row("card-refund", "CREDIT_CARD", "REFUND", "CREDIT_CARD", "", 0.0, 582.42, "") },
        Row { account: "card", ..row("card-payment", "CREDIT_CARD", "PAYMENT", "CREDIT_CARD", "", 0.0, 2697.34, "") },
        row("card-paid", "CREDIT_CARD_PAYMENT", "other", "CREDIT_CARD_PAYMENT", "", 0.0, -2697.34, ""),
        Row { account: "card", ..row("cashback", "REIMBURSEMENT", "CASHBACK", "REIMBURSEMENT", "", 0.0, 96.66, "") },
        row("intent", "INSTITUTIONAL_TRANSFER_INTENT", "TRANSFER_IN", "INSTITUTIONAL_TRANSFER_INTENT", "", 0.0, 1910.26, ""),
        row("unknown", "SOMETHING_NEW", "other", "SOMETHING_NEW", "", 0.0, 1.0, ""),
        Row { source: "csv", when: "2025-03-01", price: 2.5, ..row("csv-buy", "Trade", "BUY", "", "ABC", 10.0, -25.0, "") },
        Row { source: "bagholder-fill", price: 1.23, ..row("booked-fill", "Trade", "BUY", "", "QNC", 5.0, -6.15, "sec-s-qnc") },
        Row { when: "2026-01-20T02:01:15.227185+00:00", ..row("late-evening", "Trade", "BUY", "DIY_BUY", "QNC", 1.0, -1.0, "sec-s-qnc") },
        row("ch-first", "Trade", "BUY", "DIY_BUY", "CH", 100.0, -15.0, "sec-s-ch-a"),
        row("ch-second", "Trade", "BUY", "DIY_BUY", "CH", 100.0, -15.0, "sec-s-ch-b"),
        Row { account: "tfsa-usd", currency: "USD", ..row("usd-side", "Trade", "BUY", "DIY_BUY", "LUNR", 1.0, -10.0, "sec-s-lunr") },
        Row { account: "stranger", ..row("no-account-row", "Deposit", "deposit", "DEPOSIT", "", 0.0, 5.0, "") },
        row("bad-number", "Trade", "BUY", "DIY_BUY", "QNC", f64::NAN, -1.0, "sec-s-qnc"),
        row("sign-against", "Trade", "BUY", "DIY_BUY", "QNC", 1.0, 5.0, "sec-s-qnc"),
        Row { direction: "", ..row("transfer-undirected", "Transfer", "transfer", "INTERNAL_TRANSFER", "", 0.0, 0.0, "") },
        row("no-symbol", "Trade", "BUY", "DIY_BUY", "", 2.0, -2.0, "sec-s-qnc"),
        row("no-currency", "Trade", "BUY", "DIY_BUY", "NOCUR", 3.0, -3.0, "sec-s-nocur"),
    ]
}

fn fixture(dir: &Path) -> std::path::PathBuf {
    let path = dir.join("bagholder.db");
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch(OLD_SCHEMA).unwrap();
    conn.execute_batch("
        INSERT INTO accounts(id, nickname, unified_account_type, currency, status, type) VALUES
          ('tfsa-cad', 'Trading', 'SELF_DIRECTED_TFSA', 'CAD', 'open', 'tfsa'),
          ('tfsa-usd', 'Trading', 'SELF_DIRECTED_TFSA', 'USD', 'closed', 'tfsa'),
          ('crypto', 'Coins', 'SELF_DIRECTED_CRYPTO', 'CAD', 'open', 'non_registered_crypto'),
          ('card', NULL, 'CREDIT_CARD', 'CAD', 'open', 'ca_credit_card'),
          ('odd-a', NULL, 'SELF_DIRECTED_RRSP', 'CAD', 'open', 'rrsp'),
          ('odd-b', NULL, 'SELF_DIRECTED_NON_REGISTERED', 'CAD', 'open', 'non_registered'),
          ('future', NULL, 'SELF_DIRECTED_SOMETHING_LATER', 'CAD', 'open', 'x');
        INSERT INTO securities(id, symbol, name, primary_exchange, primary_mic, currency, underlying_id) VALUES
          ('sec-s-qnc', 'QNC', 'Quantum Example Corp', 'TSX-V', 'XTSX', 'CAD', NULL),
          ('sec-s-lunr', 'LUNR', 'Lunar Example Inc', 'NASDAQ', 'XNAS', 'USD', NULL),
          ('sec-o-lunr', 'LUNR', NULL, NULL, NULL, 'USD', 'sec-s-lunr'),
          ('sec-s-ch-a', 'CH', 'Charbone Example Hydrogen', 'TSX-V', 'XTSX', 'CAD', NULL),
          ('sec-s-ch-b', 'CH', 'Charbone Example Corp.', 'TSX-V', 'XTSX', 'CAD', NULL),
          ('sec-z-doge', 'DOGE', 'Dogecoin', NULL, NULL, 'CAD', NULL),
          ('sec-r-fed', 'FED-YES', 'Will the rate hold? - Yes', 'KALSHI', NULL, 'USD', NULL),
          ('sec-c-cad', 'CAD', 'Canadian dollar', NULL, NULL, 'CAD', NULL),
          ('sec-s-nocur', 'NOCUR', 'No Currency Example', 'TSX', 'XTSE', NULL, NULL);
    ").unwrap();
    for r in rows() {
        let date = &r.when[..10];
        // the CAD side's rows were pooled under the USD side's id by the earlier app
        let fifo = match r.account {
            "tfsa-cad" => "tfsa-usd",
            "odd-a" => "odd-b",
            other => other,
        };
        let qty: rusqlite::types::Value = if r.qty.is_nan() { "abc".to_string().into() } else { r.qty.into() };
        conn.execute(
            "INSERT INTO activities(id, canonical_id, occurred_at, transaction_date, settlement_date, account_id, book_id, fifo_id, activity_type, activity_sub_type,
                                    direction, symbol, name, currency, quantity, unit_price, commission, net_cash_amount, category, source, raw_type, security_id)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'x', ?, ?, ?)",
            rusqlite::params![
                r.id, format!("ws-{}", r.id), r.when, date, date, r.account, r.account, fifo, r.ty, r.sub, r.direction, r.symbol, r.symbol,
                r.currency, qty, r.price, r.commission, r.cash, r.source, r.raw, if r.security.is_empty() { None } else { Some(r.security) }
            ],
        )
        .unwrap();
    }
    // a pool the earlier app made of two accounts the broker types differently
    conn.execute_batch("INSERT INTO activities(id, occurred_at, transaction_date, account_id, fifo_id, activity_type, activity_sub_type, currency, net_cash_amount, source)
                        VALUES ('odd-deposit', '2026-03-02T15:00:00+00:00', '2026-03-02', 'odd-a', 'odd-b', 'Deposit', 'deposit', 'CAD', 1, 'wealthsimple')").unwrap();
    path
}

struct Imported {
    dir: tempfile::TempDir,
    book: Book,
    report: bagholder_book::import::Report,
}

fn translated() -> Translated {
    let entry = |t: &str| JournalEntry { thesis: t.into(), grade: Some(bagholder_core::journal::Grade::A), tags: vec!["winners".into()] };
    Translated {
        journal: vec![
            ImportedNote { key: "rt:trade-buy".into(), on: NoteOn::Trade(Ok("trade-buy".into())), entry: entry("on the buy") },
            ImportedNote { key: "rt:trade-buy|again".into(), on: NoteOn::Trade(Ok("trade-buy".into())), entry: entry("a second note on the same trade") },
            ImportedNote { key: "rt:made-up".into(), on: NoteOn::Trade(Ok("made-up".into())), entry: entry("on a row the earlier app made up") },
            ImportedNote { key: "rt:bad-number".into(), on: NoteOn::Trade(Ok("bad-number".into())), entry: entry("on a row that did not book") },
            ImportedNote { key: "g_1".into(), on: NoteOn::Trade(Err("no trade the earlier app matched has this key".into())), entry: entry("lost trade") },
            ImportedNote { key: "saved-1".into(), on: NoteOn::Group("saved-1".into()), entry: entry("on the group") },
            ImportedNote { key: "rt:opt-multileg".into(), on: NoteOn::Trade(Ok("opt-multileg".into())), entry: entry("on a fill with no quantity") },
        ],
        groups: vec![TranslatedGroup {
            key: "saved-1".into(),
            locked: true,
            members: vec![
                ("trade-buy|trade-sell|10".into(), Ok("trade-buy".into())),
                ("trade-fee|x|1".into(), Ok("trade-fee".into())),
                ("gone|x|1".into(), Err("the earlier app no longer matched this piece of the group's trade".into())),
            ],
        }],
    }
}

fn import() -> Imported {
    let dir = tempfile::tempdir().unwrap();
    let source = fixture(dir.path());
    let before = std::fs::read(&source).unwrap();
    let copy = dir.path().join("copy.db");
    copy_database(&source, &copy).unwrap();
    let old = old::read(&copy).unwrap();
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let (book, _) = Book::open_in(&home, "test", t0()).unwrap();
    let report = book.import(&old, &translated(), t0()).unwrap();
    assert_eq!(std::fs::read(&source).unwrap(), before, "the earlier database is only read");
    Imported { dir, book, report }
}

fn tx(i: &Imported, row: &str) -> Option<Transaction> {
    let conn = i.book.connections().unwrap()[0].id;
    let record = i.book.record_by_key(Some(conn), &bagholder_book::import::import_source(), row).unwrap().unwrap_or_else(|| panic!("no record for {row}"));
    let mut all = i.book.transactions_of(record).unwrap();
    assert!(all.len() <= 1, "{row} became {} transactions", all.len());
    all.pop()
}

fn problems(i: &Imported, row: &str) -> Vec<String> {
    let conn = i.book.connections().unwrap()[0].id;
    let record = i.book.record_by_key(Some(conn), &bagholder_book::import::import_source(), row).unwrap().unwrap();
    i.book.problems_of(record).unwrap().into_iter().map(|p| p.code).collect()
}

fn money(t: &Option<bagholder_core::Money>) -> Option<String> {
    t.map(|m| format!("{} {}", m.amount, m.currency))
}

fn q(t: &Transaction) -> Option<String> {
    t.quantity.map(|d| d.to_string())
}

#[test]
fn every_row_shape_books_as_the_table_says() {
    let i = import();
    // (row, kind, effect, quantity, cash, instrument kind)
    let cases: Vec<(&str, Kind, Option<Effect>, Option<&str>, Option<&str>, Option<InstrumentKind>)> = vec![
        ("trade-buy", Kind::Buy, None, Some("10"), Some("-100 CAD"), Some(InstrumentKind::Security)),
        ("trade-sell", Kind::Sell, None, Some("-10"), Some("105.5 CAD"), Some(InstrumentKind::Security)),
        ("opt-bto", Kind::Buy, None, Some("2"), Some("-300 USD"), Some(InstrumentKind::OptionContract)),
        ("opt-multileg", Kind::Buy, None, None, Some("-50 USD"), Some(InstrumentKind::OptionContract)),
        ("opt-sto", Kind::Sell, None, Some("-2"), Some("120 USD"), Some(InstrumentKind::OptionContract)),
        ("expir-short", Kind::OptionExpiry, None, Some("2"), None, Some(InstrumentKind::OptionContract)),
        ("expir-long", Kind::OptionExpiry, None, Some("-1"), None, Some(InstrumentKind::OptionContract)),
        ("assign", Kind::OptionAssignment, None, Some("1"), None, Some(InstrumentKind::OptionContract)),
        ("stkdis-marker", Kind::CorporateEvent, None, None, None, Some(InstrumentKind::Security)),
        ("stkdis-in", Kind::CorporateEvent, None, Some("4000"), None, Some(InstrumentKind::Security)),
        ("dividend", Kind::Dividend, None, None, Some("12.5 CAD"), Some(InstrumentKind::Security)),
        ("dividend-reversal", Kind::Dividend, None, None, Some("-12.5 CAD"), Some(InstrumentKind::Security)),
        ("interest", Kind::Interest, None, None, Some("1.1 CAD"), None),
        ("interest-charge", Kind::InterestCharge, None, None, Some("-9.9 CAD"), None),
        ("withholding", Kind::WithholdingTax, None, None, Some("-4.47 CAD"), None),
        ("fx", Kind::CurrencyConversion, None, None, Some("65.16 USD"), None),
        ("deposit", Kind::Deposit, None, None, Some("100 CAD"), None),
        ("employer", Kind::EmployerDeposit, None, None, Some("102.07 CAD"), None),
        ("employee", Kind::Deposit, None, None, Some("102.07 CAD"), None),
        ("grant", Kind::GovernmentDeposit, None, None, Some("100 CAD"), None),
        ("withdrawal", Kind::Withdrawal, None, None, Some("-50 CAD"), None),
        ("transfer-in", Kind::TransferIn, None, None, Some("10 CAD"), None),
        ("transfer-out", Kind::TransferOut, None, None, Some("-10 CAD"), None),
        ("transfer-zero", Kind::TransferOut, None, None, Some("0 CAD"), None),
        ("asset-movement", Kind::TransferOut, None, None, Some("-56.91 CAD"), None),
        ("crypto-buy", Kind::Buy, None, Some("154.699294"), Some("-49.77 CAD"), Some(InstrumentKind::Crypto)),
        ("crypto-sell", Kind::Sell, None, Some("-573.350399"), Some("295.34 CAD"), Some(InstrumentKind::Crypto)),
        ("crypto-in", Kind::TransferIn, None, Some("0.36162"), None, Some(InstrumentKind::Crypto)),
        ("crypto-out", Kind::TransferOut, None, Some("-99.899429"), None, Some(InstrumentKind::Crypto)),
        ("staking-reward", Kind::StakingReward, None, Some("0.165761"), None, Some(InstrumentKind::Crypto)),
        ("staking-move", Kind::StakingMove, None, None, None, Some(InstrumentKind::Crypto)),
        ("pred-buy", Kind::Buy, None, Some("307.6"), Some("-67.67 USD"), Some(InstrumentKind::EventContract)),
        ("pred-resolution", Kind::Resolution, None, Some("-307.6"), Some("0 USD"), Some(InstrumentKind::EventContract)),
        ("card-purchase", Kind::CardPurchase, None, None, Some("-36.4 CAD"), None),
        ("card-refund", Kind::CardRefund, None, None, Some("582.42 CAD"), None),
        ("card-payment", Kind::TransferIn, None, None, Some("2697.34 CAD"), None),
        ("card-paid", Kind::TransferOut, None, None, Some("-2697.34 CAD"), None),
        ("cashback", Kind::Cashback, None, None, Some("96.66 CAD"), None),
        ("intent", Kind::Unclassified, None, None, None, None),
        ("unknown", Kind::Unclassified, None, None, None, None),
    ];
    for (row, kind, effect, quantity, cash, instrument) in cases {
        let t = tx(&i, row).unwrap_or_else(|| panic!("{row} booked nothing: {:?}", problems(&i, row)));
        let got_kind = t.instrument.map(|id| i.book.instrument(id).unwrap().kind);
        assert_eq!(
            (t.kind, t.effect, q(&t).as_deref(), money(&t.cash).as_deref(), got_kind),
            (kind, effect, quantity, cash, instrument),
            "{row}"
        );
        // a synced row states no price
        assert_eq!(t.price, None, "{row}");
    }
    assert_eq!(problems(&i, "opt-multileg"), vec!["leg-unstated"]);
    assert_eq!(problems(&i, "intent"), vec!["unclassified"]);
    assert_eq!(problems(&i, "unknown"), vec!["unclassified"]);
    assert!(problems(&i, "trade-buy").is_empty());
}

#[test]
fn what_each_origin_states_is_what_is_kept() {
    let i = import();
    // a commission is the fee
    assert_eq!(money(&tx(&i, "trade-fee").unwrap().fee).as_deref(), Some("4.95 CAD"));
    // a CSV row: its own day, no instant, its stated price, named by symbol within the connection
    let csv = tx(&i, "csv-buy").unwrap();
    assert_eq!((csv.occurred_at, csv.trade_date.to_string(), money(&csv.price).as_deref()), (None, "2025-03-01".to_string(), Some("2.5 CAD")));
    let refs = i.book.instrument_refs(csv.instrument.unwrap()).unwrap();
    assert!(refs[0].scheme.to_text().starts_with("connection-symbol:") && refs[0].value == "ABC|CAD", "{refs:?}");
    // a booked fill: the broker's average price, and no cash (it was the earlier app's arithmetic)
    let fill = tx(&i, "booked-fill").unwrap();
    assert_eq!((money(&fill.price).as_deref(), fill.cash), (Some("1.23 CAD"), None));
    // a synced row is filed on Alberta's day
    let late = tx(&i, "late-evening").unwrap();
    assert_eq!((late.trade_date.to_string(), late.occurred_at.unwrap().to_string()), ("2026-01-19".to_string(), "2026-01-20T02:01:15.227185Z".to_string()));
    // a stored value that is not a number books nothing, and says so
    assert!(tx(&i, "bad-number").is_none());
    assert_eq!(problems(&i, "bad-number"), vec!["unreadable-number"]);
}

#[test]
fn identity_is_carried_as_the_broker_gave_it() {
    let i = import();
    let ws = Broker::named("wealthsimple");
    // one ticker under two security ids: two instruments
    assert_ne!(tx(&i, "ch-first").unwrap().instrument, tx(&i, "ch-second").unwrap().instrument);
    // the linked CAD and USD sides are one account, holding both currencies
    let cad = i.book.account_by_ref(&AccountRef::new(ws.clone(), "tfsa-cad")).unwrap().unwrap();
    let usd = i.book.account_by_ref(&AccountRef::new(ws.clone(), "tfsa-usd")).unwrap().unwrap();
    assert_eq!(cad, usd);
    assert_eq!(tx(&i, "usd-side").unwrap().account, cad);
    assert_eq!(tx(&i, "usd-side").unwrap().cash.unwrap().currency, Currency::USD);
    // a pool of two accounts the broker types differently is not merged, and it is said
    let a = i.book.account_by_ref(&AccountRef::new(ws.clone(), "odd-a")).unwrap().unwrap();
    let b = i.book.account_by_ref(&AccountRef::new(ws.clone(), "odd-b")).unwrap().unwrap();
    assert_ne!(a, b);
    assert!(i.report.account_problems.iter().any(|p| p.contains("odd-a") && p.contains("odd-b")), "{:?}", i.report.account_problems);
    // a broker type not in the vocabulary is kept in the broker's words
    let future = i.book.account_by_ref(&AccountRef::new(ws.clone(), "future")).unwrap().unwrap();
    assert_eq!(i.book.account(future).unwrap().account_type, AccountType::Unrecognised("SELF_DIRECTED_SOMETHING_LATER".into()));
    // an account rows name that the accounts table lacks is still an account
    let stranger = i.book.account_by_ref(&AccountRef::new(ws, "stranger")).unwrap().unwrap();
    assert!(matches!(i.book.account(stranger).unwrap().account_type, AccountType::Unrecognised(_)));
    // an option's terms from its printed name, its underlying from the security row, no multiplier
    let opt = tx(&i, "opt-bto").unwrap().instrument.unwrap();
    let terms = i.book.option_terms(opt).unwrap().unwrap();
    assert_eq!((terms.strike, terms.expiry.to_string(), terms.multiplier), (Dec::parse("12").unwrap(), "2027-01-15".to_string(), None));
    assert_eq!(terms.underlying, tx(&i, "usd-side").unwrap().instrument.unwrap());
    // every record carries its Wealthsimple id, for the raw row that will replace it
    let found = i.book.records_by_ref(bagholder_book::import::WEALTHSIMPLE_RECORD, "ws-trade-buy").unwrap();
    assert_eq!(found.len(), 1);
}

#[test]
fn notes_and_groups_are_placed_or_kept_with_the_reason() {
    let i = import();
    let entries = i.book.journal_entries().unwrap();
    assert_eq!(entries.len(), 7, "no note is lost: {entries:?}");
    let trade = |row: &str| {
        let t = tx(&i, row).unwrap();
        i.book.trade_on(&bagholder_core::journal::Opening { transaction: t.id.clone(), instrument: t.instrument.unwrap() }).unwrap().unwrap()
    };
    assert_eq!(i.book.journal(JournalSubject::Trade(trade("trade-buy"))).unwrap().unwrap().thesis, "on the buy");
    let orphaned: Vec<(String, String)> = i.book.orphaned_journal().unwrap().into_iter().map(|(t, e)| (t.legacy_key.unwrap(), e.thesis)).collect();
    let keys: Vec<&str> = orphaned.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(keys.len(), 5, "{orphaned:?}");
    for k in ["rt:trade-buy|again", "rt:made-up", "rt:bad-number", "g_1", "rt:opt-multileg"] {
        assert!(keys.contains(&k), "{k} in {keys:?}");
    }
    assert_eq!(i.report.journal_attached, 2, "the buy's note and the group's: {:?}", i.report);
    assert!(i.report.journal_orphaned.iter().any(|(k, why)| k == "rt:bad-number" && why.contains("could not be booked")), "{:?}", i.report.journal_orphaned);
    // the group: its two members' trades, the lost piece kept orphaned, and its note
    let groups = i.book.groups().unwrap();
    assert_eq!(groups.len(), 1);
    assert!(groups[0].locked);
    assert_eq!(groups[0].members.len(), 3);
    assert_eq!(groups[0].members[0], trade("trade-buy"));
    assert_eq!(groups[0].members[1], trade("trade-fee"));
    assert!(matches!(i.book.trade(groups[0].members[2]).unwrap().anchor, Anchor::Orphaned(_)));
    assert_eq!(i.book.journal(JournalSubject::Group(groups[0].id)).unwrap().unwrap().thesis, "on the group");
    assert_eq!((i.report.group_members_attached, i.report.group_members_orphaned.len()), (2, 1));
}

#[test]
fn importing_again_changes_nothing() {
    let i = import();
    let before = everything(&i.dir.path().join("home"));
    let copy = i.dir.path().join("copy2.db");
    copy_database(&i.dir.path().join("bagholder.db"), &copy).unwrap();
    let again = i.book.import(&old::read(&copy).unwrap(), &translated(), at("2026-09-24T12:00:00Z")).unwrap();
    assert_eq!((again.records_new, again.records_revised, again.accounts_made), (0, 0, 0));
    assert_eq!(again.records_unchanged, i.report.records_new);
    assert_eq!(everything(&i.dir.path().join("home")), before);
}

#[test]
fn every_row_is_one_record_with_a_transaction_or_a_problem() {
    let i = import();
    let n = rows().len() + 1;
    assert_eq!((i.report.rows_read, i.report.records_new), (n, n));
    let conn = i.book.connections().unwrap()[0].id;
    for r in rows() {
        let record = i.book.record_by_key(Some(conn), &bagholder_book::import::import_source(), r.id).unwrap().unwrap();
        let has_tx = !i.book.transactions_of(record).unwrap().is_empty();
        let has_problem = !i.book.problems_of(record).unwrap().is_empty();
        assert!(has_tx || has_problem, "{} has neither", r.id);
    }
    let counted: usize = i.report.transactions_by_kind.values().sum();
    assert_eq!(counted, i.book.transactions().unwrap().len());
}

#[test]
fn a_database_of_another_schema_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = fixture(dir.path());
    rusqlite::Connection::open(&path).unwrap().execute("UPDATE meta SET value = '12' WHERE key = 'schema_version'", []).unwrap();
    assert!(old::read(&path).is_err());
}

#[test]
fn an_empty_database_makes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("empty.db");
    rusqlite::Connection::open(&path).unwrap().execute_batch(OLD_SCHEMA).unwrap();
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let (book, _) = Book::open_in(&home, "test", t0()).unwrap();
    let report = book.import(&old::read(&path).unwrap(), &Translated::default(), t0()).unwrap();
    assert_eq!(report.rows_read, 0);
    assert!(book.connections().unwrap().is_empty());
}

#[test]
fn what_the_earlier_rows_do_not_state_is_shown_not_guessed() {
    let i = import();
    // a sign the earlier app set against the kind: booked as the kind says, and shown
    let t = tx(&i, "sign-against").unwrap();
    assert_eq!(money(&t.cash).as_deref(), Some("-5 CAD"));
    assert_eq!(problems(&i, "sign-against"), vec!["sign-against-kind"]);
    // a transfer of nothing, with no direction, is not placed
    assert_eq!(tx(&i, "transfer-undirected").unwrap().kind, Kind::Unclassified);
    assert_eq!(problems(&i, "transfer-undirected"), vec!["unclassified"]);
    // a security id with no symbol still names its instrument
    assert_eq!(tx(&i, "no-symbol").unwrap().instrument, tx(&i, "trade-buy").unwrap().instrument);
    assert_eq!(q(&tx(&i, "no-symbol").unwrap()).as_deref(), Some("2"));
    // a security whose currency the database does not state: no instrument is made
    // up for it, and the cash is still booked
    let t = tx(&i, "no-currency").unwrap();
    assert_eq!((t.instrument, t.quantity, money(&t.cash).as_deref()), (None, None, Some("-3 CAD")));
    assert_eq!(problems(&i, "no-currency"), vec!["instrument-currency-unknown"]);
    // an account type not in the vocabulary is reported, not only kept
    assert!(i.report.account_problems.iter().any(|p| p.contains("future") && p.contains("SELF_DIRECTED_SOMETHING_LATER")), "{:?}", i.report.account_problems);
    // a note on a row that moves no position is kept, orphaned, with why
    assert!(i.report.journal_orphaned.iter().any(|(k, why)| k == "rt:opt-multileg" && why.contains("legs")), "{:?}", i.report.journal_orphaned);
}

#[test]
fn a_pool_that_grew_since_the_last_import_gains_the_new_id() {
    let i = import();
    let source = i.dir.path().join("bagholder.db");
    rusqlite::Connection::open(&source)
        .unwrap()
        .execute_batch("INSERT INTO activities(id, occurred_at, transaction_date, account_id, fifo_id, activity_type, activity_sub_type, currency, net_cash_amount, source, direction)
                        VALUES ('later', '2026-04-02T15:00:00+00:00', '2026-04-02', 'tfsa-extra', 'tfsa-usd', 'Deposit', 'deposit', 'CAD', 7, 'wealthsimple', 'CREDIT')")
        .unwrap();
    let copy = i.dir.path().join("copy3.db");
    copy_database(&source, &copy).unwrap();
    let again = i.book.import(&old::read(&copy).unwrap(), &translated(), t0()).unwrap();
    assert_eq!((again.records_new, again.accounts_made), (1, 0));
    let ws = Broker::named("wealthsimple");
    let tfsa = i.book.account_by_ref(&AccountRef::new(ws.clone(), "tfsa-cad")).unwrap().unwrap();
    assert_eq!(i.book.account_by_ref(&AccountRef::new(ws, "tfsa-extra")).unwrap(), Some(tfsa));
    assert_eq!(tx(&i, "later").unwrap().account, tfsa);
}

#[test]
fn notes_in_a_database_with_nothing_else_are_kept() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("empty.db");
    rusqlite::Connection::open(&path).unwrap().execute_batch(OLD_SCHEMA).unwrap();
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let (book, _) = Book::open_in(&home, "test", t0()).unwrap();
    let notes = Translated {
        journal: vec![ImportedNote { key: "rt:x".into(), on: NoteOn::Trade(Ok("x".into())), entry: JournalEntry { thesis: "alone".into(), grade: None, tags: vec![] } }],
        groups: vec![],
    };
    let report = book.import(&old::read(&path).unwrap(), &notes, t0()).unwrap();
    assert_eq!(report.journal_orphaned.len(), 1);
    assert_eq!(book.orphaned_journal().unwrap()[0].1.thesis, "alone");
    assert!(book.connections().unwrap().is_empty());
}

#[test]
fn a_row_the_earlier_app_could_not_have_written_stops_the_import_saying_where() {
    for (what, sql) in [
        ("no id", "INSERT INTO activities(id, transaction_date, account_id, source) VALUES (NULL, '2026-01-02', 'tfsa-cad', 'wealthsimple')"),
        ("empty id", "INSERT INTO activities(id, transaction_date, account_id, source) VALUES ('', '2026-01-02', 'tfsa-cad', 'wealthsimple')"),
        ("a blob", "INSERT INTO activities(id, transaction_date, account_id, source, symbol) VALUES ('blobby', '2026-01-02', 'tfsa-cad', 'wealthsimple', x'00ff')"),
        ("not UTF-8", "INSERT INTO activities(id, transaction_date, account_id, source, symbol) VALUES ('bytes', '2026-01-02', 'tfsa-cad', 'wealthsimple', CAST(x'c328' AS TEXT))"),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = fixture(dir.path());
        rusqlite::Connection::open(&path).unwrap().execute(sql, []).unwrap();
        let err = old::read(&path).unwrap_err().to_string();
        assert!(err.contains("activities"), "{what}: {err}");
    }
}
