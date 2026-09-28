//! The book and what is derived from it: the model, one trade, the header's
//! status, the journal, imports, and the Data & storage dialog.

use axum::extract::State;
use axum::routing::get;
use axum::Json;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::extract::trimmed;
use super::{api_routes, blocking, Api, ApiError, AppState, Body, Params, Routed};

use crate::app::App;

pub fn routes() -> Routed {
    let mut routed = api_routes! {
        get "/api/status" => status;
        get "/api/figures/detail" => figures_detail;
        get "/api/figures/details" => figures_details;
        get "/api/figures/trades" => figures_trades;
        get "/api/view" => view;
        post "/api/data/clear" => data_clear;
        post "/api/journal" => journal;
        post "/api/entries" => entries;
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
    Ok(Json(blocking(move || crate::status::answer(&state.app)).await?.map_err(ApiError::Failed)?))
}

/// `GET /api/figures/detail`: a trade's or a holding's fills, by its id.
async fn figures_detail(State(state): State<AppState>, Params(q): Params<TradeQuery>) -> Api<crate::wire::figures::Detail> {
    let app = state.app;
    let id = q.id.unwrap_or_default();
    let found = blocking(move || app.figures.get().and_then(|f| f.read(|e| crate::wire::build::detail(e, &id))).flatten()).await?;
    Ok(Json(found.ok_or_else(|| ApiError::NotFound("no such trade or holding".into()))?))
}

/// `GET /api/figures/details`: every trade's and holding's fills.
async fn figures_details(State(state): State<AppState>) -> Api<crate::wire::figures::Details> {
    let app = state.app;
    let found = blocking(move || app.figures.get().and_then(|f| f.read(|e| crate::wire::build::details(e)))).await?;
    Ok(Json(crate::wire::figures::Details { details: found.ok_or_else(|| ApiError::Conflict("the figures are not open".into()))? }))
}

#[derive(Deserialize, TS)]
pub struct ViewQuery {
    /// The subscription's key: `book`, `dashboard`, `positions`, `trades`,
    /// `cashflow`, `exposure`, `markets`, `trade:<id>`.
    #[serde(default, deserialize_with = "trimmed")]
    key: Option<String>,
    /// Its parameters, as the JSON a subscription is asked with (`views::Params`).
    #[serde(default, deserialize_with = "trimmed")]
    params: Option<String>,
}

/// A document of the figures, as a subscription is first sent it.
#[derive(Serialize, TS)]
#[ts(type = "unknown")]
pub struct ViewAnswer(serde_json::Value);

/// `GET /api/view`: one screen's document once, exactly as a page subscribing to
/// it is first sent it. A key or a parameter the figures do not know is refused,
/// naming it.
async fn view(State(state): State<AppState>, Params(q): Params<ViewQuery>) -> Api<ViewAnswer> {
    let app = state.app;
    let built = blocking(move || -> Result<serde_json::Value, ApiError> {
        let key = q.key.ok_or_else(|| ApiError::BadRequest("key required".into()))?;
        let params: serde_json::Value = match q.params {
            Some(raw) => serde_json::from_str(&raw).map_err(|e| ApiError::BadRequest(format!("the parameters: {e}")))?,
            None => serde_json::json!({}),
        };
        let f = app.figures.get().ok_or_else(|| ApiError::Failed("the figures are not open".into()))?;
        let names = f.names().map_err(ApiError::Failed)?;
        let context = app.market_context().map_err(|e| ApiError::Failed(format!("the market's context: {e}")))?;
        f.read(|e| crate::views::snapshot_of(&crate::views::Cx { engine: e, names: &names, tables: &crate::wire::context::Door { built: &context, app: &app }, following: &context.following }, &key, params))
            .ok_or_else(|| ApiError::Conflict("no page has stated its zone yet".into()))?
            .map_err(ApiError::BadRequest)
    })
    .await??;
    Ok(Json(ViewAnswer(built)))
}

#[derive(Deserialize, TS)]
pub struct TradesQuery {
    /// The page's filters, as the JSON it keeps them in (`wire::filters::Filters`).
    #[serde(default, deserialize_with = "trimmed")]
    filters: Option<String>,
    /// The column the list is sorted by, as the page's header names it.
    #[serde(default, deserialize_with = "trimmed")]
    sort: Option<String>,
    /// `asc` or `desc`.
    #[serde(default, deserialize_with = "trimmed")]
    dir: Option<String>,
}

/// `GET /api/figures/trades`: every trade under the page's filters, in its order:
/// what the Trades CSV export writes. A filter or a column the figures do not
/// know is refused, naming it.
async fn figures_trades(State(state): State<AppState>, Params(q): Params<TradesQuery>) -> Api<crate::wire::figures::TradesDoc> {
    let app = state.app;
    let built = blocking(move || -> Result<crate::wire::figures::TradesDoc, ApiError> {
        let filters: crate::wire::filters::Filters = match q.filters {
            Some(raw) => serde_json::from_str(&raw).map_err(|e| ApiError::BadRequest(format!("the filters: {e}")))?,
            None => Default::default(),
        };
        let filters = filters.to_engine().map_err(ApiError::BadRequest)?;
        let dir = match q.dir.as_deref() {
            None | Some("desc") => crate::views::Dir::Desc,
            Some("asc") => crate::views::Dir::Asc,
            Some(other) => return Err(ApiError::BadRequest(format!("no direction {other:?}"))),
        };
        let sort = crate::views::Sort { key: q.sort.unwrap_or_else(|| "activity".into()), dir };
        let f = app.figures.get().ok_or_else(|| ApiError::Failed("the figures are not open".into()))?;
        let names = f.names().map_err(ApiError::Failed)?;
        let context = app.market_context().map_err(|e| ApiError::Failed(format!("the market's context: {e}")))?;
        f.read(|e| crate::views::all_trades(&crate::views::Cx { engine: e, names: &names, tables: &crate::wire::context::Door { built: &context, app: &app }, following: &context.following }, filters, sort))
            .ok_or_else(|| ApiError::Conflict("no page has stated its zone yet".into()))?
            .map_err(ApiError::BadRequest)
    })
    .await??;
    Ok(Json(built))
}

#[derive(Deserialize, TS)]
pub struct TradeQuery {
    #[serde(default, deserialize_with = "trimmed")]
    id: Option<String>,
}

/// `POST /api/data/clear`: the kinds of data ticked.
#[derive(Deserialize, Default, TS)]
#[serde(deny_unknown_fields)]
pub struct Clear {
    kinds: Vec<crate::clear::Kind>,
}

#[derive(Serialize, TS)]
pub struct ClearAnswer {
    ok: bool,
}

async fn data_clear(State(state): State<AppState>, Body(what): Body<Clear>) -> Api<ClearAnswer> {
    if what.kinds.is_empty() {
        return Err(ApiError::BadRequest("nothing was ticked".into()));
    }
    let app = state.app;
    blocking(move || -> Result<ClearAnswer, ApiError> {
        let f = open_figures(&app)?;
        match crate::clear::clear(&app, f, &what.kinds, bagholder_core::jiff::Timestamp::now()) {
            Ok(()) => Ok(ClearAnswer { ok: true }),
            Err(e @ (crate::clear::Refused::Pulling | crate::clear::Refused::BracketLive(_))) => Err(ApiError::Conflict(e.to_string())),
            Err(crate::clear::Refused::Failed(why)) => Err(ApiError::Failed(why)),
        }
    })
    .await?
    .map(Json)
}

/// One trade's journal entry, whole: the page sends all three fields.
#[derive(Deserialize, Default, TS)]
#[serde(deny_unknown_fields)]
pub struct JournalEntryRequest {
    /// The trade's or the group's id, as the figures name it.
    id: String,
    thesis: String,
    /// `A`, `B`, `C`, `F`, or empty for none.
    grade: String,
    tags: Vec<String>,
}

/// `POST /api/journal`: written to the book; the page hears it on its stream.
#[derive(Serialize, TS)]
pub struct JournalAnswer {
    #[ts(type = "true")]
    ok: bool,
}

async fn journal(State(state): State<AppState>, Body(e): Body<JournalEntryRequest>) -> Api<JournalAnswer> {
    let id = e.id.trim().to_string();
    if id.is_empty() {
        return Err(ApiError::BadRequest("id required".into()));
    }
    let grade = match e.grade.trim() {
        "" => None,
        g => Some(bagholder_core::journal::Grade::parse(g).map_err(|err| ApiError::BadRequest(format!("grade {g:?}: {err}")))?),
    };
    let entry = bagholder_core::journal::JournalEntry { thesis: e.thesis, grade, tags: e.tags.iter().map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect() };
    let app = state.app;
    blocking(move || -> Result<JournalAnswer, ApiError> {
        let f = app.figures.get().ok_or_else(|| ApiError::Failed("the figures are not open".into()))?;
        match f.write_journal(&id, &entry, bagholder_core::jiff::Timestamp::now()) {
            Ok(_) => Ok(JournalAnswer { ok: true }),
            Err(crate::figures::JournalRefused::Unknown(id)) => Err(ApiError::NotFound(format!("no trade {id}"))),
            Err(crate::figures::JournalRefused::Failed(e)) => Err(ApiError::Failed(e)),
        }
    })
    .await?
    .map(Json)
}

/// `POST /api/entries`: kept in the book as the person's record; the page hears the
/// figures move on its stream.
#[derive(Serialize, TS)]
pub struct EntryAnswer {
    #[ts(type = "true")]
    ok: bool,
}

async fn entries(State(state): State<AppState>, Body(e): Body<crate::entries::EntryRequest>) -> Api<EntryAnswer> {
    let app = state.app;
    blocking(move || -> Result<EntryAnswer, ApiError> {
        let f = app.figures.get().ok_or_else(|| ApiError::Failed("the figures are not open".into()))?;
        match crate::entries::enter(f, &e, bagholder_core::jiff::Timestamp::now()) {
            Ok(()) => Ok(EntryAnswer { ok: true }),
            Err(crate::entries::Refused::Entry(why)) => Err(ApiError::BadRequest(why)),
            Err(crate::entries::Refused::Failed(why)) => Err(ApiError::Failed(why)),
        }
    })
    .await?
    .map(Json)
}

/// The figures, or a failure saying they are not open.
fn open_figures(app: &App) -> Result<&crate::figures::Figures, ApiError> {
    app.figures.get().ok_or_else(|| ApiError::Failed("the figures are not open".into()))
}

fn refused(r: crate::entries::Refused) -> ApiError {
    match r {
        crate::entries::Refused::Entry(why) => ApiError::BadRequest(why),
        crate::entries::Refused::Failed(why) => ApiError::Failed(why),
    }
}

fn account_of(text: &str) -> Result<Option<bagholder_core::AccountId>, ApiError> {
    match text.trim() {
        "" => Ok(None),
        a => bagholder_core::AccountId::parse(a).map(Some).map_err(|_| ApiError::BadRequest(format!("{a:?} is not an account"))),
    }
}

/// `POST /api/import`: a file's rows kept, and what they did.
async fn import(State(state): State<AppState>, Body(i): Body<crate::csv_import::ImportRequest>) -> Api<crate::csv_import::ImportReport> {
    let app = state.app;
    blocking(move || -> Result<crate::csv_import::ImportReport, ApiError> {
        let name = if i.name.trim().is_empty() { "upload.csv" } else { i.name.trim() };
        crate::csv_import::import(open_figures(&app)?, name, &i.text, account_of(&i.account)?, bagholder_core::jiff::Timestamp::now()).map_err(refused)
    })
    .await?
    .map(Json)
}

async fn watch_status(State(state): State<AppState>) -> Api<crate::csv_import::WatchStatus> {
    let app = state.app;
    blocking(move || crate::csv_import::watch_status(open_figures(&app)?).map_err(ApiError::Failed)).await?.map(Json)
}

/// `POST /api/watch`: the folder watched, its files going to the account named,
/// and every one read at once.
async fn watch_set(State(state): State<AppState>, Body(w): Body<crate::csv_import::WatchRequest>) -> Api<crate::csv_import::WatchStatus> {
    let app = state.app;
    blocking(move || -> Result<crate::csv_import::WatchStatus, ApiError> {
        let f = open_figures(&app)?;
        let now = bagholder_core::jiff::Timestamp::now();
        crate::csv_import::watch(f, &w, now).map_err(refused)?;
        app.events.signal();
        crate::csv_import::scan(f, true, now).map_err(ApiError::Failed)
    })
    .await?
    .map(Json)
}

/// `POST /api/watch/scan`: every file of the watched folder read again.
async fn watch_scan(State(state): State<AppState>) -> Api<crate::csv_import::WatchStatus> {
    let app = state.app;
    blocking(move || -> Result<crate::csv_import::WatchStatus, ApiError> {
        let f = open_figures(&app)?;
        if !crate::csv_import::watching(f) {
            return Err(ApiError::BadRequest("no folder is watched".into()));
        }
        crate::csv_import::scan(f, true, bagholder_core::jiff::Timestamp::now()).map_err(ApiError::Failed)
    })
    .await?
    .map(Json)
}

async fn watch_clear(State(state): State<AppState>) -> Api<crate::csv_import::WatchStatus> {
    let app = state.app;
    blocking(move || -> Result<crate::csv_import::WatchStatus, ApiError> {
        let f = open_figures(&app)?;
        crate::csv_import::unwatch(f, bagholder_core::jiff::Timestamp::now()).map_err(ApiError::Failed)?;
        crate::csv_import::watch_status(f).map_err(ApiError::Failed)
    })
    .await?
    .map(Json)
}
