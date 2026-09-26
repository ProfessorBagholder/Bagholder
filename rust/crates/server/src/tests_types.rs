//! The page's order types are generated from the server's (`web/src/lib/generated/orders.ts`),
//! as the model's are from the model's: a field renamed, added or made nullable here
//! fails the page's type check there.
//!
//! After an intended change: `BAGHOLDER_BLESS=1 cargo test -p bagholder-server the_pages_order_types`,
//! and the page's `npm run check` says what it breaks.

use std::path::PathBuf;
use ts_rs::TS;

use bagholder_store::bars::{ChartBars, DayBar, TimeBar};
use bagholder_store::feeds::{
    FiledDocument, Filing, Gauge, GaugePart, GaugePoint, GaugeReading, Regulator, ShortMarket, ShortPoint, Shorts, StoredGauge, StoredShorts, VolumeSpan,
};
use bagholder_store::orders::{Bracket, BracketStatus, Order, OrderStatus, OrderType, Role, Side, SlKind, SlMode, Source, StopLoss, TakeProfit, TrailUnit};

use bagholder_store::activities::ActivityRow;
use bagholder_store::tables::LegacyNote;
use bagholder_store::feeds::{Notification, NotificationExtra};

use crate::feeds::{ChartHistory, Enriched, FearDoc, FeedFiling, FilingsDoc, FilingsFeed, FilingsPayload, ShortsFeed, ShortsFeedRow, ShortsPayload, SourceStatus};
use crate::http::orders::{Adjust, Modify, Named, QuoteOf, RefreshAndOrders};
use crate::orders::{
    OrderAccount, OrderActionAnswer, OrderCard, OrdersDoc, PlaceTicketAnswer, RefreshOrdersAnswer, Ticket, TicketQuote, TicketQuoteDetail, TicketQuoteOk, TicketStop, TicketTarget,
};

fn declarations() -> String {
    let config = ts_rs::Config::new().with_large_int("number");
    macro_rules! decls {
        ($($t:ty),* $(,)?) => { vec![$(<$t>::decl(&config)),*] };
    }
    let decls: Vec<String> = decls![
        Side, OrderType, OrderStatus, Role, Source, BracketStatus, SlKind, TrailUnit, SlMode, StopLoss, TakeProfit, Order, OrderCard, Bracket, OrdersDoc,
        OrderActionAnswer, RefreshOrdersAnswer, Named, Modify, Adjust, RefreshAndOrders, QuoteOf, OrderAccount, TicketQuoteDetail, TicketQuoteOk, TicketQuote, TicketStop, TicketTarget, Ticket,
        PlaceTicketAnswer,
        crate::orders::preview::StopInput, crate::orders::preview::TargetInput, crate::orders::preview::QuoteInput, crate::orders::preview::PreviewRequest, crate::orders::preview::Preview,
    ];
    let mut out = String::from("// Generated from rust/crates/store/src/orders/types.rs and the server's orders document. Do not\n// edit: change the Rust type, then `BAGHOLDER_BLESS=1 cargo test -p bagholder-server the_pages_order_types`.\n\nimport type { OkOr } from './common'\nimport type { Dec } from '../dec'\nimport type { Fig } from './figures'\n\n");
    for d in decls {
        out.push_str("export ");
        out.push_str(d.trim());
        out.push_str("\n\n");
    }
    out.trim_end().to_string() + "\n"
}

#[test]
fn test_the_pages_order_types_are_the_servers() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../web/src/lib/generated/orders.ts");
    let want = declarations();
    if std::env::var("BAGHOLDER_BLESS").map_or(false, |v| v == "1") {
        std::fs::write(&path, &want).unwrap();
        return;
    }
    let have = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(have == want, "web/src/lib/generated/orders.ts is not what the server's types generate: run with BAGHOLDER_BLESS=1 and check the page");
}

