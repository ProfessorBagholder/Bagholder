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

use crate::app::App;
use crate::feeds;

pub fn routes() -> Routed {
    let mut routed = api_routes! {
        get "/api/status" => status;
        get "/api/model" => model;
        get "/api/trade" => trade;
        get "/api/figures" => figures;
        get "/api/figures/detail" => figures_detail;
        get "/api/book" => book;
        post "/api/data/clear" => data_clear;
        post "/api/journal" => journal;
        post "/api/entries" => entries;
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
pub struct FiguresQuery {
    /// The page's filters, as the JSON it keeps them in (`wire::filters::Filters`).
    #[serde(default, deserialize_with = "trimmed")]
    filters: Option<String>,
}

/// `GET /api/figures`: the figures document from the engine, for the page's
/// filters. A filter the engine does not know is refused, naming it; before a
/// page has stated its zone there is nothing to build.
async fn figures(State(state): State<AppState>, Params(q): Params<FiguresQuery>) -> Api<crate::wire::figures::Figures> {
    let app = state.app;
    let built = blocking(move || -> Result<crate::wire::figures::Figures, ApiError> {
        let filters: crate::wire::filters::Filters = match q.filters {
            Some(raw) => serde_json::from_str(&raw).map_err(|e| ApiError::BadRequest(format!("the filters: {e}")))?,
            None => Default::default(),
        };
        let filters = filters.to_engine().map_err(ApiError::BadRequest)?;
        let f = app.figures.get().ok_or_else(|| ApiError::Failed("the figures are not open".into()))?;
        let book = f.book().map_err(ApiError::Failed)?;
        let names = f.read(|e| crate::wire::build::Names::load(&book, e.inputs())).ok_or_else(|| ApiError::Conflict("no page has stated its zone yet".into()))?.map_err(ApiError::Failed)?;
        let base = app.base().map_err(|e| ApiError::Failed(format!("the market's context: {e}")))?;
        f.read(|e| crate::wire::build::build(e, &names, &filters, &base)).ok_or_else(|| ApiError::Conflict("no page has stated its zone yet".into()))
    })
    .await??;
    Ok(Json(built))
}

/// `GET /api/figures/detail`: a trade's or a holding's fills, by its id.
async fn figures_detail(State(state): State<AppState>, Params(q): Params<TradeQuery>) -> Api<crate::wire::figures::Detail> {
    let app = state.app;
    let id = q.id.unwrap_or_default();
    let found = blocking(move || app.figures.get().and_then(|f| f.read(|e| crate::wire::build::detail(e, &id))).flatten()).await?;
    Ok(Json(found.ok_or_else(|| ApiError::NotFound("no such trade or holding".into()))?))
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
