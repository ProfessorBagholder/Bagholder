//! The book and what is derived from it: the model, one trade, the header's
//! status, the journal, imports, and the Data & storage dialog.

use axum::extract::State;
use axum::routing::get;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use super::extract::trimmed;
use super::{api_routes, blocking, with_store, Api, ApiError, AppState, Body, Params, Routed};
use std::sync::Arc;

use crate::app::{self, App};
use crate::{feeds, session};

pub fn routes() -> Routed {
    let mut routed = api_routes! {
        get "/api/status" => status;
        get "/api/model" => model;
        get "/api/trade" => trade;
        get "/api/book" => book;
        get "/api/data" => data;
        post "/api/data/clear" => data_clear;
        post "/api/journal" => journal;
        post "/api/groups" => groups;
        post "/api/notes" => notes;
        post "/api/import" => import;
        post "/api/watch/clear" => watch_clear;
    };
    // `/api/watch`'s GET and POST share one path, which `api_routes!` cannot
    // declare twice; their entries are read from the handlers all the same
    routed.router = routed
        .router
        .route("/api/watch", get(watch_status).post(watch_set))
        .route("/api/watch/scan", axum::routing::post(watch_scan));
    routed.table.push(super::RouteEntry::of(watch_status, "get", "/api/watch"));
    routed.table.push(super::RouteEntry::of(watch_set, "post", "/api/watch"));
    routed.table.push(super::RouteEntry::of(watch_scan, "post", "/api/watch/scan"));
    routed
}

async fn status(State(state): State<AppState>) -> Api<crate::status::StatusAnswer> {
    super::answer(move || crate::status::answer(&state.app)).await
}

#[derive(Deserialize, TS)]
pub struct ModelQuery {
    /// the page's filters, as the JSON it keeps them in
    #[serde(default, deserialize_with = "trimmed")]
    filters: Option<String>,
    /// the trade whose detail to carry
    #[serde(default, deserialize_with = "trimmed")]
    trade: Option<String>,
    /// `live`: only what a price moves (the legacy page's quote tick)
    #[serde(default, deserialize_with = "trimmed")]
    only: Option<String>,
    /// with `only=live`: the heatmap is on screen and wants its tiles too
    #[serde(default, deserialize_with = "trimmed")]
    markets: Option<String>,
}

/// `GET /api/model`, `only=live`: only the sections that move on a price
/// tick, not the whole view.
#[derive(Serialize, TS)]
pub struct ModelLiveAnswer {
    ok: bool,
    today: String,
    currency: &'static str,
    market: bagholder_model::wire::MarketDates,
    positions: Vec<bagholder_model::wire::Position>,
    #[serde(rename = "positionsSummary")]
    positions_summary: bagholder_model::wire::PositionsSummary,
    portfolio: bagholder_model::wire::Portfolio,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    markets: Option<bagholder_model::wire::Markets>,
    status: crate::status::Status,
}

/// `GET /api/model`: the whole view, with the header's status riding beside it.
#[derive(Serialize, TS)]
pub struct ModelAnswer {
    #[serde(flatten)]
    #[ts(flatten)]
    view: bagholder_model::wire::View,
    status: crate::status::Status,
}

/// `GET /api/model`'s answer: the slim `only=live` shape, or the whole view.
#[derive(Serialize, TS)]
#[serde(untagged)]
pub enum ModelViewAnswer {
    Live(ModelLiveAnswer),
    Full(Box<ModelAnswer>),
}