/// The chart's bar types, generated to `web/src/lib/generated/chart.ts`
/// alongside the orders document -- a field renamed, added or made nullable
/// on `DayBar`, `TimeBar` or `ChartHistory` fails the page's type check.
fn chart_declarations() -> String {
    let config = ts_rs::Config::new().with_large_int("number");
    macro_rules! decls {
        ($($t:ty),* $(,)?) => { vec![$(<$t>::decl(&config)),*] };
    }
    let decls: Vec<String> = decls![DayBar, TimeBar, ChartBars, ChartHistory, crate::feeds::HistoryAnswer, crate::feeds::HistoryQuery];
    let mut out = String::from("// Generated from rust/crates/store/src/bars.rs and the server's chart history. Do not\n// edit: change the Rust type, then `BAGHOLDER_BLESS=1 cargo test -p bagholder-server the_pages_chart_types`.\n\nimport type { OkOr } from './common'\n\n");
    for d in decls {
        out.push_str("export ");
        out.push_str(d.trim());
        out.push_str("\n\n");
    }
    out.trim_end().to_string() + "\n"
}

#[test]
fn test_the_pages_chart_types_are_the_servers() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../web/src/lib/generated/chart.ts");
    let want = chart_declarations();
    if std::env::var("BAGHOLDER_BLESS").map_or(false, |v| v == "1") {
        std::fs::write(&path, &want).unwrap();
        return;
    }
    let have = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(have == want, "web/src/lib/generated/chart.ts is not what the server's types generate: run with BAGHOLDER_BLESS=1 and check the page");
}

/// The filings pipeline's types, generated to `web/src/lib/generated/filings.ts` --
/// a field renamed, added or made nullable on any of these fails the page's type check.
fn filings_declarations() -> String {
    let config = ts_rs::Config::new().with_large_int("number");
    macro_rules! decls {
        ($($t:ty),* $(,)?) => { vec![$(<$t>::decl(&config)),*] };
    }
    let decls: Vec<String> = decls![
        Regulator, FiledDocument, Filing, SourceStatus, FilingsDoc, FilingsPayload, FeedFiling, FilingsFeed, Enriched,
        crate::feeds::FilingsAnswer, crate::feeds::EnrichAnswer, crate::http::markets::Filings, crate::http::markets::Scope, crate::http::markets::Document,
    ];
    let mut out = String::from("// Generated from rust/crates/store/src/feeds.rs and the server's filings documents. Do not\n// edit: change the Rust type, then `BAGHOLDER_BLESS=1 cargo test -p bagholder-server the_pages_filing_types`.\n\nimport type { OkOr } from './common'\nimport type { Listing } from './markets'\n\n");
    for d in decls {
        out.push_str("export ");
        out.push_str(d.trim());
        out.push_str("\n\n");
    }
    out.trim_end().to_string() + "\n"
}

#[test]
fn test_the_pages_filing_types_are_the_servers() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../web/src/lib/generated/filings.ts");
    let want = filings_declarations();
    if std::env::var("BAGHOLDER_BLESS").map_or(false, |v| v == "1") {
        std::fs::write(&path, &want).unwrap();
        return;
    }
    let have = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(have == want, "web/src/lib/generated/filings.ts is not what the server's types generate: run with BAGHOLDER_BLESS=1 and check the page");
}

