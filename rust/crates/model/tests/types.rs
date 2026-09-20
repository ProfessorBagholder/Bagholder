//! The page's types are generated from the model's (`web/src/lib/generated/wire.ts`),
//! so the two sides of the wire cannot drift: a field renamed, added or made
//! nullable here fails the page's type check there.
//!
//! After an intended change: `BAGHOLDER_BLESS=1 cargo test -p bagholder-model
//! --test types`, and the page's `npm run check` says what it breaks.

use std::path::PathBuf;
use ts_rs::TS;

use bagholder_model::activity::{Direction, Kind, Side};
use bagholder_model::fifo::Unmatched;
use bagholder_model::filters::{Filters, Lists, Op, Range, Ranges};
use bagholder_model::nav::Point;
use bagholder_model::wire::*;

fn declarations() -> String {
    // every integer the model sends fits a JavaScript number
    let config = ts_rs::Config::new().with_large_int("number");
    macro_rules! decls {
        ($($t:ty),* $(,)?) => { vec![$(<$t>::decl(&config)),*] };
    }
    let decls: Vec<String> = decls![
        Kind, Side, Direction, ExitSide, Mark, Payment, RateSource, Priced, Op,
        Range, Lists, Ranges, Filters,
        Leg, Fill, Tally, Trade, OpenLot, Position, TradeDetail,
        Kpi, BySymbolRow, MonthlyBar, GradeBucket, Grades, QueueRow,
        Point, YearRow, Annualized, Drawdown, EquityBlock, BenchmarkRef,
        Allocation, ExposureSlice, Portfolio, PositionsSummary, Account,
        CashflowRow, CashflowTile, CashflowMonth, CashflowHolding, Cashflow,
        HeldTile, UniverseTile, WatchItem, NewsTag, NewsItem, MarketTile, MarketInstrument, Markets,
        ListingInfo, Options, MarketDates, Unmatched, View,
    ];
    let mut out = String::from("// Generated from rust/crates/model (wire.rs and the types it names). Do not edit:\n// change the Rust type, then `BAGHOLDER_BLESS=1 cargo test -p bagholder-model --test types`.\n\n");
    for d in decls {
        out.push_str("export ");
        out.push_str(d.trim());
        out.push_str("\n\n");
    }
    out.trim_end().to_string() + "\n"
}

#[test]
fn test_the_pages_types_are_the_models() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../web/src/lib/generated/wire.ts");
    let want = declarations();
    if std::env::var("BAGHOLDER_BLESS").map_or(false, |v| v == "1") {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &want).unwrap();
        return;
    }
    let have = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(have == want, "web/src/lib/generated/wire.ts is not what the model's types generate: run with BAGHOLDER_BLESS=1 and check the page");
}