/// `GET /api/model`: the whole view for a set of filters. The Svelte page gets its
/// view over `/api/events`; this is what the legacy page and the phones read.
async fn model(State(state): State<AppState>, Params(q): Params<ModelQuery>) -> Api<ModelViewAnswer> {
    let app = state.app;
    let built = blocking(move || -> Result<ModelViewAnswer, ApiError> {
        if let (Ok(conn), Ok(base)) = (app.open(), app.base()) {
            let (today, now, _) = bagholder_market::clock_now();
            if bagholder_market::refresh::is_stale(&conn, &today, &bagholder_model::symbols_of::payer_symbols(&base)) {
                let a = app.clone();
                app.kick("market", move || {
                    feeds::refresh_market_data(&a);
                });
            } else {
                let mut syms = bagholder_model::symbols_of::held_symbols(&base);
                syms.extend(bagholder_model::markets::quote_symbols(&base));
                let due = bagholder_market::quotes::quote_symbols_needing_refresh(&conn, &syms, now, bagholder_market::quotes::QUOTE_REFRESH_MINUTES).map(|v| !v.is_empty()).unwrap_or(false);
                if due {
                    let a = app.clone();
                    app.kick("quotes", move || {
                        feeds::refresh_quotes(&a);
                    });
                }
            }
        }
        // the page's filters, parsed once, the way the event stream's `Feed` does
        let raw = q.filters.and_then(|raw| serde_json::from_str::<Value>(&raw).ok());
        let filters = raw.map(|v| bagholder_model::filters::clean_filters(Some(&v)));
        let view = app.view(filters.as_ref(), q.trade.as_deref()).map_err(|e| ApiError::Model(e.to_string()))?;
        let status = crate::status::status(&app);
        let answer = if q.only.as_deref() == Some("live") {
            ModelViewAnswer::Live(ModelLiveAnswer {
                ok: view.ok,
                today: view.today.clone(),
                currency: view.currency,
                market: view.market.clone(),
                positions: view.positions.clone(),
                positions_summary: view.positions_summary.clone(),
                portfolio: view.portfolio.clone(),
                markets: q.markets.is_some().then(|| view.markets.clone()),
                status,
            })
        } else {
            ModelViewAnswer::Full(Box::new(ModelAnswer { view: (*view).clone(), status }))
        };
        Ok(answer)
    })
    .await??;
    Ok(Json(built))
}

#[derive(Deserialize, TS)]
pub struct TradeQuery {
    #[serde(default, deserialize_with = "trimmed")]
    id: Option<String>,
}

/// `GET /api/trade`.
#[derive(Serialize, TS)]
pub struct TradeAnswer {
    ok: bool,
    id: String,
    legs: Vec<bagholder_model::wire::Leg>,
    fills: Vec<bagholder_model::wire::Fill>,
}

/// `GET /api/trade`: the legs and fills of one trade or holding, fetched when its page opens.
async fn trade(State(state): State<AppState>, Params(q): Params<TradeQuery>) -> Api<TradeAnswer> {
    let app = state.app;
    let id = q.id.unwrap_or_default();
    let found = blocking(move || app.base().map(|base| bagholder_model::view::trade_detail(&base, &id))).await?.map_err(|e| ApiError::Model(e.to_string()))?;
    let detail = found.ok_or_else(|| ApiError::NotFound("no such trade".into()))?;
    Ok(Json(TradeAnswer { ok: true, id: detail.id, legs: detail.legs.to_vec(), fills: detail.fills.to_vec() }))
}

/// `GET /api/book`: the stored rows as they are, for the phones and for export.
async fn book(State(state): State<AppState>) -> Api<bagholder_store::book::Book> {
    with_store(&state, |conn| bagholder_store::book::book(conn)).await
}

/// The row counts the Data & storage dialog shows before a wipe.
#[derive(Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DataSummary {
    ok: bool,
    path: String,
    activities: i64,
    first_activity: String,
    last_activity: String,
    accounts: i64,
    balances: i64,
    nav_days: i64,
    securities: i64,
    journal: usize,
    fx_days: i64,
    benchmark_days: i64,
    filings: i64,
    synced_at: String,
    session_present: bool,
}