/// The market feeds' types -- the published gauges -- generated to
/// `web/src/lib/generated/markets.ts`.
fn markets_declarations() -> String {
    let config = ts_rs::Config::new().with_large_int("number");
    macro_rules! decls {
        ($($t:ty),* $(,)?) => { vec![$(<$t>::decl(&config)),*] };
    }
    let decls: Vec<String> = decls![
        GaugeReading, GaugePart, GaugePoint, Gauge, StoredGauge, FearDoc, crate::feeds::UniverseDoc, crate::feeds::NewsDoc,
        ShortMarket, VolumeSpan, ShortPoint, Shorts, StoredShorts, ShortsPayload, ShortsFeedRow, ShortsFeed,
        crate::feeds::FearAnswer, crate::feeds::ShortsAnswer, crate::feeds::ListingAnswer, crate::feeds::NewsSymbolAnswer, crate::feeds::WatchlistAnswer, crate::feeds::TilesAnswer,
        crate::http::markets::Listing, crate::http::markets::Fear, crate::http::markets::ShortsQuery, crate::http::markets::GlanceAnswer,
        crate::http::markets::Search, crate::http::markets::SymbolSearchAnswer, crate::http::markets::WatchlistBody, crate::http::markets::TilesSet,
        bagholder_store::feeds::WatchedListing, bagholder_model::input::TileRef,
    ];
    let mut out = String::from("// Generated from rust/crates/store/src/feeds.rs and the server's market documents. Do not\n// edit: change the Rust type, then `BAGHOLDER_BLESS=1 cargo test -p bagholder-server the_pages_market_types`.\n\nimport type { OkOr } from './common'\nimport type { Fill } from './figures'\nimport type { MarketTile, SymbolMatch } from './wire'\n\n");
    for d in decls {
        out.push_str("export ");
        out.push_str(d.trim());
        out.push_str("\n\n");
    }
    out.trim_end().to_string() + "\n"
}

#[test]
fn test_the_pages_market_types_are_the_servers() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../web/src/lib/generated/markets.ts");
    let want = markets_declarations();
    if std::env::var("BAGHOLDER_BLESS").map_or(false, |v| v == "1") {
        std::fs::write(&path, &want).unwrap();
        return;
    }
    let have = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(have == want, "web/src/lib/generated/markets.ts is not what the server's types generate: run with BAGHOLDER_BLESS=1 and check the page");
}

/// The book's types -- an activity row, and the import/watch/append answers --
/// generated to `web/src/lib/generated/book.ts`.
fn book_declarations() -> String {
    let config = ts_rs::Config::new().with_large_int("number");
    macro_rules! decls {
        ($($t:ty),* $(,)?) => { vec![$(<$t>::decl(&config)),*] };
    }
    let decls: Vec<String> = decls![
        ActivityRow, LegacyNote,
        bagholder_store::broker::Account, bagholder_model::securities::Security, bagholder_store::book::BookBalance, bagholder_store::book::BookNav, bagholder_store::book::Book,
    ];
    let mut out = String::from(
        "// Generated from rust/crates/store/src/activities.rs, book.rs,\n// broker.rs and the server's book-append answer. Do not edit: change the Rust type, then\n// `BAGHOLDER_BLESS=1 cargo test -p bagholder-server the_pages_book_types`.\n\nimport type { TradeGroup } from './model_api'\n\n",
    );
    for d in decls {
        out.push_str("export ");
        out.push_str(d.trim());
        out.push_str("\n\n");
    }
    out.trim_end().to_string() + "\n"
}

#[test]
fn test_the_pages_book_types_are_the_servers() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../web/src/lib/generated/book.ts");
    let want = book_declarations();
    if std::env::var("BAGHOLDER_BLESS").map_or(false, |v| v == "1") {
        std::fs::write(&path, &want).unwrap();
        return;
    }
    let have = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(have == want, "web/src/lib/generated/book.ts is not what the server's types generate: run with BAGHOLDER_BLESS=1 and check the page");
}

/// The header's status, generated to `web/src/lib/generated/status.ts` -- a
/// field renamed, added or made nullable on `Status`, `NotifyStatus` or
/// `NotifySettings` fails the page's type check.
fn status_declarations() -> String {
    let config = ts_rs::Config::new().with_large_int("number");
    macro_rules! decls {
        ($($t:ty),* $(,)?) => { vec![$(<$t>::decl(&config)),*] };
    }
    let decls: Vec<String> = decls![crate::notify::NotifySettings, crate::notify::NotifyStatus, crate::status::Status, crate::status::StatusAnswer];
    let mut out = String::from("// Generated from the server's status and notify modules. Do not edit: change the Rust type,\n// then `BAGHOLDER_BLESS=1 cargo test -p bagholder-server the_pages_status_types`.\n\n");
    for d in decls {
        out.push_str("export ");
        out.push_str(d.trim());
        out.push_str("\n\n");
    }
    out.trim_end().to_string() + "\n"
}

