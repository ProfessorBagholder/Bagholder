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

use crate::app::{self, truthy, App};
use crate::{feeds, session};

pub fn routes() -> Routed {
    let mut routed = api_routes! {
        get "/api/status" => status, answer: "StatusAnswer";
        get "/api/trade" => trade, query: "TradeQuery", answer: "TradeAnswer";
        get "/api/data" => data, answer: "DataSummary";
        post "/api/data/clear" => data_clear, body: "Clear", answer: "DataSummary";
        post "/api/journal" => journal, body: "JournalEntryRequest", answer: "JournalAnswer";
        post "/api/groups" => groups, body: "Groups", answer: "GroupsAnswer";
        post "/api/notes" => notes, body: "Notes", answer: "NotesAnswer";
        post "/api/import" => import, body: "Import", answer: "ImportReport";
        post "/api/watch/clear" => watch_clear, answer: "WatchStatus";
    };
    // not yet in the table: `/api/model` (stage 5d7d's Filters typing, §3, is
    // unfinished), `/api/book` (`Book` reaches into `bagholder_store::broker::Account`
    // and `bagholder_model::securities::Security`, neither typed for the page yet),
    // and `/api/watch`'s GET (shares its path with the POST below) and
    // `/api/watch/scan`, which answer with their own status code
    routed.router = routed
        .router
        .route("/api/model", get(model))
        .route("/api/book", get(book))
        .route("/api/watch", get(watch_status).post(watch_set))
        .route("/api/watch/scan", axum::routing::post(watch_scan));
    routed
}

async fn status(State(state): State<AppState>) -> Api<crate::status::StatusAnswer> {
    super::answer(move || crate::status::answer(&state.app)).await
}

#[derive(Deserialize)]
struct ModelQuery {
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

/// `GET /api/model`: the whole view for a set of filters. The Svelte page gets its
/// view over `/api/events`; this is what the legacy page and the phones read.
async fn model(State(state): State<AppState>, Params(q): Params<ModelQuery>) -> super::Api<Value> {
    let app = state.app;
    let built = blocking(move || -> Result<Value, ApiError> {
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
        let filters = q.filters.and_then(|raw| serde_json::from_str::<Value>(&raw).ok());
        let view = app.view(filters.as_ref(), q.trade.as_deref()).map_err(|e| ApiError::Model(e.to_string()))?;
        let mut payload = if q.only.as_deref() == Some("live") {
            // only these sections are written out of the shared view, not the whole of it
            let mut live = serde_json::json!({
                "ok": view.ok, "today": view.today, "currency": view.currency, "market": view.market,
                "positions": view.positions, "positionsSummary": view.positions_summary, "portfolio": view.portfolio,
            });
            if q.markets.is_some() {
                live["markets"] = serde_json::to_value(&view.markets).map_err(|e| ApiError::Model(e.to_string()))?;
            }
            live
        } else {
            view.to_value()
        };
        payload["status"] = serde_json::to_value(crate::status::status(&app)).unwrap_or(Value::Null);
        Ok(payload)
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
    journal: std::collections::HashMap<String, bagholder_model::input::JournalEntry>,
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

fn to_value<T: serde::Serialize>(v: &T) -> Value {
    serde_json::to_value(v).unwrap_or(Value::Null)
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

#[derive(Deserialize, Default)]
struct WatchFolder {
    #[serde(default)]
    path: String,
}

async fn watch_set(State(state): State<AppState>, Body(w): Body<WatchFolder>) -> Result<(axum::http::StatusCode, Json<Value>), ApiError> {
    let app = state.app;
    let out = blocking(move || -> rusqlite::Result<(bool, Value)> {
        let conn = app.open()?;
        let set = bagholder_store::csvimport::set_watch_folder(&conn, &w.path)?;
        if !set.ok {
            return Ok((false, to_value(&set)));
        }
        let result = bagholder_store::csvimport::scan_folder(&conn, None, true)?;
        let mut result = to_value(&result);
        result["status"] = to_value(&bagholder_store::csvimport::status(&conn)?);
        Ok((true, result))
    })
    .await??;
    Ok((if out.0 { axum::http::StatusCode::OK } else { axum::http::StatusCode::BAD_REQUEST }, Json(out.1)))
}

async fn watch_scan(State(state): State<AppState>) -> Result<(axum::http::StatusCode, Json<Value>), ApiError> {
    let app = state.app;
    let result = blocking(move || -> rusqlite::Result<Value> {
        let conn = app.open()?;
        let result = bagholder_store::csvimport::scan_folder(&conn, None, true)?;
        let mut result = to_value(&result);
        result["status"] = to_value(&bagholder_store::csvimport::status(&conn)?);
        Ok(result)
    })
    .await??;
    Ok((if truthy(result.get("ok")) { axum::http::StatusCode::OK } else { axum::http::StatusCode::BAD_REQUEST }, Json(result)))
}

async fn watch_clear(State(state): State<AppState>) -> Api<bagholder_store::csvimport::WatchStatus> {
    with_store(&state, |conn| {
        bagholder_store::csvimport::clear_watch_folder(conn)?;
        bagholder_store::csvimport::status(conn)
    })
    .await
}
