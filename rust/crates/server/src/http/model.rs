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
        get "/api/transactions" => transactions;
        post "/api/data/clear" => data_clear;
        post "/api/journal" => journal;
        post "/api/debts/answer" => debts_answer;
        post "/api/import/told" => import_told;
        post "/api/entries" => entries;
        post "/api/watch/clear" => watch_clear;
    };
    // `/api/watch`'s GET and POST share one path, which `api_routes!` cannot
    // declare twice; their entries are read from the handlers all the same
    // one import at a time: a second is refused before its body is read (brief 19, change 6)
    routed.router = routed
        .router
        .merge(axum::Router::new().route("/api/import", axum::routing::post(import)).layer(axum::middleware::from_fn(import_slot)))
        .route("/api/import/stop", axum::routing::post(import_stop))
        .route("/api/watch", get(watch_status).post(watch_set))
        .route("/api/watch/scan", axum::routing::post(watch_scan));
    routed.table.push(super::RouteEntry::of(import, "post", "/api/import"));
    routed.table.push(super::RouteEntry::of(import_stop, "post", "/api/import/stop"));
    routed.table.push(super::RouteEntry::of(watch_status, "get", "/api/watch"));
    routed.table.push(super::RouteEntry::of(watch_set, "post", "/api/watch"));
    routed.table.push(super::RouteEntry::of(watch_scan, "post", "/api/watch/scan"));
    routed
}

pub(crate) async fn status(State(state): State<AppState>) -> Api<crate::status::StatusAnswer> {
    Ok(Json(blocking(move || crate::status::answer(&state.app)).await?))
}

/// `GET /api/figures/detail`: a trade's or a holding's fills, by its id.
pub(crate) async fn figures_detail(State(state): State<AppState>, Params(q): Params<TradeQuery>) -> Api<crate::wire::figures::Detail> {
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

#[derive(Deserialize, Serialize, TS)]
pub struct ViewQuery {
    /// The subscription's key: `book`, `dashboard`, `positions`, `trades`,
    /// `cashflow`, `exposure`, `markets`, `trade:<id>`.
    #[serde(default, deserialize_with = "trimmed")]
    pub(crate) key: Option<String>,
    /// Its parameters, as the JSON a subscription is asked with (`views::Params`).
    #[serde(default, deserialize_with = "trimmed")]
    pub(crate) params: Option<String>,
}

/// A document of the figures, as a subscription is first sent it.
#[derive(Serialize, TS)]
#[ts(type = "unknown")]
pub struct ViewAnswer(serde_json::Value);

/// `GET /api/view`: one screen's document once, exactly as a page subscribing to
/// it is first sent it. A key or a parameter the figures do not know is refused,
/// naming it.
pub(crate) async fn view(State(state): State<AppState>, Params(q): Params<ViewQuery>) -> Api<ViewAnswer> {
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

#[derive(Deserialize, Serialize, TS)]
pub struct TradeQuery {
    #[serde(default, deserialize_with = "trimmed")]
    pub(crate) id: Option<String>,
}

#[derive(Deserialize, Serialize, TS, Default)]
pub struct TransactionsQuery {
    /// The first day, `YYYY-MM-DD`.
    #[serde(default, deserialize_with = "trimmed")]
    pub(crate) from: Option<String>,
    /// The last day, `YYYY-MM-DD`.
    #[serde(default, deserialize_with = "trimmed")]
    pub(crate) to: Option<String>,
    /// One chequing or credit card account's id, as the book's accounts give it.
    #[serde(default, deserialize_with = "trimmed")]
    pub(crate) account: Option<String>,
    /// Where the page starts: the `next` the previous page gave, 0 for the first.
    #[serde(default, deserialize_with = "trimmed")]
    pub(crate) offset: Option<String>,
    /// How many rows the page holds, 1 to 1,000.
    #[serde(default, deserialize_with = "trimmed")]
    pub(crate) size: Option<String>,
}

/// A page of the money in and out of the person's chequing and credit card accounts.
#[derive(Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TransactionsAnswer {
    pub total: u32,
    pub offset: u32,
    pub next: Option<u32>,
    pub transactions: Vec<TransactionRow>,
}

#[derive(Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TransactionRow {
    /// The day the account files it under, `YYYY-MM-DD`.
    pub date: String,
    /// The account's name, as the page names it, and its id.
    pub account: String,
    pub account_id: String,
    /// What it is: `card-purchase`, `card-refund`, `cashback`, `deposit`, `withdrawal`, `transfer-in`,
    /// `transfer-out`, `interest`, `interest-charge`, `fee` and the like.
    pub kind: String,
    /// The company it names: a card purchase's merchant, a bill payment's company, a pre-authorized
    /// debit's originator. Never a person's name.
    pub company: Option<String>,
    /// Into the account (positive) or out of it (negative), as the account states it.
    pub amount: String,
    pub currency: String,
}