#[test]
fn test_the_pages_status_types_are_the_servers() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../web/src/lib/generated/status.ts");
    let want = status_declarations();
    if std::env::var("BAGHOLDER_BLESS").map_or(false, |v| v == "1") {
        std::fs::write(&path, &want).unwrap();
        return;
    }
    let have = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(have == want, "web/src/lib/generated/status.ts is not what the server's types generate: run with BAGHOLDER_BLESS=1 and check the page");
}

/// The notification list's own types, generated to `web/src/lib/generated/notifications.ts`
/// -- a field renamed, added or made nullable on any of these fails the page's type check.
fn notifications_declarations() -> String {
    let config = ts_rs::Config::new().with_large_int("number");
    macro_rules! decls {
        ($($t:ty),* $(,)?) => { vec![$(<$t>::decl(&config)),*] };
    }
    let decls: Vec<String> = decls![
        NotificationExtra, Notification, crate::notify::NotifySettingsPatch, crate::notify::NotificationIds,
        crate::notify::NotificationsAnswer, crate::notify::NotifySettingsAnswer, crate::notify::NotifyTestAnswer,
        crate::notify::NotificationsReadAnswer, crate::notify::NotificationsSeenAnswer, crate::notify::NotificationsClearAnswer,
    ];
    let mut out = String::from(
        "// Generated from rust/crates/store/src/feeds.rs and the server's notify module. Do not\n// edit: change the Rust type, then `BAGHOLDER_BLESS=1 cargo test -p bagholder-server the_pages_notifications_types`.\n\nimport type { NotifyStatus } from './status'\n\n",
    );
    for d in decls {
        out.push_str("export ");
        out.push_str(d.trim());
        out.push_str("\n\n");
    }
    out.trim_end().to_string() + "\n"
}

#[test]
fn test_the_pages_notifications_types_are_the_servers() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../web/src/lib/generated/notifications.ts");
    let want = notifications_declarations();
    if std::env::var("BAGHOLDER_BLESS").map_or(false, |v| v == "1") {
        std::fs::write(&path, &want).unwrap();
        return;
    }
    let have = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(have == want, "web/src/lib/generated/notifications.ts is not what the server's types generate: run with BAGHOLDER_BLESS=1 and check the page");
}

/// `{"ok": true} | {"ok": false, "error": string}`, generated to
/// `web/src/lib/generated/common.ts` -- the one shape several routes across
/// `login`, `session` and `update` answer.
fn common_declarations() -> String {
    let config = ts_rs::Config::new().with_large_int("number");
    let mut out = String::from("// Generated from the server's http module. Do not edit: change the Rust type, then\n// `BAGHOLDER_BLESS=1 cargo test -p bagholder-server the_pages_common_types`.\n\n");
    out.push_str("export ");
    out.push_str(crate::http::OkOr::decl(&config).trim());
    out.push('\n');
    out
}

#[test]
fn test_the_pages_common_types_are_the_servers() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../web/src/lib/generated/common.ts");
    let want = common_declarations();
    if std::env::var("BAGHOLDER_BLESS").map_or(false, |v| v == "1") {
        std::fs::write(&path, &want).unwrap();
        return;
    }
    let have = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(have == want, "web/src/lib/generated/common.ts is not what the server's types generate: run with BAGHOLDER_BLESS=1 and check the page");
}

/// The session and login types, generated to `web/src/lib/generated/session.ts`
/// -- a field renamed, added or made nullable on any of these fails the page's type check.
fn session_declarations() -> String {
    let config = ts_rs::Config::new().with_large_int("number");
    macro_rules! decls {
        ($($t:ty),* $(,)?) => { vec![$(<$t>::decl(&config)),*] };
    }
    let decls: Vec<String> = decls![
        bagholder_ws::session::IdentityKeys, bagholder_ws::session::Expiry, crate::session::Capture, crate::login::LoginInput,
        crate::login::StartLoginAnswer, crate::login::CancelLoginAnswer, crate::session::RefreshAnswer, crate::session::SyncAnswer,
    ];
    let mut out = String::from(
        "// Generated from rust/crates/ws/src/session.rs, session.rs and login.rs. Do not edit: change\n// the Rust type, then `BAGHOLDER_BLESS=1 cargo test -p bagholder-server the_pages_session_types`.\n\n",
    );
    for d in decls {
        out.push_str("export ");
        out.push_str(d.trim());
        out.push_str("\n\n");
    }
    out.trim_end().to_string() + "\n"
}