fn data_summary(app: &Arc<App>, conn: &rusqlite::Connection) -> rusqlite::Result<DataSummary> {
    let count = |sql: &str| -> rusqlite::Result<i64> { conn.query_row(sql, [], |r| r.get(0)) };
    let journal = bagholder_store::admin::journal(conn)?;
    let (first_act, last_act): (Option<String>, Option<String>) =
        conn.query_row("SELECT MIN(transaction_date), MAX(transaction_date) FROM activities", [], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(DataSummary {
        ok: true,
        path: app.db_path().display().to_string(),
        activities: count("SELECT COUNT(*) FROM activities")?,
        first_activity: first_act.unwrap_or_default(),
        last_activity: last_act.unwrap_or_default(),
        accounts: count("SELECT COUNT(*) FROM accounts")?,
        balances: count("SELECT COUNT(*) FROM balances")?,
        nav_days: count("SELECT COUNT(*) FROM nav_history")?,
        securities: count("SELECT COUNT(*) FROM securities")?,
        journal: journal.len(),
        fx_days: count("SELECT COUNT(*) FROM fx_rates")?,
        benchmark_days: count("SELECT COUNT(*) FROM benchmark_prices")?,
        filings: count("SELECT COUNT(*) FROM filings")?,
        synced_at: bagholder_store::tables::get_meta(conn, "synced_at", "")?,
        session_present: session::load_session(app).is_some(),
    })
}

async fn data(State(state): State<AppState>) -> Api<DataSummary> {
    let app = state.app.clone();
    with_store(&state, move |conn| data_summary(&app, conn)).await
}

/// What `POST /api/data/clear` removes besides the synced rows.
#[derive(Deserialize, Default, TS)]
pub struct Clear {
    #[serde(default)]
    #[ts(optional)]
    journal: Option<bool>,
    #[serde(default)]
    #[ts(optional)]
    market: Option<bool>,
    #[serde(default)]
    #[ts(optional)]
    session: Option<bool>,
}

async fn data_clear(State(state): State<AppState>, Body(what): Body<Clear>) -> Api<DataSummary> {
    if state.app.state.lock().unwrap().syncing {
        return Err(ApiError::Conflict("A sync is running. Wait for it to finish.".into()));
    }
    let app = state.app.clone();
    with_store(&state, move |conn| {
        bagholder_store::admin::clear_synced_data(conn, !what.journal.unwrap_or(false), !what.market.unwrap_or(false))?;
        if what.session.unwrap_or(false) {
            session::delete_session(&app);
        }
        {
            let mut st = app.state.lock().unwrap();
            st.last_sync.clear();
            st.error.clear();
        }
        data_summary(&app, conn)
    })
    .await
}

/// One trade's journal entry. A field left out is cleared, as the page sends all three.
#[derive(Deserialize, Default, TS)]
#[serde(default)]
pub struct JournalEntryRequest {
    #[serde(deserialize_with = "trimmed")]
    id: Option<String>,
    #[serde(flatten)]
    #[ts(flatten)]
    entry: bagholder_model::input::JournalEntry,
}

/// `POST /api/journal`.
#[derive(Serialize, TS)]
pub struct JournalAnswer {
    ok: bool,
    journal: bagholder_model::input::Journal,
}

async fn journal(State(state): State<AppState>, Body(e): Body<JournalEntryRequest>) -> Api<JournalAnswer> {
    let id = e.id.unwrap_or_default();
    if id.trim().is_empty() {
        return Err(ApiError::BadRequest("id required".into()));
    }
    with_store(&state, move |conn| Ok(JournalAnswer { ok: true, journal: bagholder_store::admin::save_journal_entry(conn, &id, Some(&e.entry))? })).await
}

#[derive(Deserialize, Default, TS)]
#[serde(default)]
pub struct Groups {
    #[serde(deserialize_with = "bagholder_model::lenient::list")]
    groups: Vec<bagholder_model::input::TradeGroup>,
}

/// `POST /api/groups`.
#[derive(Serialize, TS)]
pub struct GroupsAnswer {
    ok: bool,
    groups: Vec<bagholder_model::input::TradeGroup>,
}

async fn groups(State(state): State<AppState>, Body(g): Body<Groups>) -> Api<GroupsAnswer> {
    with_store(&state, move |conn| Ok(GroupsAnswer { ok: true, groups: bagholder_store::tables::save_trade_groups(conn, &g.groups)? })).await
}

#[derive(Deserialize, Default, TS)]
#[serde(default)]
pub struct Notes {
    #[serde(deserialize_with = "bagholder_model::lenient::map")]
    notes: std::collections::BTreeMap<String, bagholder_store::tables::LegacyNote>,
}

/// `POST /api/notes`.
#[derive(Serialize, TS)]
pub struct NotesAnswer {
    ok: bool,
    notes: bagholder_store::tables::LegacyNotes,
}

async fn notes(State(state): State<AppState>, Body(n): Body<Notes>) -> Api<NotesAnswer> {
    with_store(&state, move |conn| Ok(NotesAnswer { ok: true, notes: bagholder_store::tables::save_trade_notes(conn, &n.notes)? })).await
}

/// A CSV the page read from a file the person chose.
#[derive(Deserialize, Default, TS)]
pub struct Import {
    #[serde(default)]
    text: String,
    #[serde(default)]
    name: String,
}

async fn import(State(state): State<AppState>, Body(i): Body<Import>) -> Api<bagholder_store::csvimport::ImportReport> {
    if bagholder_model::textrules::trim_space(&i.text).is_empty() {
        return Err(ApiError::BadRequest("text required".into()));
    }
    let name = if i.name.is_empty() { "upload.csv".to_string() } else { i.name };
    let app = state.app;
    let report = blocking(move || -> Result<bagholder_store::csvimport::ImportReport, ApiError> {
        let conn = app.open()?;
        bagholder_store::csvimport::import_text(&conn, &name, &i.text).map_err(|e| {
            app::log(&format!("bagholder: import failed: {}", e));
            ApiError::Failed(e.to_string())
        })
    })
    .await??;
    Ok(Json(report))
}

async fn watch_status(State(state): State<AppState>) -> Api<bagholder_store::csvimport::WatchStatus> {
    with_store(&state, |conn| bagholder_store::csvimport::status(conn)).await
}

#[derive(Deserialize, Default, TS)]
pub struct WatchFolder {
    #[serde(default)]
    path: String,
}

/// A folder scanned, with the watch status it left: `POST /api/watch` (once
/// the path is accepted) and `POST /api/watch/scan` answer this shape.
#[derive(Serialize, TS)]
pub struct ScanWithStatus {
    #[serde(flatten)]
    #[ts(flatten)]
    report: bagholder_store::csvimport::ScanReport,
    status: bagholder_store::csvimport::WatchStatus,
}

/// `POST /api/watch`: the path refused (`WatchSet`, with `ok: false`), or
/// accepted and scanned at once.
#[derive(Serialize, TS)]
#[serde(untagged)]
pub enum WatchSetAnswer {
    Refused(bagholder_store::csvimport::WatchSet),
    Scanned(ScanWithStatus),
}

async fn watch_set(State(state): State<AppState>, Body(w): Body<WatchFolder>) -> Result<(axum::http::StatusCode, Json<WatchSetAnswer>), ApiError> {
    let app = state.app;
    let out = blocking(move || -> rusqlite::Result<(bool, WatchSetAnswer)> {
        let conn = app.open()?;
        let set = bagholder_store::csvimport::set_watch_folder(&conn, &w.path)?;
        if !set.ok {
            return Ok((false, WatchSetAnswer::Refused(set)));
        }
        let report = bagholder_store::csvimport::scan_folder(&conn, None, true)?;
        let status = bagholder_store::csvimport::status(&conn)?;
        Ok((true, WatchSetAnswer::Scanned(ScanWithStatus { report, status })))
    })
    .await??;
    Ok((if out.0 { axum::http::StatusCode::OK } else { axum::http::StatusCode::BAD_REQUEST }, Json(out.1)))
}

async fn watch_scan(State(state): State<AppState>) -> Result<(axum::http::StatusCode, Json<ScanWithStatus>), ApiError> {
    let app = state.app;
    let out = blocking(move || -> rusqlite::Result<ScanWithStatus> {
        let conn = app.open()?;
        let report = bagholder_store::csvimport::scan_folder(&conn, None, true)?;
        let status = bagholder_store::csvimport::status(&conn)?;
        Ok(ScanWithStatus { report, status })
    })
    .await??;
    Ok((if out.report.ok { axum::http::StatusCode::OK } else { axum::http::StatusCode::BAD_REQUEST }, Json(out)))
}

async fn watch_clear(State(state): State<AppState>) -> Api<bagholder_store::csvimport::WatchStatus> {
    with_store(&state, |conn| {
        bagholder_store::csvimport::clear_watch_folder(conn)?;
        bagholder_store::csvimport::status(conn)
    })
    .await
}
