//! The page's order types are generated from the server's (`web/src/lib/generated/orders.ts`),
//! as the model's are from the model's: a field renamed, added or made nullable here
//! fails the page's type check there.
//!
//! After an intended change: `BAGHOLDER_BLESS=1 cargo test -p bagholder-server the_pages_order_types`,
//! and the page's `npm run check` says what it breaks.

use std::path::PathBuf;
use ts_rs::TS;

use bagholder_store::bars::{ChartBars, DayBar, TimeBar};
use bagholder_store::orders::{Bracket, BracketStatus, Order, OrderStatus, OrderType, Role, Side, SlKind, SlMode, Source, StopLoss, TakeProfit, TrailUnit};

use crate::feeds::ChartHistory;
use crate::orders::{OrderCard, OrdersDoc};

fn declarations() -> String {
    let config = ts_rs::Config::new().with_large_int("number");
    macro_rules! decls {
        ($($t:ty),* $(,)?) => { vec![$(<$t>::decl(&config)),*] };
    }
    let decls: Vec<String> = decls![Side, OrderType, OrderStatus, Role, Source, BracketStatus, SlKind, TrailUnit, SlMode, StopLoss, TakeProfit, Order, OrderCard, Bracket, OrdersDoc];
    let mut out = String::from("// Generated from rust/crates/store/src/orders/types.rs and the server's orders document. Do not\n// edit: change the Rust type, then `BAGHOLDER_BLESS=1 cargo test -p bagholder-server the_pages_order_types`.\n\n");
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
    let decls: Vec<String> = decls![DayBar, TimeBar, ChartBars, ChartHistory];
    let mut out = String::from("// Generated from rust/crates/store/src/bars.rs and the server's chart history. Do not\n// edit: change the Rust type, then `BAGHOLDER_BLESS=1 cargo test -p bagholder-server the_pages_chart_types`.\n\n");
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