#[test]
fn test_the_pages_session_types_are_the_servers() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../web/src/lib/generated/session.ts");
    let want = session_declarations();
    if std::env::var("BAGHOLDER_BLESS").map_or(false, |v| v == "1") {
        std::fs::write(&path, &want).unwrap();
        return;
    }
    let have = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(have == want, "web/src/lib/generated/session.ts is not what the server's types generate: run with BAGHOLDER_BLESS=1 and check the page");
}

/// `http::model`'s own request and answer types, generated to
/// `web/src/lib/generated/model_api.ts` -- named apart from `model.ts` (the
/// page's own hand-written model types) so neither shadows the other.
fn model_api_declarations() -> String {
    let config = ts_rs::Config::new().with_large_int("number");
    macro_rules! decls {
        ($($t:ty),* $(,)?) => { vec![$(<$t>::decl(&config)),*] };
    }
    let decls: Vec<String> = decls![
        bagholder_model::input::JournalEntry, bagholder_model::input::TradeGroup,
        crate::http::model::TradeQuery, crate::http::model::Clear, crate::http::model::ClearAnswer, crate::clear::Kind,
        crate::http::model::JournalEntryRequest, crate::http::model::JournalAnswer,
        crate::http::model::FiguresQuery, crate::http::stream::Resync,
        crate::entries::EntryRequest, crate::entries::ChildShare, crate::http::model::EntryAnswer,
        crate::csv_import::ImportRequest, crate::csv_import::RowNote, crate::csv_import::ImportReport,
        crate::csv_import::WatchRequest, crate::csv_import::WatchStatus, crate::csv_import::WatchedFile, crate::csv_import::FileOutcome,
    ];
    let mut out = String::from(
        "// Generated from the server's http::model module. Do not edit: change the Rust type, then\n// `BAGHOLDER_BLESS=1 cargo test -p bagholder-server the_pages_model_api_types`.\n\nimport type { Leg, Fill, MarketDates, Position, PositionsSummary, Portfolio, Markets, Filters, Options, Kpi, EquityBlock, YearRow, BenchmarkRef, MonthlyBar, BySymbolRow, Grades, QueueRow, Trade, Cashflow, Unmatched, Account } from './wire'\nimport type { LegacyNote } from './book'\nimport type { Status } from './status'\n\n",
    );
    for d in decls {
        out.push_str("export ");
        out.push_str(d.trim());
        out.push_str("\n\n");
    }
    out.trim_end().to_string() + "\n"
}

#[test]
fn test_the_pages_model_api_types_are_the_servers() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../web/src/lib/generated/model_api.ts");
    let want = model_api_declarations();
    if std::env::var("BAGHOLDER_BLESS").map_or(false, |v| v == "1") {
        std::fs::write(&path, &want).unwrap();
        return;
    }
    let have = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(have == want, "web/src/lib/generated/model_api.ts is not what the server's types generate: run with BAGHOLDER_BLESS=1 and check the page");
}

