//! The book and what is derived from it: the model, one trade, the header's
//! status, the journal, imports, and the Data & storage dialog.

use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

use super::extract::trimmed;
use super::{blocking, with_store, Api, ApiError, AppState, Body, Params};
use crate::app::{self, app, truthy};
use crate::{feeds, session};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/status", get(status))
        .route("/api/model", get(model))
        .route("/api/trade", get(trade))
        .route("/api/book", get(book))
        .route("/api/data", get(data))
        .route("/api/data/clear", post(data_clear))
        .route("/api/journal", post(journal))
        .route("/api/groups", post(groups))
        .route("/api/notes", post(notes))
        .route("/api/import", post(import))
        .route("/api/watch", get(watch_status).post(watch_set))
        .route("/api/watch/scan", post(watch_scan))
        .route("/api/watch/clear", post(watch_clear))
}

async fn status() -> Api {
    super::answer(crate::status::payload).await
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
async fn model(State(state): State<AppState>, Params(q): Params<ModelQuery>) -> Api {
    let app = state.app;
    let built = blocking(move || -> Result<Value, ApiError> {
        if let (Ok(conn), Ok(base)) = (app.open(), app.base()) {
            let (today, now, _) = bagholder_market::clock_now();
            if bagholder_market::refresh::is_stale(&conn, &today, &bagholder_model::input::listings_json(&bagholder_model::symbols_of::payer_symbols(&base))) {
                app.kick("market", || {
                    feeds::refresh_market_data();
                });
            } else {
                let mut syms = bagholder_model::input::listings_json(&bagholder_model::symbols_of::held_symbols(&base));
                syms.extend(bagholder_model::input::listings_json(&bagholder_model::markets::quote_symbols(&base)));
                let due = bagholder_market::quotes::quote_symbols_needing_refresh(&conn, &syms, now, bagholder_market::quotes::QUOTE_REFRESH_MINUTES).map(|v| !v.is_empty()).unwrap_or(false);
                if due {
                    app.kick("quotes", || {
                        feeds::refresh_quotes();
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
        payload["status"] = crate::status::payload();
        Ok(payload)
    })
    .await??;
    Ok(Json(built))
}

#[derive(Deserialize)]
struct TradeQuery {
    #[serde(default, deserialize_with = "trimmed")]
    id: Option<String>,
}

/// `GET /api/trade`: the legs and fills of one trade or holding, fetched when its page opens.
async fn trade(State(state): State<AppState>, Params(q): Params<TradeQuery>) -> Api {
    let app = state.app;
    let id = q.id.unwrap_or_default();
    let found = blocking(move || app.base().map(|base| bagholder_model::view::trade_detail(&base, &id))).await?.map_err(|e| ApiError::Model(e.to_string()))?;
    let detail = found.ok_or_else(|| ApiError::NotFound("no such trade".into()))?;
    Ok(Json(json!({"ok": true, "id": detail.id, "legs": detail.legs, "fills": detail.fills})))
}

/// `GET /api/book`: the stored rows as they are, for the phones and for export.
async fn book(State(state): State<AppState>) -> Api {
    with_store(&state, |conn| {
        let book = bagholder_store::snapshot::snapshot(conn, true)?;
        let or = |k: &str, d: Value| book.get(k).filter(|v| truthy(Some(v))).cloned().unwrap_or(d);
        Ok(json!({
            "ok": true,
            "activities": or("activities", json!([])),
            "accounts": or("accounts", json!([])),
            "balances": or("balances", json!([])),
            "navHistory": or("navHistory", json!([])),
            "navByAccount": or("navByAccount", json!({})),
            "syncedAt": or("syncedAt", json!("")),
            "tradeGroups": or("tradeGroups", json!([])),
            "notes": or("notes", json!({})),
            "securities": or("securities", json!([])),
        }))
    })
    .await
}

/// The row counts the Data & storage dialog shows before a wipe.
fn data_summary(conn: &rusqlite::Connection) -> rusqlite::Result<Value> {
    let count = |sql: &str| -> rusqlite::Result<i64> { conn.query_row(sql, [], |r| r.get(0)) };
    let journal_raw = bagholder_store::tables::get_meta(conn, "journal_v2", "")?;
    let journal_n = serde_json::from_str::<Value>(&journal_raw).ok().and_then(|v| v.as_object().map(|m| m.len())).unwrap_or(0);
    let (first_act, last_act): (Option<String>, Option<String>) =
        conn.query_row("SELECT MIN(transaction_date), MAX(transaction_date) FROM activities", [], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(json!({
        "ok": true,
        "path": app().db_path().display().to_string(),
        "activities": count("SELECT COUNT(*) FROM activities")?,
        "firstActivity": first_act.unwrap_or_default(),
        "lastActivity": last_act.unwrap_or_default(),
        "accounts": count("SELECT COUNT(*) FROM accounts")?,
        "balances": count("SELECT COUNT(*) FROM balances")?,
        "navDays": count("SELECT COUNT(*) FROM nav_history")?,
        "securities": count("SELECT COUNT(*) FROM securities")?,
        "journal": journal_n,
        "fxDays": count("SELECT COUNT(*) FROM fx_rates")?,
        "benchmarkDays": count("SELECT COUNT(*) FROM benchmark_prices")?,
        "filings": count("SELECT COUNT(*) FROM filings")?,
        "syncedAt": bagholder_store::tables::get_meta(conn, "synced_at", "")?,
        "sessionPresent": session::load_session().is_some(),
    }))
}

async fn data(State(state): State<AppState>) -> Api {
    with_store(&state, data_summary).await
}

/// What `POST /api/data/clear` removes besides the synced rows.
#[derive(Deserialize, Default)]
struct Clear {
    #[serde(default)]
    journal: bool,
    #[serde(default)]
    market: bool,
    #[serde(default)]
    session: bool,
}

async fn data_clear(State(state): State<AppState>, Body(what): Body<Clear>) -> Api {
    if state.app.state.lock().unwrap().syncing {
        return Err(ApiError::Conflict("A sync is running. Wait for it to finish.".into()));
    }
    with_store(&state, move |conn| {
        bagholder_store::admin::clear_synced_data(conn, !what.journal, !what.market)?;
        if what.session {
            session::delete_session();
        }
        {
            let mut st = app().state.lock().unwrap();
            st.last_sync.clear();
            st.error.clear();
        }
        data_summary(conn)
    })
    .await
}

/// One trade's journal entry. A field left out is cleared, as the page sends all three.
#[derive(Deserialize, Default)]
struct JournalEntry {
    #[serde(default)]
    id: String,
    #[serde(default)]
    thesis: Value,
    #[serde(default)]
    tags: Value,
    #[serde(default)]
    grade: Value,
}

async fn journal(State(state): State<AppState>, Body(e): Body<JournalEntry>) -> Api {
    let id = e.id.trim().to_string();
    if id.is_empty() {
        return Err(ApiError::BadRequest("id required".into()));
    }
    let entry = json!({"thesis": e.thesis, "tags": e.tags, "grade": e.grade});
    with_store(&state, move |conn| Ok(json!({"ok": true, "journal": bagholder_store::admin::save_journal_entry(conn, &id, Some(&entry))?}))).await
}

#[derive(Deserialize, Default)]
struct Groups {
    groups: Option<Value>,
}

async fn groups(State(state): State<AppState>, Body(g): Body<Groups>) -> Api {
    with_store(&state, move |conn| Ok(json!({"ok": true, "groups": bagholder_store::tables::save_trade_groups(conn, g.groups.as_ref())?}))).await
}

#[derive(Deserialize, Default)]
struct Notes {
    notes: Option<Value>,
}

async fn notes(State(state): State<AppState>, Body(n): Body<Notes>) -> Api {
    with_store(&state, move |conn| Ok(json!({"ok": true, "notes": bagholder_store::tables::save_trade_notes(conn, n.notes.as_ref())?}))).await
}

/// A CSV the page read from a file the person chose.
#[derive(Deserialize, Default)]
struct Import {
    #[serde(default)]
    text: String,
    #[serde(default)]
    name: String,
}

async fn import(State(state): State<AppState>, Body(i): Body<Import>) -> Api {
    if bagholder_model::textrules::trim_space(&i.text).is_empty() {
        return Err(ApiError::BadRequest("text required".into()));
    }
    let name = if i.name.is_empty() { "upload.csv".to_string() } else { i.name };
    let app = state.app;
    let report = blocking(move || -> Result<Value, ApiError> {
        let conn = app.open()?;
        bagholder_store::csvimport::import_text(&conn, &name, &i.text).map_err(|e| {
            app::log(&format!("bagholder: import failed: {}", e));
            ApiError::Failed(e.to_string())
        })
    })
    .await??;
    Ok(Json(report))
}

async fn watch_status(State(state): State<AppState>) -> Api {
    with_store(&state, bagholder_store::csvimport::status).await
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
        if set.get("ok") != Some(&json!(true)) {
            return Ok((false, set));
        }
        let mut result = bagholder_store::csvimport::scan_folder(&conn, None, true)?;
        result["status"] = bagholder_store::csvimport::status(&conn)?;
        Ok((true, result))
    })
    .await??;
    Ok((if out.0 { axum::http::StatusCode::OK } else { axum::http::StatusCode::BAD_REQUEST }, Json(out.1)))
}

async fn watch_scan(State(state): State<AppState>) -> Result<(axum::http::StatusCode, Json<Value>), ApiError> {
    let app = state.app;
    let result = blocking(move || -> rusqlite::Result<Value> {
        let conn = app.open()?;
        let mut result = bagholder_store::csvimport::scan_folder(&conn, None, true)?;
        result["status"] = bagholder_store::csvimport::status(&conn)?;
        Ok(result)
    })
    .await??;
    Ok((if truthy(result.get("ok")) { axum::http::StatusCode::OK } else { axum::http::StatusCode::BAD_REQUEST }, Json(result)))
}

async fn watch_clear(State(state): State<AppState>) -> Api {
    with_store(&state, |conn| {
        bagholder_store::csvimport::clear_watch_folder(conn)?;
        bagholder_store::csvimport::status(conn)
    })
    .await
}