/// `GET /api/transactions`: the money in and out of the person's chequing and
/// credit card accounts, newest first, a page at a time, each with the company it
/// names (`bagholder_wealthsimple::spending`) and never a person's name: what an
/// agent reads of the person's spending. A day or an account the book does not
/// hold is refused, naming it.
pub(crate) async fn transactions(State(state): State<AppState>, Params(q): Params<TransactionsQuery>) -> Api<TransactionsAnswer> {
    use bagholder_core::account::{AccountKind, AccountType};
    let app = state.app;
    blocking(move || -> Result<TransactionsAnswer, ApiError> {
        let day = |s: &Option<String>, what: &str| s.as_deref().map(|d| d.parse::<bagholder_core::jiff::civil::Date>().map_err(|_| ApiError::BadRequest(format!("{what} is not a day (YYYY-MM-DD): {d}")))).transpose();
        let count = |s: &Option<String>, what: &str| s.as_deref().map(|n| n.parse::<usize>().map_err(|_| ApiError::BadRequest(format!("{what} is not a number: {n}")))).transpose();
        let (from, to) = (day(&q.from, "from")?, day(&q.to, "to")?);
        let (offset, size) = (count(&q.offset, "offset")?.unwrap_or(0), count(&q.size, "size")?.unwrap_or(crate::views::FIRST_ROWS).clamp(1, 1000));
        let f = app.figures.get().ok_or_else(|| ApiError::Conflict("the figures are not open".into()))?;
        let (total, mut page) = f
            .read(|e| -> Result<_, ApiError> {
                let inputs = e.inputs();
                let mut accounts: Vec<(bagholder_core::AccountId, String)> = inputs
                    .ledger
                    .accounts
                    .iter()
                    .filter(|(_, i)| matches!(i.account.account_type, AccountType::Known { kind: AccountKind::Spending | AccountKind::CreditCard, .. }))
                    .map(|(id, _)| (*id, crate::wire::build::account_name(inputs, *id)))
                    .collect();
                if let Some(a) = &q.account {
                    accounts.retain(|(id, _)| id.to_string() == *a);
                    if accounts.is_empty() {
                        return Err(ApiError::BadRequest(format!("{a} is not a chequing or credit card account in this book")));
                    }
                }
                let mut rows: Vec<(&bagholder_core::transaction::Transaction, &String)> = accounts
                    .iter()
                    .flat_map(|(id, name)| inputs.ledger.transactions.cash_of(*id).map(move |t| (t, name)))
                    .filter(|(t, _)| t.cash.is_some() && from.is_none_or(|f| t.trade_date >= f) && to.is_none_or(|d| t.trade_date <= d))
                    .collect();
                rows.sort_by(|a, b| b.0.trade_date.cmp(&a.0.trade_date).then(b.0.occurred_at.cmp(&a.0.occurred_at)).then(a.0.id.cmp(&b.0.id)));
                let total = rows.len();
                let page: Vec<(bagholder_core::TransactionId, TransactionRow)> = rows
                    .into_iter()
                    .skip(offset)
                    .take(size)
                    .filter_map(|(t, name)| {
                        let cash = t.cash.as_ref()?;
                        Some((t.id.clone(), TransactionRow { date: t.trade_date.to_string(), account: name.clone(), account_id: t.account.to_string(), kind: t.kind.as_str().to_string(), company: None, amount: cash.amount.to_string(), currency: cash.currency.to_string() }))
                    })
                    .collect();
                Ok((total, page))
            })
            .ok_or_else(|| ApiError::Conflict("the figures are not open".into()))??;
        let book = f.book().map_err(ApiError::Failed)?;
        let records: Vec<bagholder_core::RecordId> = page.iter().map(|(id, _)| id.record).collect();
        let payloads = book.newest_payloads(&records).map_err(|e| ApiError::Failed(e.to_string()))?;
        for (id, row) in &mut page {
            row.company = payloads.get(&id.record).and_then(|p| bagholder_wealthsimple::spending::company_of_record(p));
        }
        let end = offset + page.len();
        Ok(TransactionsAnswer { total: total as u32, offset: offset as u32, next: (end < total).then_some(end as u32), transactions: page.into_iter().map(|(_, r)| r).collect() })
    })
    .await?
    .map(Json)
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

/// The person's answer for a payee no rule matched to a debt held outside: the
/// debt its payments repay, or none (not a debt).
#[derive(Deserialize, Default, TS)]
#[serde(deny_unknown_fields)]
pub struct DebtAnswerRequest {
    /// The payee, as the Cashflow document's `asked` names it.
    payee: String,
    /// The outside debt's id, or null: not a debt.
    debt: Option<String>,
}

/// `POST /api/debts/answer`: kept in the book; every payment to the payee follows
/// it, and the page hears the figures move on its stream.
#[derive(Serialize, TS)]
pub struct DebtAnswer {
    #[ts(type = "true")]
    ok: bool,
}

async fn debts_answer(State(state): State<AppState>, Body(a): Body<DebtAnswerRequest>) -> Api<DebtAnswer> {
    let payee = a.payee.trim().to_string();
    if payee.is_empty() {
        return Err(ApiError::BadRequest("payee required".into()));
    }
    let debt = a.debt.map(|d| d.trim().to_string()).filter(|d| !d.is_empty());
    let app = state.app;
    blocking(move || -> Result<DebtAnswer, ApiError> {
        let f = app.figures.get().ok_or_else(|| ApiError::Failed("the figures are not open".into()))?;
        let book = f.book().map_err(ApiError::Failed)?;
        // an answer is for a payee asked about, naming a debt it may be
        let asked = crate::engine_inputs::debts(&book).map_err(ApiError::Failed)?.asked;
        let Some(q) = asked.iter().find(|q| q.key == payee) else {
            return Err(ApiError::NotFound(format!("no payment waits for an answer about {payee}")));
        };
        if let Some(d) = &debt {
            if !q.candidates.contains(d) {
                return Err(ApiError::BadRequest(format!("{d} is not a debt these payments may repay")));
            }
        }
        let now = bagholder_core::jiff::Timestamp::now();
        book.answer_repayment(&payee, debt.as_deref(), now).map_err(|e| ApiError::Failed(e.to_string()))?;
        f.record_changed(now).map_err(ApiError::Failed)?;
        Ok(DebtAnswer { ok: true })
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

/// Whether an import request is being answered now.
pub(crate) static IMPORTING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// What a second import is told while one runs.
pub const IMPORT_BUSY: &str = "An import is already running: try again when it has finished.";

/// The import's slot, taken before its body is read and given back when its answer
/// has gone: a second import while one runs is refused at once, its body never read.
async fn import_slot(req: axum::extract::Request, next: axum::middleware::Next) -> axum::response::Response {
    use axum::response::IntoResponse;
    use std::sync::atomic::Ordering;
    if IMPORTING.swap(true, Ordering::SeqCst) {
        return ApiError::Conflict(IMPORT_BUSY.into()).into_response();
    }
    struct Release;
    impl Drop for Release {
        fn drop(&mut self) {
            IMPORTING.store(false, Ordering::SeqCst);
        }
    }
    let _held = Release;
    next.run(req).await
}

/// `POST /api/import`'s query: the file's name and the account its rows go to.
#[derive(Debug, Default, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct ImportQuery {
    pub name: String,
    /// Empty is the Manual account.
    pub account: String,
}

/// The file an import is receiving, removed once its import is over, however it ended.
struct Incoming(std::path::PathBuf);

impl Drop for Incoming {
    fn drop(&mut self) {
        if let Err(e) = std::fs::remove_file(&self.0) {
            if e.kind() != std::io::ErrorKind::NotFound {
                crate::app::log(&format!("bagholder import: {} could not be removed: {e}", self.0.display()));
            }
        }
    }
}

/// `POST /api/import`: the file as it is, then its rows kept as a job. The file is
/// written to the data folder as it arrives and read from there a row at a time, so a
/// file of any size is taken in bounded memory. The answer is the import's id, once
/// the file has arrived whole: the reading then runs on whatever happens to the
/// request (a reverse proxy's timeout, the tab closed), as Sharesight and Tradervue
/// run an upload. How far it has come is the status's `importing`, what it did its
/// `imported`, and `POST /api/import/stop` ends it: rows already kept stay.
async fn import(State(state): State<AppState>, Params(q): Params<ImportQuery>, upload: super::extract::Upload) -> Api<crate::csv_import::ImportAccepted> {
    use futures_util::StreamExt;
    use tokio::io::AsyncWriteExt;
    let app = state.app;
    let name = if q.name.trim().is_empty() { "upload.csv".to_string() } else { q.name.trim().to_string() };
    let account = account_of(&q.account)?;
    let dir = app.home.join("incoming");
    tokio::fs::create_dir_all(&dir).await.map_err(|e| ApiError::Failed(format!("The file could not be received: {e}")))?;
    let incoming = Incoming(dir.join(format!("{}.csv", crate::app::uuid4())));
    let id = crate::app::uuid4();
    {
        let mut st = app.state.lock().unwrap();
        // a job still reading holds the slot past its request
        if st.importing.is_some() {
            return Err(ApiError::Conflict(IMPORT_BUSY.into()));
        }
        st.import_stop = false;
        st.importing = Some(crate::csv_import::Importing { id: id.clone(), file: name.clone(), received: 0, size: upload.size, checked: 0, rows: 0, total: None });
    }
    // the slot given back with the import, whichever way it ends: the request's while
    // the file arrives, the job's once it reads. A job that ends without saying what
    // it did (it panicked) says that, so nothing waits on it for ever
    struct Done(std::sync::Arc<crate::app::App>, Option<(String, String)>);
    impl Drop for Done {
        fn drop(&mut self) {
            let mut st = self.0.state.lock().unwrap();
            if let Some((id, file)) = self.1.take() {
                st.imported = Some(crate::csv_import::Imported { id, file, report: None, error: Some("The import failed part way: the rows it kept stay, and importing the file again goes on from them.".into()), told: false });
            }
            st.importing = None;
            st.import_stop = false;
        }
    }
    let mut done = Done(app.clone(), None);
    let mut out = tokio::fs::File::create(&incoming.0).await.map_err(|e| ApiError::Failed(format!("The file could not be received: {e}")))?;
    let mut stream = upload.body.into_data_stream();
    let mut received: u64 = 0;
    let mut said: Option<u64> = None;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| ApiError::BadRequest(format!("The file did not arrive whole: {e}")))?;
        out.write_all(&chunk).await.map_err(|e| ApiError::Failed(format!("The file could not be received: {e}")))?;
        received += chunk.len() as u64;
        // told when the whole percent arrived moves, or by the megabyte where no size is stated
        let step = match upload.size {
            Some(size) if size > 0 => received * 100 / size,
            _ => received >> 20,
        };
        if said != Some(step) {
            said = Some(step);
            let mut st = app.state.lock().unwrap();
            if st.import_stop {
                return Err(ApiError::Conflict(crate::csv_import::IMPORT_STOPPED.into()));
            }
            if let Some(i) = st.importing.as_mut() {
                i.received = received;
            }
        }
    }
    out.flush().await.map_err(|e| ApiError::Failed(format!("The file could not be received: {e}")))?;
    drop(out);
    // nothing arrived: refused now, with the request, as a file that cannot be read
    if received == 0 {
        return Err(ApiError::BadRequest("the file is empty".into()));
    }
    // the whole file arrived: what it is checked against
    if let Some(i) = app.state.lock().unwrap().importing.as_mut() {
        i.received = received;
    }
    let a2 = app.clone();
    let job = id.clone();
    done.1 = Some((id.clone(), name.clone()));
    // the reading, as a job of its own: nothing waits on it, and its end is the status's
    tokio::task::spawn_blocking(move || {
        let path = incoming.0.clone();
        let open = || std::fs::File::open(&path).map(|f| Box::new(std::io::BufReader::new(f)) as Box<dyn std::io::BufRead>);
        let mut said: Option<(bool, u64)> = None;
        let mut progress = |step: crate::csv_import::Step| {
            let mut st = a2.state.lock().unwrap();
            if st.import_stop {
                return crate::csv_import::Go::Stop;
            }
            // told when the whole percent moves, of the bytes checked, then of the rows
            // kept; the status is written then only, each write telling the page
            let received = st.importing.as_ref().map_or(0, |i| i.received);
            let at = match step {
                crate::csv_import::Step::Checked(bytes) => (false, bytes * 100 / received.max(1)),
                crate::csv_import::Step::Kept { rows, total } => (true, rows * 100 / total.max(1)),
            };
            if said != Some(at) {
                said = Some(at);
                if let Some(i) = st.importing.as_mut() {
                    match step {
                        crate::csv_import::Step::Checked(bytes) => i.checked = bytes,
                        crate::csv_import::Step::Kept { rows, total } => {
                            i.checked = i.received;
                            i.rows = rows;
                            i.total = Some(total);
                        }
                    }
                }
            }
            crate::csv_import::Go::On
        };
        let outcome = match open_figures(&a2) {
            Ok(f) => crate::csv_import::import_from(f, &name, &open, account, bagholder_core::jiff::Timestamp::now(), &mut progress).map_err(|e| match e {
                crate::entries::Refused::Entry(why) => why,
                other => refused(other).message(),
            }),
            Err(e) => Err(e.message()),
        };
        drop(incoming);
        // what it did, said with the slot given back, in one write
        let mut st = a2.state.lock().unwrap();
        let (report, error) = match outcome {
            Ok(r) => (Some(r), None),
            Err(why) => (None, Some(why)),
        };
        st.imported = Some(crate::csv_import::Imported { id: job, file: name, report, error, told: false });
        st.importing = None;
        st.import_stop = false;
        drop(st);
        let mut done = done;
        done.1 = None;
        drop(done);
    });
    Ok(Json(crate::csv_import::ImportAccepted { id }))
}

/// `POST /api/import/told`'s body: the import a page has shown the report of.
#[derive(Debug, Default, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ImportTold {
    pub id: String,
}

/// `POST /api/import/told`: a page has shown what the import did, so no page says
/// it again. Kept by the server, so it holds for every page on every device, never
/// per browser.
async fn import_told(State(state): State<AppState>, Body(t): Body<ImportTold>) -> Api<super::OkOr> {
    let mut st = state.app.state.lock().unwrap();
    if let Some(done) = st.imported.as_mut().filter(|d| d.id == t.id && !d.told) {
        done.told = true;
    }
    Ok(Json(super::OkOr::ok()))
}

/// `POST /api/import/stop`: the running import ends at its next row; the rows it kept stay.
async fn import_stop(State(state): State<AppState>) -> Api<super::OkOr> {
    let mut st = state.app.state.lock().unwrap();
    if st.importing.is_some() {
        st.import_stop = true;
    }
    Ok(Json(super::OkOr::ok()))
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