/// Which generated file declares a route-table type by name, so the route
/// table's own generator (`routes_declarations`) can import each from where
/// it lives rather than repeating its declaration.
fn generated_file_of(name: &str) -> &'static str {
    match name {
        "NotificationIds" | "NotificationsAnswer" | "NotifySettingsAnswer" | "NotifySettingsPatch" | "NotifyTestAnswer" | "NotificationsReadAnswer" | "NotificationsSeenAnswer" | "NotificationsClearAnswer" => "notifications",
        "OkOr" => "common",
        "StartLoginAnswer" | "CancelLoginAnswer" | "LoginInput" | "Capture" | "RefreshAnswer" | "SyncAnswer" => "session",
        "OrdersDoc" | "OrderActionAnswer" | "RefreshOrdersAnswer" | "Named" | "Modify" | "Adjust" | "RefreshAndOrders" | "QuoteOf" | "TicketQuote" | "PlaceTicketAnswer" | "Ticket" | "PreviewRequest" | "Preview" => "orders",
        "LegacyNote" => "book",
        "StatusAnswer" => "status",
        "TradeQuery" | "Clear" | "ClearAnswer" | "JournalEntryRequest" | "JournalAnswer" | "EntryRequest" | "ChildShare" | "EntryAnswer" | "ImportRequest" | "ImportReport" | "WatchRequest" | "WatchStatus" => "model_api",
        "Book" => "book",
        "FilingsAnswer" | "EnrichAnswer" | "Filings" | "Scope" | "Document" | "FilingsFeed" => "filings",
        "FearAnswer" | "ShortsAnswer" | "Listing" | "Fear" | "ShortsQuery" | "GlanceAnswer" | "ShortsFeed" | "Search" | "SymbolSearchAnswer" | "ListingAnswer" | "NewsSymbolAnswer" | "WatchlistBody" | "WatchlistAnswer" | "TilesSet" | "TilesAnswer" => "markets",
        "HistoryAnswer" | "HistoryQuery" => "chart",
        "Figures" | "Detail" => "figures",
        "FiguresQuery" | "Resync" => "model_api",
        other => panic!("route table type {} has no generated file mapped in generated_file_of", other),
    }
}

/// The route table, generated to `web/src/lib/generated/routes.ts` -- a route
/// added, moved or given a different request or answer type fails the page's
/// type check on `call()`.
fn routes_declarations() -> String {
    let table = crate::http::route_table();
    let mut names: Vec<&str> = table.iter().flat_map(|e| [e.query.as_deref(), e.body.as_deref(), Some(e.answer.as_str())]).flatten().collect();
    names.sort();
    names.dedup();
    let mut by_file: std::collections::BTreeMap<&str, Vec<&str>> = std::collections::BTreeMap::new();
    for n in names {
        by_file.entry(generated_file_of(n)).or_default().push(n);
    }
    let mut out = String::from("// Generated from the server's route table (`api_routes!`). Do not edit: change the\n// route's declaration, then `BAGHOLDER_BLESS=1 cargo test -p bagholder-server the_pages_routes_are_the_servers`.\n\n");
    for (file, names) in by_file {
        out.push_str(&format!("import type {{ {} }} from './{}'\n", names.join(", "), file));
    }
    out.push_str("\nexport interface Routes {\n");
    for e in table {
        let key = format!("{} {}", e.method.to_uppercase(), e.path);
        let mut fields = Vec::new();
        if let Some(q) = &e.query {
            fields.push(format!("query: {}", q));
        }
        if let Some(b) = &e.body {
            fields.push(format!("body: {}", b));
        }
        fields.push(format!("answer: {}", e.answer));
        out.push_str(&format!("  '{}': {{ {} }}\n", key, fields.join("; ")));
    }
    out.push_str("}\n");
    out
}

#[test]
fn test_the_pages_routes_are_the_servers() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../web/src/lib/generated/routes.ts");
    let want = routes_declarations();
    if std::env::var("BAGHOLDER_BLESS").map_or(false, |v| v == "1") {
        std::fs::write(&path, &want).unwrap();
        return;
    }
    let have = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(have == want, "web/src/lib/generated/routes.ts is not what the server's route table generates: run with BAGHOLDER_BLESS=1 and check the page");
}

/// The figures document (`wire`), generated to `web/src/lib/generated/figures.ts`.
/// Money, quantities and prices are `Dec`, exact decimal text the page formats
/// without a float (`web/src/lib/dec.ts`, which also holds `Fig`'s helpers).
fn figures_declarations() -> String {
    use crate::wire::figures::*;
    let config = ts_rs::Config::new().with_large_int("number");
    macro_rules! decls {
        ($($t:ty),* $(,)?) => { vec![$(<$t>::decl(&config)),*] };
    }
    let decls: Vec<String> = decls![
        crate::wire::Fig<()>,
        Partial, Status, Trade, Position, Fill, Detail, Kpi, Point, Drawdown, Annualized, PnlCurve, Equity, YearRow, BenchmarkRef, MonthlyBar, BySymbolRow, GradeBucket, Grades, QueueRow,
        Slice, Portfolio, Account, CashflowTile, CashflowMonth, CashflowHolding, CashflowRow, Cashflow, Waiting, AccountOption, InstrumentOption, Options, Figures,
        crate::wire::filters::Range, crate::wire::filters::Filters,
    ];
    let mut out = String::from("// Generated from rust/crates/server/src/wire. Do not edit: change the Rust type, then\n// `BAGHOLDER_BLESS=1 cargo test -p bagholder-server the_pages_figures_types`.\n\nimport type { Dec } from '../dec'\nimport type { Markets, ExposureSlice } from './wire'\n\n");
    for d in decls {
        out.push_str("export ");
        out.push_str(d.trim());
        out.push_str("\n\n");
    }
    out.trim_end().to_string() + "\n"
}

#[test]
fn test_the_pages_figures_types_are_the_servers() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../web/src/lib/generated/figures.ts");
    let want = figures_declarations();
    if std::env::var("BAGHOLDER_BLESS").map_or(false, |v| v == "1") {
        std::fs::write(&path, &want).unwrap();
        return;
    }
    let have = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(have == want, "web/src/lib/generated/figures.ts is not what the server's types generate: run with BAGHOLDER_BLESS=1 and check the page");
}

/// No amount on the figures wire is a float: a field typed `number` is a ratio,
/// a count, a number of days or a year, and is named here; anything else that is
/// a number fails (`docs/plans/stage-3c-switch.md`, §4, "Decimals as text").
#[test]
fn test_no_amount_on_the_figures_wire_is_a_number() {
    const NUMBERS: [&str; 19] = [
        "leftOut", "realizedLeftOut", "count", "wins", "losses", "breakeven", "winRate", "profitFactor", "pct", "rate", "r", "spR", "n", "graded", "share", "positionCount", "navAccounts", "activityCount", "holdDays",
    ];
    const MORE: [&str; 8] = ["pnlPct", "unrealPct", "percentChange", "held", "avgHold", "unrealizedPct", "marginUsedPct", "cashPct"];
    const MORE2: [&str; 5] = ["dayChangePct", "yield", "yoc", "currentYield", "vs"];
    // the declarations without their doc comments
    let text = regex::Regex::new(r"(?s)/\*\*.*?\*/").unwrap().replace_all(&figures_declarations(), "").to_string();
    let field = regex::Regex::new(r"(\w+)\??: ([^,;}]*)").unwrap();
    let mut numbers: Vec<String> = field.captures_iter(&text).filter(|c| c[2].contains("number")).map(|c| c[1].to_string()).collect();
    numbers.sort();
    numbers.dedup();
    let allowed: Vec<&str> = NUMBERS.iter().chain(MORE.iter()).chain(MORE2.iter()).copied().collect();
    let stray: Vec<&String> = numbers.iter().filter(|n| !allowed.contains(&n.as_str())).collect();
    assert!(stray.is_empty(), "a number on the figures wire that is not a ratio, count or day: {stray:?}");
    // the scan finds what it looks for
    assert!(field.captures_iter("{ total: number, ").any(|c| &c[1] == "total" && c[2].contains("number")));
}

/// Each document's lists of rows and the field each is told apart by, as the
/// server's differ keys them, generated to `web/src/lib/generated/keys.ts`: what the
/// page reconciles a whole state by when one arrives again, never a key guessed
/// from the data.
fn keys_declarations() -> String {
    use bagholder_model::patch::keys_of;
    let docs: Vec<(&str, Vec<(String, &'static str)>)> = vec![
        ("model", {
            let mut k = keys_of::<crate::wire::figures::Figures>();
            k.extend(keys_of::<crate::status::Status>().into_iter().map(|(p, f)| (format!("status.{p}"), f)));
            k
        }),
        ("orders", keys_of::<crate::orders::OrdersDoc>()),
        ("shorts", keys_of::<crate::feeds::ShortsFeed>()),
        ("notifications", keys_of::<crate::notify::NotificationsDoc>()),
        ("filings", keys_of::<crate::docs::FilingsAnswer>()),
        ("filings-feed", keys_of::<crate::feeds::FilingsFeed>()),
        ("fear", keys_of::<crate::feeds::FearDoc>()),
        ("quote", keys_of::<crate::orders::TicketQuote>()),
        ("history", keys_of::<crate::docs::HistoryPending>()),
        ("universe", keys_of::<crate::feeds::UniverseDoc>()),
        ("news", keys_of::<crate::feeds::NewsDoc>()),
    ];
    let mut out = String::from("// Generated from the server's differ (`bagholder_model::patch::keys_of`). Do not edit:\n// change the Rust type, then `BAGHOLDER_BLESS=1 cargo test -p bagholder-server the_pages_row_keys`.\n\n/** Each document's lists of rows, by path (`*` for a list's rows or a map's values), and the field that tells the rows apart. */\nexport const ROW_KEYS: Record<string, Record<string, string>> = {\n");
    for (doc, keys) in docs {
        out.push_str(&format!("  '{doc}': {{\n"));
        for (path, field) in keys {
            out.push_str(&format!("    '{path}': '{field}',\n"));
        }
        out.push_str("  },\n");
    }
    out.push_str("}\n");
    out
}

#[test]
fn test_the_pages_row_keys_are_the_servers() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../web/src/lib/generated/keys.ts");
    let want = keys_declarations();
    if std::env::var("BAGHOLDER_BLESS").map_or(false, |v| v == "1") {
        std::fs::write(&path, &want).unwrap();
        return;
    }
    let have = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(have == want, "web/src/lib/generated/keys.ts is not what the server's differ keys: run with BAGHOLDER_BLESS=1 and check the page");
    // the figures' own lists are all there, each by its id
    assert!(want.contains("'trades': 'id'") && want.contains("'positions': 'id'") && want.contains("'cashflow.tiles': 'label'"), "{want}");
}

/// Every word the engine can say a figure waits on (`Gap::word`), generated to
/// `web/src/lib/generated/gaps.ts`: the page has a word for each (its own test).
fn gap_words() -> Vec<String> {
    let source = include_str!("../../engine/src/gap.rs");
    let body = &source[source.find("pub fn word(&self)").expect("Gap::word")..];
    let body = &body[..body.find("\n    }\n").expect("its end")];
    regex::Regex::new(r#"=>\s*"([a-z-]+)""#).unwrap().captures_iter(body).map(|c| c[1].to_string()).collect()
}

#[test]
fn test_the_pages_gap_words_are_the_engines() {
    let words = gap_words();
    assert!(words.len() > 20, "the scan finds the engine's list");
    let want = format!(
        "// Generated from the engine's gap words (`Gap::word`, rust/crates/engine/src/gap.rs). Do not edit:\n// `BAGHOLDER_BLESS=1 cargo test -p bagholder-server the_pages_gap_words`.\n\nexport const GAP_WORDS: string[] = [\n{}]\n",
        words.iter().map(|w| format!("  '{w}',\n")).collect::<String>()
    );
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../web/src/lib/generated/gaps.ts");
    if std::env::var("BAGHOLDER_BLESS").map_or(false, |v| v == "1") {
        std::fs::write(&path, &want).unwrap();
        return;
    }
    assert!(std::fs::read_to_string(&path).unwrap_or_default() == want, "web/src/lib/generated/gaps.ts is not the engine's gap words: run with BAGHOLDER_BLESS=1");
}
