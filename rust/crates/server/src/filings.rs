//! The disclosures and the release notices: which filers a listing's filings come
//! from, each filer's documents read and kept in the market cache, the sweep that
//! tells what is new, the reading of each document's title and sentence, and the
//! notices a wire's or a regulator's press release becomes (plan
//! `market-data-keyed-by-listing`, Approach, 2 and 3).

use rusqlite::Connection;
use serde::Serialize;
use std::collections::{BTreeMap, HashSet};
use std::sync::atomic::Ordering;
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use ts_rs::TS;

use bagholder_diff_derive::Diff;
use bagholder_market::disclosures::{self, SourceOutcome};
use bagholder_market::{edgar, enrich, localmodel, news, sedar};
use bagholder_model::securities::Security;
use bagholder_core::ListingKey;
use bagholder_market::stored::feeds::{self as sf, FiledDocument, Filing, NewsItem, Regulator};
use bagholder_market::stored::meta::{get_meta, set_meta};

use crate::app::{log, now_iso, now_unix, parse_instant, spawn, App, ENRICH_VERSION};
use crate::feeds::{base, conn, feed_failed, notice_settings, notices, sha1_hex12, today, went, DISCLOSURES, DISCLOSURES_READING, DISCLOSURES_STORED, RELEASES};
use crate::http::OkOr;
use crate::notify;

// ---------------------------------------------------------------------------
// disclosures
// ---------------------------------------------------------------------------

pub const FILINGS_STALE_HOURS: f64 = 24.0;
pub use bagholder_market::disclosures::FILINGS_SWEEP_EVERY_SEC;
pub const FILINGS_HOLD_MAX_MIN: f64 = 120.0;
pub const FILINGS_HOLD_KEY: &str = "filings:held-since";
pub const FILINGS_SWEEP_AGE_MIN: f64 = 30.0;

pub use bagholder_market::disclosures::FILERS_RECHECK_HOURS;

/// The name, the venue in words and the currency the book's securities give the
/// listing `key` names; empty where the book holds none on that listing.
pub fn instrument_meta(app: &App, key: &ListingKey) -> (String, String, String) {
    let secs = match crate::market_context::securities(app) {
        Ok(v) => v,
        Err(e) => {
            log(&format!("bagholder: the book's securities could not be read for {key}: {e}"));
            vec![]
        }
    };
    meta_of(&secs, key)
}

/// The name, venue in words and currency of the one of `secs` on the listing `key`
/// names; never another listing's with the same ticker (brief 34, U11).
pub fn meta_of(secs: &[Security], key: &ListingKey) -> (String, String, String) {
    for sec in secs {
        if ListingKey::on(&sec.symbol, &sec.primary_exchange).is_ok_and(|k| k == *key) {
            return (sec.name.trim().to_string(), sec.primary_exchange.trim().to_string(), sec.currency.trim().to_string());
        }
    }
    (String::new(), String::new(), String::new())
}

/// Hours since an instant stored as text; none for one never stored or unreadable.
fn age_hours(when: &str) -> Option<f64> {
    if when.is_empty() {
        return None;
    }
    parse_instant(when).map(|then| (now_unix() - then) / 3600.0)
}

/// The regulator a listing files with: the SEC for a US listing, on an exchange or
/// over the counter, SEDAR+ for a Canadian one; none for any other (a coin, an index).
pub fn own_regulator(key: &ListingKey) -> Option<Regulator> {
    if key.us() || key.otc() {
        Some(Regulator::Sec)
    } else if key.canadian() {
        Some(Regulator::Sedar)
    } else {
        None
    }
}

pub use bagholder_market::disclosures::FAILED_READ_REST;

/// Whether a failed read is still within its rest.
fn resting(f: Option<&sf::ReadFailure>) -> bool {
    f.is_some_and(|f| age_hours(&f.failed_at).is_some_and(|h| h * 3600.0 < bagholder_sources::market::grown_rest(FAILED_READ_REST, f.failures).as_secs_f64()))
}

/// A read of `subject` from `source` that failed, kept so it rests; an offline
/// refusal is no read, and is not kept.
fn failed_read(c: &Connection, subject: &str, source: &str, why: &bagholder_net::Failure) -> Result<Option<sf::ReadFailure>, String> {
    match why {
        bagholder_net::Failure::Offline => Ok(None),
        bagholder_net::Failure::Said(why) => sf::read_failed(c, subject, source, why, &now_iso()).map(Some).map_err(|e| e.to_string()),
    }
}

/// The TSX's interlisted list, read again when the copy kept is a day old; a read
/// that fails keeps the last list, rests (`FAILED_READ_REST`), and goes to the log
/// as it starts failing and as it ends, never to the page (brief 79, amended
/// 2026-10-28).
fn interlisted_current(app: &Arc<App>, c: &Connection, regulators: &dyn disclosures::Regulators) -> Result<(), String> {
    let read = sf::interlisted_read_at(c).map_err(|e| e.to_string())?;
    if age_hours(&read).is_some_and(|h| h < bagholder_market::interlisted::READ_EVERY_HOURS) {
        return Ok(());
    }
    let failure = sf::read_failure(c, INTERLISTED, INTERLISTED_SOURCE).map_err(|e| e.to_string())?;
    if resting(failure.as_ref()) {
        return Ok(());
    }
    match regulators.interlisted() {
        Ok((as_of, rows)) => {
            sf::replace_interlisted(c, &rows, &as_of, &now_iso()).map_err(|e| e.to_string())?;
            sf::read_answered(c, INTERLISTED, INTERLISTED_SOURCE).map_err(|e| e.to_string())?;
            crate::feeds::feed_answered(app, INTERLISTED);
        }
        Err(e) => {
            let why = e.failure().map(|e| format!("The TSX's list of interlisted companies could not be read: {e}"));
            failed_read(c, INTERLISTED, INTERLISTED_SOURCE, &why)?;
            feed_failed(app, INTERLISTED, why);
        }
    }
    Ok(())
}

/// The header's name for the interlisted list's failure, and the list's source.
pub(crate) const INTERLISTED: &str = "interlisted";
const INTERLISTED_SOURCE: &str = "tsx";

/// A listing's own filer settled again where it is a day old: the one its own
/// regulator lists it under, or why none settles.
fn settle_own(c: &Connection, key: &ListingKey, regulators: &dyn disclosures::Regulators) -> Result<(), String> {
    let Some(source) = own_regulator(key) else { return Ok(()) };
    let checked = sf::listing_filers_checked_at(c, key, source).map_err(|e| e.to_string())?;
    if age_hours(&checked).is_some_and(|h| h < FILERS_RECHECK_HOURS) {
        return Ok(());
    }
    // a regulator that could not be asked rests before it is asked again, its failure said meanwhile
    let subject = key.to_string();
    if let Some(f) = sf::read_failure(c, &subject, source.as_str()).map_err(|e| e.to_string())?.filter(|f| resting(Some(f))) {
        return Err(f.why);
    }
    let Some((_, found)) = regulators.filer(key, &today()) else { return Ok(()) };
    let now = now_iso();
    // a regulator that could not be asked leaves what was settled before standing
    let found = match found {
        Ok(found) => found,
        Err(e) => {
            let why = e.failure().map(|e| format!("{} could not be asked who files {}: {e}", source.as_str(), key.named()));
            failed_read(c, &subject, source.as_str(), &why)?;
            return Err(why.to_string());
        }
    };
    sf::read_answered(c, &subject, source.as_str()).map_err(|e| e.to_string())?;
    let by = if source == Regulator::Sec { sf::SettledBy::SecTicker } else { sf::SettledBy::SedarName };
    match found {
        Ok(id) => sf::settle_listing_filers(c, key, source, Ok(&[(id, by)]), &now),
        Err(why) => sf::settle_listing_filers(c, key, source, Err(&why), &now),
    }
    .map_err(|e| e.to_string())
}

/// The regulator of the other country, whose filer a listing's comes through the
/// interlisted list: none for a listing no regulator follows.
fn other_regulator(key: &ListingKey) -> Option<Regulator> {
    match own_regulator(key)? {
        Regulator::Sec => Some(Regulator::Sedar),
        Regulator::Sedar => Some(Regulator::Sec),
    }
}

/// The filers a listing's filings come from: its own regulator's, and where the
/// TSX's interlisted list pairs it with a listing in the other country, that
/// listing's own regulator's. Each settled again where it is a day old; a
/// regulator that cannot be asked leaves what was settled before. The other
/// country's filer is what the list pairs it with at each settle, or none.
pub fn settled_filers(app: &Arc<App>, c: &Connection, key: &ListingKey, regulators: &dyn disclosures::Regulators) -> Result<Vec<sf::Filer>, String> {
    let mut failed: Vec<String> = Vec::new();
    if let Err(e) = settle_own(c, key, regulators) {
        failed.push(e);
    }
    interlisted_current(app, c, regulators)?;
    let partner = sf::interlisted_partner(c, key).map_err(|e| e.to_string())?.filter(|p| own_regulator(p).is_some() && own_regulator(p) == other_regulator(key));
    match (partner, other_regulator(key)) {
        (Some(partner), Some(source)) => {
            let partner_failed = settle_own(c, &partner, regulators).err();
            // the partner's own filer is this listing's under the partner's regulator
            let theirs: Vec<(String, sf::SettledBy)> = sf::listing_filers(c, &partner)
                .map_err(|e| e.to_string())?
                .into_iter()
                .filter(|(f, _)| f.source == source)
                .map(|(f, _)| (f.id, sf::SettledBy::Interlisted))
                .collect();
            let now = now_iso();
            let why = sf::listing_filer_miss(c, &partner, source).map_err(|e| e.to_string())?.map(|(w, _)| w).unwrap_or_default();
            if !theirs.is_empty() {
                sf::settle_listing_filers(c, key, source, Ok(&theirs), &now).map_err(|e| e.to_string())?;
            } else if !why.is_empty() {
                sf::settle_listing_filers(c, key, source, Err(&why), &now).map_err(|e| e.to_string())?;
            } else if partner_failed.is_none() {
                sf::settle_listing_filers(c, key, source, Ok(&[]), &now).map_err(|e| e.to_string())?;
            }
            failed.extend(partner_failed);
        }
        // the list pairs it with none: no filer of the other country's is its
        (None, Some(source)) => sf::settle_listing_filers(c, key, source, Ok(&[]), &now_iso()).map_err(|e| e.to_string())?,
        (_, None) => {}
    }
    let filers: Vec<sf::Filer> = sf::listing_filers(c, key).map_err(|e| e.to_string())?.into_iter().map(|(f, _)| f).collect();
    if filers.is_empty() && !failed.is_empty() {
        return Err(failed.join(" "));
    }
    Ok(filers)
}

/// The filers a listing's filings come from as last settled, asking no one.
pub fn filers_kept(c: &Connection, key: &ListingKey) -> Result<Vec<sf::Filer>, String> {
    Ok(sf::listing_filers(c, key).map_err(|e| e.to_string())?.into_iter().map(|(f, _)| f).collect())
}

/// Whether a listing's filings are due a read: its filers never settled, or a
/// filer's documents read longer ago than `hours` (`FILINGS_STALE_HOURS` unsaid).
pub fn filings_stale(c: &Connection, key: &ListingKey, hours: Option<f64>) -> Result<bool, String> {
    let limit = hours.unwrap_or(FILINGS_STALE_HOURS);
    let Some(own) = own_regulator(key) else { return Ok(false) };
    if sf::listing_filers_checked_at(c, key, own).map_err(|e| e.to_string())?.is_empty() {
        return Ok(true);
    }
    for f in filers_kept(c, key)? {
        if age_hours(&sf::filer_fetched_at(c, &f).map_err(|e| e.to_string())?).is_none_or(|h| h > limit) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// When a listing's filings were last read: the oldest of its filers' reads;
/// empty when one was never read.
fn fetched_at(c: &Connection, key: &ListingKey) -> Result<String, String> {
    let mut out: Option<String> = None;
    for f in filers_kept(c, key)? {
        let when = sf::filer_fetched_at(c, &f).map_err(|e| e.to_string())?;
        if when.is_empty() {
            return Ok(String::new());
        }
        out = Some(out.map_or(when.clone(), |o: String| o.min(when)));
    }
    Ok(out.unwrap_or_default())
}

fn can_name_documents() -> bool {
    if enrich::summary_available() {
        return true;
    }
    let status = enrich::summary_status();
    if localmodel::COMING_UP.contains(&status) {
        return enrich::wait_for_summary(enrich::SUMMARY_WAIT_SEC);
    }
    status != "downloading"
}

/// A bounded hold on what cannot be named yet.
fn naming_held(c: &Connection, holding: bool) -> Result<bool, String> {
    if !holding {
        set_meta(c, FILINGS_HOLD_KEY, "").map_err(|e| e.to_string())?;
        return Ok(false);
    }
    let since = get_meta(c, FILINGS_HOLD_KEY, "").map_err(|e| e.to_string())?;
    if since.is_empty() {
        set_meta(c, FILINGS_HOLD_KEY, &now_iso()).map_err(|e| e.to_string())?;
        return Ok(true);
    }
    Ok(match parse_instant(&since) {
        Some(then) => now_unix() - then <= FILINGS_HOLD_MAX_MIN * 60.0,
        None => true,
    })
}

/// What makes a filing itself.
pub fn filing_mark(r: &Filing) -> [String; 5] {
    [r.doc.source.as_str().to_string(), r.doc.date.clone(), r.doc.form.clone(), r.doc.title.clone(), r.doc.size.clone()]
}

/// A listing followed for filings: its key, and what it is called and where, as
/// the book or the watchlist gives it, for the notice and the feed to show.
#[derive(Clone, Debug)]
pub struct FilingListing {
    pub key: ListingKey,
    pub symbol: String,
    pub exchange: String,
}

/// The listings to watch for filings, once each by their key: the held, those
/// traded in the past year (`all`), the watched. A contract stands for what it is
/// written on; a coin, an index, a listing that names no venue and one no
/// regulator here covers are left out (brief 34, U10). The book's listings not
/// read is an error.
pub fn known_filing_listings(app: &Arc<App>, scopes: &[String]) -> Result<Vec<FilingListing>, String> {
    let has = |k: &str| scopes.iter().any(|x| x == k);
    // (symbol, exchange, kind)
    let mut rows: Vec<(String, String, String)> = Vec::new();
    let b = if has("held") || has("all") || has("watched") { base(app)? } else { None };
    if let Some(b) = &b {
        if has("held") || has("all") {
            rows.extend(bagholder_model::symbols_of::held_symbols(b).into_iter().map(|l| (l.symbol, l.exchange, l.kind)));
        }
        if has("all") {
            for t in b.traded.iter() {
                rows.push((t.symbol.clone(), t.exchange.clone(), t.kind.to_string()));
            }
        }
        if has("watched") || has("all") {
            rows.extend(b.watchlist.iter().map(|w| (w.symbol.clone(), w.exchange.clone(), String::new())));
        }
    }
    let mut out = Vec::new();
    let mut seen: HashSet<ListingKey> = HashSet::new();
    for (symbol, exchange, kind) in rows {
        let symbol = if kind == "Options" {
            let under = bagholder_model::symbols::underlying_symbol(&symbol);
            if under.is_empty() || under == "—" {
                continue;
            }
            under
        } else {
            symbol
        };
        if kind == "Crypto" {
            continue;
        }
        let Ok(key) = ListingKey::on(&symbol, &exchange) else { continue };
        if own_regulator(&key).is_none() || !seen.insert(key.clone()) {
            continue;
        }
        out.push(FilingListing { symbol: key.symbol().to_string(), key, exchange });
    }
    Ok(out)
}

/// While a Disclosures or Releases set is on, each
/// chosen ticker not read within half an hour is read again and what is new is
/// told. Returns how many tickers had something new; a read or a write of the
/// store that fails fails the sweep, which is said in the header until the
/// next one succeeds.
pub fn sweep_filings(app: &Arc<App>) -> usize {
    // said in the header by `conn` and `notices`
    let (Ok(c), Ok(book)) = (conn(app), notices(app)) else { return 0 };
    went(app, DISCLOSURES, sweep_filings_in(app, &c, &book, &|key| refresh_filings(app, key)).map_err(|e| format!("The disclosures could not be swept: {e}"))).unwrap_or(0)
}

/// The sweep, the disclosures read and kept in the market cache (`c`), what is
/// told of them and the marks of what was met kept in the book. Each filer's
/// documents are one stream (`filings:<source>:<filer>`), so a filing two listings
/// of one company share is told once. `refresh` reads a listing's filings again.
fn sweep_filings_in(app: &Arc<App>, c: &Connection, book: &bagholder_book::Book, refresh: &dyn Fn(&ListingKey) -> Result<i64, String>) -> Result<usize, String> {
    let scopes = notify::disclosure_scopes(book).map_err(|e| e.to_string())?;
    let rel_scopes = notify::release_scopes(book).map_err(|e| e.to_string())?;
    if scopes.is_empty() && rel_scopes.is_empty() {
        return Ok(0);
    }
    let mut told = 0;
    let disc: HashSet<ListingKey> = if scopes.is_empty() { HashSet::new() } else { known_filing_listings(app, &scopes)?.into_iter().map(|l| l.key).collect() };
    let hold = !disc.is_empty() && naming_held(c, !can_name_documents())?;
    let mut both: Vec<String> = scopes.clone();
    for r in &rel_scopes {
        if !both.contains(r) {
            both.push(r.clone());
        }
    }
    for inst in known_filing_listings(app, &both)? {
        let key = inst.key.clone();
        let sym = inst.symbol.clone();
        if !filings_stale(c, &key, Some(FILINGS_SWEEP_AGE_MIN / 60.0))? {
            continue;
        }
        if hold && disc.contains(&key) {
            continue;
        }
        let before: HashSet<[String; 5]> = sf::filings_of(c, &filers_kept(c, &key)?).map_err(|e| e.to_string())?.iter().map(filing_mark).collect();
        let wrote = refresh(&key)?;
        if wrote < 0 {
            continue;
        }
        // what each filer shows now, read first; its marks are kept below with what is told of it
        let mut new: Vec<Filing> = Vec::new();
        let mut marks: Vec<(String, Vec<String>, Option<(String, String)>)> = Vec::new();
        for filer in filers_kept(c, &key)? {
            let rows = sf::filings_of(c, std::slice::from_ref(&filer)).map_err(|e| e.to_string())?;
            let scope = format!("filings:{}:{}", filer.source.as_str(), filer.id);
            let events: Vec<String> = rows.iter().map(|r| filing_mark(r).join("|")).collect();
            let met = sf::events_told(book.notices(), &scope, &events).map_err(|e| e.to_string())?;
            let fresh = notify::fresh_in(book, &scope, &rows, |r| r.doc.date.clone(), |r| filing_mark(r).join("|"), |r| {
                before.contains(&filing_mark(r)) || met.contains(&filing_mark(r).join("|"))
            }).map_err(|e| e.to_string())?;
            new.extend(fresh.items);
            marks.push((scope, events, fresh.mark));
        }
        let rel: Vec<Filing> = new.iter().filter(|r| is_news_release(r)).cloned().collect();
        let rest: Vec<Filing> = new.iter().filter(|r| !is_news_release(r)).cloned().collect();
        let mut notices: Vec<(&str, String, String, String, sf::NotificationExtra)> = Vec::new();
        if !rel.is_empty() && in_release_scope(app, &key, &rel_scopes)? && !sf::has_wire_release(c, &key).map_err(|e| e.to_string())? {
            let (t, bd) = release_notice(app, &key, &rel);
            notices.push(("releases", release_key(&key, &rel), t, bd, notice_extra(&key, &inst.exchange, &rel)));
        }
        if !rest.is_empty() && disc.contains(&key) {
            let (t, bd) = filings_notice(app, &sym, &rest);
            let mut marked: Vec<String> = rest.iter().map(|r| filing_mark(r).join("/")).collect();
            marked.sort();
            let digest = sha1_hex12(&marked.join("|"));
            notices.push(("disclosures", format!("filings:{key}:{digest}"), t, bd, notice_extra(&key, &inst.exchange, &rest)));
        }
        // the marks and the notices kept together: a mark not kept would tell these
        // again, a notice not kept would never tell them; either fails the sweep and
        // leaves both as they were
        let now = now_iso();
        let rows = bagholder_sqlite::atomically(book.notices(), || {
            for (scope, events, mark) in &marks {
                notify::keep_mark(book, mark)?;
                sf::mark_told(book.notices(), scope, events, &now)?;
            }
            let mut rows = vec![];
            for (kind, key, t, bd, extra) in &notices {
                rows.extend(notify::record(book, kind, key, t, bd, Some(extra.clone()))?);
            }
            Ok(rows)
        })
        .map_err(|e| format!("what is new in {}'s disclosures could not be told: {e}", key.named()))?;
        for row in &rows {
            notify::announce(app, row);
        }
        if !rows.is_empty() {
            told += 1;
        }
    }
    Ok(told)
}

fn feed_scope(key: &str) -> Vec<String> {
    vec![match key {
        "holdings" => "held",
        "watchlist" => "watched",
        _ => "all",
    }
    .to_string()]
}

/// One filing as a feed across listings carries it: the filing itself, with
/// which listing it belongs to.
#[derive(Clone, Debug, Serialize, TS, Diff)]
#[serde(rename_all = "camelCase")]
#[diff(key = id)]
pub struct FeedFiling {
    #[serde(flatten)]
    #[ts(flatten)]
    pub filing: Filing,
    pub symbol: String,
    pub exchange: String,
    /// The listing it belongs to (`SYMBOL@VENUE`): what its symbol opens.
    pub listing: String,
}

/// The newest disclosures across a set of listings, as a page shows them.
#[derive(Clone, Debug, Serialize, TS, Diff)]
#[serde(rename_all = "camelCase")]
pub struct FilingsFeed {
    #[ts(type = "true")]
    pub ok: bool,
    pub scope: String,
    pub filings: Vec<FeedFiling>,
    pub reading: bool,
}

/// The stored disclosures of every ticker in a set,
/// newest first. The store failing is said in the header until it answers.
pub fn filings_feed(app: &Arc<App>, scope: &str, limit: i64) -> FilingsFeed {
    let key = { let k = scope.trim().to_lowercase(); if k.is_empty() { "all".to_string() } else { k } };
    let read = || -> Result<Vec<FeedFiling>, String> {
        let c = conn(app)?;
        let mut rows: Vec<FeedFiling> = Vec::new();
        // a document two listings of one company share is shown once, under the first
        let mut seen: HashSet<(Regulator, String)> = HashSet::new();
        for inst in known_filing_listings(app, &feed_scope(&key))? {
            for r in fresh_filings(&c, &filers_kept(&c, &inst.key)?)? {
                if seen.insert((r.doc.source, r.doc.id.clone())) {
                    rows.push(FeedFiling { filing: r, symbol: inst.symbol.clone(), exchange: inst.exchange.clone(), listing: inst.key.to_string() });
                }
            }
        }
        Ok(rows)
    };
    let Some(mut rows) = went(app, DISCLOSURES_STORED, read().map_err(|e| format!("The disclosures held could not be read: {e}"))) else {
        return FilingsFeed { ok: true, scope: key, filings: vec![], reading: false };
    };
    rows.sort_by(|a, b| b.filing.doc.date.cmp(&a.filing.doc.date));
    rows.truncate(limit.max(1) as usize);
    FilingsFeed { ok: true, scope: key, filings: rows, reading: enrich::summary_available() }
}

/// The issuers' own news releases as they filed them, for the News card's
/// Releases tab: every listing's in `scope` (`all`, `holdings`, `watchlist`), or
/// the chip's listing's alone. Read as stored, a named document's title as its
/// name gives it, and nothing written: a title is read by the disclosures'
/// own reading.
pub fn filed_releases(app: &Arc<App>, scope: &str, chip: Option<(&ListingKey, &str)>) -> Result<Vec<crate::wire::news::FiledRelease>, String> {
    let c = conn(app)?;
    let listings: Vec<(ListingKey, String)> = match chip {
        Some((key, exchange)) => vec![(key.clone(), exchange.to_string())],
        None => known_filing_listings(app, &feed_scope(scope))?.into_iter().map(|f| (f.key, f.exchange)).collect(),
    };
    let reading = enrich::summary_available();
    let mut out = Vec::new();
    for (key, exchange) in listings {
        let symbol = key.symbol().to_string();
        for mut r in sf::filings_of(&c, &filers_kept(&c, &key)?).map_err(|e| e.to_string())? {
            if !is_news_release(&r) {
                continue;
            }
            if r.enrich_version.unwrap_or(0) < ENRICH_VERSION {
                r.subject = String::new();
            }
            if r.subject.is_empty() {
                if let Some(t) = disclosures::quick_title(&r.doc) {
                    r.subject = t;
                }
            }
            out.push(crate::wire::news::FiledRelease {
                pending: r.subject.is_empty() && !r.enrich_final && reading,
                id: r.doc.id.clone(),
                symbol: symbol.clone(),
                exchange: exchange.clone(),
                subject: r.subject.clone(),
                form: r.doc.form.clone(),
                source: r.doc.source.as_str().to_string(),
                url: r.doc.url.clone(),
                date: r.doc.date.clone(),
            });
        }
    }
    Ok(out)
}

/// A filed document that is the company's own
/// press release.
pub fn is_news_release(filing: &Filing) -> bool {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| regex::Regex::new(r"(?i)news release|press release").unwrap()).is_match(&filing.doc.form)
}

/// Whether a release of the listing `key` names is one the person chose to be
/// told of: every listing's, the held listings', or the watched ones', each by its
/// own key, never another listing's with the same ticker (brief 34, U12).
pub fn in_release_scope(app: &Arc<App>, key: &ListingKey, scopes: &[String]) -> Result<bool, String> {
    if scopes.is_empty() {
        return Ok(false);
    }
    if scopes.iter().any(|x| x == "all") {
        return Ok(true);
    }
    let same = |symbol: &str, exchange: &str| ListingKey::on(symbol, exchange).is_ok_and(|k| k == *key);
    let Some(b) = base(app)? else { return Ok(false) };
    if scopes.iter().any(|x| x == "held") && b.positions.iter().any(|p| same(&p.symbol, &p.exchange)) {
        return Ok(true);
    }
    if scopes.iter().any(|x| x == "watched") {
        return Ok(b.watchlist.iter().any(|w| same(&w.symbol, &w.exchange)));
    }
    Ok(false)
}

/// What a notice is told of: a wire's item or a filed document.
pub trait Notable: Clone {
    fn id(&self) -> String;
    /// What it says it is: a headline, else a filing's subject, else its type.
    fn heading(&self) -> String;
    /// When its source dates it.
    fn moment(&self) -> String;
    fn summary(&self) -> String;
    fn url(&self) -> String;
    /// The regulator a filed document came from ("SEDAR+", "SEC", "SEC EDGAR"), opened through the app.
    fn filed_source(&self) -> Option<String>;
    fn is_release(&self) -> bool;
}

impl Notable for NewsItem {
    fn id(&self) -> String {
        self.id.clone()
    }
    fn heading(&self) -> String {
        self.headline.clone()
    }
    fn moment(&self) -> String {
        self.published_at.clone()
    }
    fn summary(&self) -> String {
        self.summary.clone()
    }
    fn url(&self) -> String {
        self.url.clone()
    }
    fn filed_source(&self) -> Option<String> {
        None
    }
    fn is_release(&self) -> bool {
        self.kind == sf::NewsKind::Release
    }
}

impl Notable for Filing {
    fn id(&self) -> String {
        self.doc.id.clone()
    }
    fn heading(&self) -> String {
        // a filing has no headline
        if self.subject.is_empty() { self.doc.form.clone() } else { self.subject.clone() }
    }
    fn moment(&self) -> String {
        self.doc.date.clone()
    }
    fn summary(&self) -> String {
        self.summary.clone()
    }
    fn url(&self) -> String {
        self.doc.url.clone()
    }
    fn filed_source(&self) -> Option<String> {
        Some(self.doc.source.as_str().to_string())
    }
    fn is_release(&self) -> bool {
        is_news_release(self)
    }
}

pub fn release_notice<T: Notable>(app: &Arc<App>, key: &ListingKey, rows: &[T]) -> (String, String) {
    let sym = key.symbol();
    let first = newest_of(rows);
    let mut head = first.heading();
    if head.is_empty() {
        head = "A new release.".into();
    }
    let title = if rows.len() == 1 { "Press release · ".to_string() } else { format!("{} press releases · ", rows.len()) } + sym;
    // A release announcing distributions carries the figures beneath the headline, since the
    // headline alone ("Announces August 2026 Distributions") says nothing a holder can act on.
    let mut detail = if is_distribution_release(&head) { distribution_detail(app, key, first) } else { String::new() };
    if detail.is_empty() {
        // what the source said beneath its own headline, where it said anything
        detail = first.summary().trim().to_string();
    }
    if !detail.is_empty() {
        head = format!("{}\n{}", head, detail);
    }
    (title, head)
}

/// A release whose subject is a distribution or a dividend.
pub fn is_distribution_release(headline: &str) -> bool {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| regex::Regex::new(r"(?i)\b(distribution|distributions|dividend|dividends)\b").unwrap()).is_match(headline)
}

/// `2026-08-31` as `Aug 31`, and a year that is not this one carries it.
pub fn stamp_day(iso: &str) -> String {
    let day = bagholder_model::dates::head10(iso);
    let (y, m, d) = match bagholder_model::dates::parse_iso(&day) { Some(x) => x, None => return String::new() };
    if !(1..=12).contains(&m) || d == 0 || d > bagholder_model::dates::days_in_month(y, m) {
        return String::new();
    }
    let text = format!("{} {}", bagholder_model::dates::MONTHS[(m - 1) as usize], d);
    let this_year: i64 = bagholder_model::clock::now_utc_stamp()[..4].parse().unwrap_or(0);
    if y == this_year { text } else { format!("{} {}", text, y) }
}

/// What a distribution release announces for this listing, read from the
/// release itself: the amount, when it goes ex and when it is paid, and the
/// amount before it from the declared record.
///
/// A headline says only that distributions were announced, and the fund's page
/// and the exchange's record list the new one days after the release (Harvest,
/// 2026-10-23: the release at 7:30, its pages and TMX's record still at
/// September's that evening). So the figures are the fund company's own where
/// its record, read again now, already lists a distribution going ex on or after
/// the release's day (a company reader that reads its releases, as Harvest's
/// does); else the release's text, as the feed that carried it keeps it; else
/// the exchange's record where that lists it. Otherwise the headline alone, and
/// a release that could not be read says why in the header, until a later one
/// can.
pub fn distribution_detail<T: Notable>(app: &Arc<App>, key: &ListingKey, release: &T) -> String {
    let Some(f) = app.figures.get() else { return String::new() };
    // the record is read again for the amount before, so it is the one just paid
    let detail = read_record_for_notice(app, f, key)
        .map_err(|e| format!("{}'s declared distributions could not be read again: {e}", key.named()))
        .and_then(|read| {
            let book = f.book().map_err(|e| format!("The book could not be read for {}'s distributions: {e}", key.named()))?;
            // the release's text is asked for only where the company's own record lacks it
            let text = || release_text(app, release).map_err(|e| format!("{}'s distribution release could not be read: {e}", key.named()));
            distribution_detail_in(&book, &read, text, key.symbol(), &release.moment())
        });
    went(app, "distributions", detail).unwrap_or_default()
}

/// A release's whole text: TMX's copy for an item TMX listed, else the page its
/// link opens.
#[cfg(not(test))]
fn release_text<T: Notable>(_app: &Arc<App>, release: &T) -> Result<String, String> {
    if let Some(newsid) = release.id().strip_prefix("tmx:") {
        return bagholder_market::news::tmx_story(&bagholder_market::news::LIVE, newsid).map_err(|e| e.text);
    }
    // a Google item's link stands for the publisher's page
    let mut url = release.url();
    if url.contains("news.google.com/") {
        url = bagholder_market::news::read_article(&url).map_err(|e| e.text)?.url;
    }
    bagholder_market::http::get_text(&url, &[("User-Agent", "Mozilla/5.0")]).map_err(|e| bagholder_market::http::describe_failure(&e))
}

/// A test's release carries the text it was given.
#[cfg(test)]
fn release_text<T: Notable>(app: &Arc<App>, release: &T) -> Result<String, String> {
    app.feeds.release_texts.lock().unwrap().get(&release.id()).cloned().ok_or_else(|| format!("no text is recorded for {}", release.id()))
}

/// The payers on the listing `key` names, their records read again.
#[cfg(not(test))]
fn read_record_for_notice(app: &Arc<App>, f: &crate::figures::Figures, key: &ListingKey) -> Result<Vec<bagholder_core::InstrumentId>, String> {
    crate::due::read_payer_now(app, f, key, bagholder_core::jiff::Timestamp::now())
}

/// A test counts the reads rather than reaching the source, and finds the
/// instruments by the listing the book calls them.
#[cfg(test)]
fn read_record_for_notice(app: &Arc<App>, f: &crate::figures::Figures, key: &ListingKey) -> Result<Vec<bagholder_core::InstrumentId>, String> {
    app.feeds.record_reads.fetch_add(1, Ordering::SeqCst);
    if let Some(why) = app.feeds.record_fails.lock().unwrap().clone() {
        return Err(why);
    }
    let book = f.book()?;
    let mut out = vec![];
    for i in book.instruments().map_err(|e| e.to_string())? {
        if book.names(i.id).map_err(|e| e.to_string())?.last().is_some_and(|n| ListingKey::of(i.kind, n).is_ok_and(|k| k == *key)) {
            out.push(i.id);
        }
    }
    Ok(out)
}

/// A distribution a unit, exactly as the record states it, never fewer than cents.
fn per_unit(m: &bagholder_core::Money) -> String {
    let text = m.amount.to_text();
    let (whole, cents) = text.split_once('.').unwrap_or((&text, ""));
    let sign = if m.currency.as_str() == "USD" { "US$" } else { "$" };
    format!("{sign}{whole}.{cents:0<2}")
}

/// How often a payer pays, in words.
fn frequency_word(per_year: u32) -> String {
    match per_year {
        52 => "weekly".into(),
        12 => "monthly".into(),
        4 => "quarterly".into(),
        2 => "semi-annual".into(),
        1 => "annual".into(),
        n => format!("{n} a year"),
    }
}

/// The line under a distribution release's headline, for `ticker`: the
/// distribution the release (its moment `released`) announces, from the fund
/// company's declared record where that lists one going ex on or after the
/// release's day, else from the release's text (`text`, asked for only then),
/// else from the exchange's record beside the company's; its frequency from the
/// record (the instruments given, the one listing a symbol names), and the
/// amount before it, the company's where it states one. Empty where none has
/// it, unless the release's text could not be had or read, which is the error.
pub fn distribution_detail_in(book: &bagholder_book::Book, instruments: &[bagholder_core::InstrumentId], text: impl FnOnce() -> Result<String, String>, ticker: &str, released: &str) -> Result<String, String> {
    let (declared, frequencies) = match (book.declared(), book.frequencies()) {
        (Ok(d), Ok(f)) => (d, f),
        (Err(e), _) | (_, Err(e)) => return Err(format!("The declared distributions could not be read: {e}")),
    };
    let id = match instruments {
        [id] => Some(*id),
        _ => None,
    };
    let read = id.and_then(|id| declared.get(&id));
    let newest_first = |rows: &mut Vec<&bagholder_book::facts::DeclaredRow>| rows.sort_by(|x, y| y.ex_date.cmp(&x.ex_date));
    let mut company: Vec<&bagholder_book::facts::DeclaredRow> = read.iter().flat_map(|r| r.items.iter()).collect();
    let mut market: Vec<&bagholder_book::facts::DeclaredRow> = read.iter().flat_map(|r| r.market.iter().flat_map(|m| m.items.iter())).collect();
    newest_first(&mut company);
    newest_first(&mut market);
    let day = bagholder_model::dates::head10(released).parse::<bagholder_core::jiff::civil::Date>().ok();
    let new = |r: &&&bagholder_book::facts::DeclaredRow| day.is_some_and(|d| r.ex_date >= d);
    let (cash, ex, pay) = if let Some(r) = company.first().filter(new) {
        (r.amount.amount, r.ex_date, r.pay_date)
    } else {
        let html = text();
        match html.as_deref().map(|h| bagholder_sources::payers::announced::announced(h, ticker)) {
            Ok(Ok(Some(a))) => (a.cash, a.ex_date, a.pay_date),
            Ok(Err(why)) => return Err(format!("the release: {why}")),
            _ => match market.first().filter(new) {
                Some(r) => (r.amount.amount, r.ex_date, r.pay_date),
                None => return html.map(|_| String::new()),
            },
        }
    };
    // the amount before it: the company's, else the exchange's
    let was = company.iter().find(|r| r.ex_date < ex).or_else(|| market.iter().find(|r| r.ex_date < ex));
    // the fund pays in the currency its record states; a .U listing in US dollars
    let currency = company.first().or(market.first()).map(|r| r.amount.currency).unwrap_or(if ticker.ends_with(".U") { bagholder_core::Currency::USD } else { bagholder_core::Currency::CAD });
    let mut out = format!("{} a share", per_unit(&bagholder_core::Money::new(cash, currency)));
    if let Some(f) = id.and_then(|id| frequencies.get(&id)) {
        out += &format!(", {}", frequency_word(f.per_year));
    }
    out += &format!(" · ex-div {}", stamp_day(&ex.to_string()));
    if let Some(p) = pay {
        out += &format!(" · pays {}", stamp_day(&p.to_string()));
    }
    if let Some(was) = was {
        out += &format!(" · was {}", per_unit(&was.amount));
    }
    Ok(out)
}

/// When the newest of these happened, as its source dates it.
///
/// A release found today can have been published weeks ago -- the app reads a
/// listing's back catalogue the first time it sees it -- and a notice that
/// shows only when it was told reads as news that is not new.
pub fn notice_moment<T: Notable>(rows: &[T]) -> String {
    newest_of(rows).moment()
}

/// Where a notification's rows can be read: the newest one's own page, and,
/// for a filed document, the regulator it opens through.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NoticeLink {
    pub url: String,
    pub doc: String,
    pub source: String,
}

/// Where a notification's rows can be read: the newest one's own page.
///
/// A filed document is opened through the app, which is what the Disclosures
/// table does, so it opens the same way from here; anything else carries the
/// source's own link.
pub fn notice_link<T: Notable>(rows: &[T]) -> NoticeLink {
    let newest = newest_of(rows);
    let url = newest.url();
    let id = newest.id();
    if let Some(source) = newest.filed_source() {
        if !id.is_empty() {
            return NoticeLink { url, doc: id, source };
        }
    }
    NoticeLink { url, ..NoticeLink::default() }
}

/// The newest row by its own moment.
fn newest_of<T: Notable>(rows: &[T]) -> &T {
    let mut sorted: Vec<&T> = rows.iter().collect();
    sorted.sort_by(|a, b| b.moment().cmp(&a.moment()));
    sorted[0]
}

/// A notification's extra: the symbol, the moment and the link, in one struct.
fn notice_extra<T: Notable>(key: &ListingKey, exchange: &str, rows: &[T]) -> bagholder_market::stored::feeds::NotificationExtra {
    let (at, link) = (notice_moment(rows), notice_link(rows));
    bagholder_market::stored::feeds::NotificationExtra {
        symbol: key.symbol().to_string(),
        listing: key.to_string(),
        exchange: exchange.to_string(),
        at,
        url: link.url,
        doc: link.doc,
        source: link.source,
    }
}

/// What a release *is*, independent of the id, the source and the date each
/// carries: the story its headline tells.
///
/// TMX, Yahoo, Seeking Alpha and Google all carry the same release under their
/// own ids, a week apart in their own timestamps; keyed by id, one release is
/// four events.
pub fn release_event<T: Notable>(row: &T) -> String {
    news::news_text(&row.heading())
}

fn release_key<T: Notable>(key: &ListingKey, rows: &[T]) -> String {
    let mut ids: Vec<String> = rows.iter().map(release_event).collect();
    ids.sort();
    format!("release:{key}:{}", sha1_hex12(&ids.join("|")))
}

/// The press releases a wire answered with that
/// are newer than any it showed for the listing.
///
/// A read or a write of the book that fails is the error: a mark not kept
/// would tell the same release twice.
pub fn note_wire_releases(app: &Arc<App>, symbol: &str, exchange: &str, rows: &[NewsItem], new_ids: &[String]) -> Result<(), String> {
    let book = notify::book(app).map_err(|e| e.to_string())?;
    let c = &*book;
    let scopes = notify::release_scopes(c).map_err(|e| e.to_string())?;
    // a listing that names no venue was read under none
    let Some(key) = news::news_listing(symbol, exchange) else { return Ok(()) };
    if scopes.is_empty() || !in_release_scope(app, &key, &scopes)? {
        return Ok(());
    }
    let rel: Vec<NewsItem> = rows.iter().filter(|r| r.is_release()).cloned().collect();
    // An event is told once. The stream keeps what it has met, by what the thing is rather than by
    // the id a source gave it, so the same release reaching the app again -- from another source,
    // under another id, dated a week apart, or simply returning to a search's results after
    // dropping out of them -- is recognised and passed over. Everything met is recorded, told or
    // not, so the back catalogue a first read brings can never ring later.
    let scope = format!("news:{key}");
    let events: Vec<String> = rel.iter().map(release_event).collect();
    let met = sf::events_told(c.notices(), &scope, &events).map_err(|e| e.to_string())?;
    let fresh = notify::fresh_in(c, &scope, &rel, |r: &NewsItem| r.published_at.clone(), |r: &NewsItem| r.id.clone(), |r: &NewsItem| {
        !new_ids.contains(&r.id) || met.contains(&release_event(r))
    }).map_err(|e| e.to_string())?;
    // the notice is worded first (it may read the issuer's record); then the marks and
    // the notice are kept together, so a failure of either leaves the release to tell again
    let notice = if fresh.items.is_empty() { None } else { Some((release_key(&key, &fresh.items), release_notice(app, &key, &fresh.items), notice_extra(&key, exchange, &fresh.items))) };
    let now = now_iso();
    let row = bagholder_sqlite::atomically(c.notices(), || {
        notify::keep_mark(c, &fresh.mark)?;
        sf::mark_told(c.notices(), &scope, &events, &now)?;
        match &notice {
            Some((key, (title, body), extra)) => notify::record(c, "releases", key, title, body, Some(extra.clone())),
            None => Ok(None),
        }
    })
    .map_err(|e| format!("what is new of {} could not be told: {e}", key.named()))?;
    if let Some(row) = &row {
        notify::announce(app, row);
    }
    Ok(())
}

/// `note_wire_releases` from the news pass: a failure is said in the header until
/// a wire's releases are next noted.
pub(crate) fn note_wire_releases_said(app: &Arc<App>, symbol: &str, exchange: &str, rows: &[NewsItem], new_ids: &[String]) {
    went(app, RELEASES, note_wire_releases(app, symbol, exchange, rows, new_ids).map_err(|e| format!("The press releases could not be told: {e}")));
}

/// `New disclosure · QNC` and what was filed.
pub fn filings_notice(app: &Arc<App>, sym: &str, new: &[Filing]) -> (String, String) {
    let (mut named, mut said): (Vec<String>, String) = (Vec::new(), String::new());
    for r in new.iter().take(3) {
        let mut title = r.subject.trim().to_string();
        let mut summary = r.summary.trim().to_string();
        if (title.is_empty() || summary.is_empty()) && !r.doc.id.is_empty() {
            if let Ok(read) = filings_enrich_result(app, r.doc.source, &r.doc.id) {
                if title.is_empty() {
                    title = read.subject.trim().to_string();
                }
                if summary.is_empty() {
                    summary = read.summary.trim().to_string();
                }
            }
        }
        if title.is_empty() {
            title = form_name(&r.doc.form);
        }
        if !title.is_empty() && !named.contains(&title) {
            named.push(title);
        }
        if said.is_empty() {
            said = summary;
        }
    }
    let mut sources: Vec<String> = Vec::new();
    for r in new {
        let src = r.doc.source.as_str().trim().to_string();
        if !src.is_empty() && !sources.contains(&src) {
            sources.push(src);
        }
    }
    let label = |x: &str| match x.to_lowercase().as_str() {
        "sedar" | "sedar+" => "SEDAR+".to_string(),
        "sec" | "sec edgar" => "SEC EDGAR".to_string(),
        _ => x.to_string(),
    };
    let tail = sources.iter().map(|x| label(x)).collect::<Vec<_>>().join(", ");
    let head = named.join(", ") + if new.len() > 3 { " and more" } else { "" };
    let mut body = head.clone() + if !head.is_empty() && !tail.is_empty() { " · " } else { "" } + &tail;
    if body.is_empty() {
        body = "A new filing.".into();
    }
    // the sentence the document itself yielded, under the line that names it: a form code and a
    // regulator say what arrived, never what it says
    if !said.is_empty() && news::news_text(&said) != news::news_text(&head) {
        body = format!("{}\n{}", body, said);
    }
    let title = if new.len() == 1 { "New disclosure · ".to_string() } else { format!("{} new disclosures · ", new.len()) } + sym;
    (title, body)
}

/// What a form is, for the forms whose code is all a row carries until a
/// document has been read.
///
/// A code names the form and not what happened, and "4" alone tells a holder
/// nothing at all.
pub const FORM_NAMES: &[(&str, &str)] = &[
    ("3", "Insider's first report (Form 3)"),
    ("4", "Insider transaction (Form 4)"),
    ("5", "Insider's annual report (Form 5)"),
    ("8-K", "Material event (8-K)"),
    ("6-K", "Foreign issuer report (6-K)"),
    ("10-K", "Annual report (10-K)"),
    ("10-Q", "Quarterly report (10-Q)"),
    ("144", "Notice of proposed sale (144)"),
    ("S-1", "Registration (S-1)"),
    ("SC 13D", "Beneficial ownership (13D)"),
    ("SC 13G", "Beneficial ownership (13G)"),
    ("DEF 14A", "Proxy statement (DEF 14A)"),
    ("424B5", "Prospectus supplement (424B5)"),
    ("FWP", "Free writing prospectus (FWP)"),
];

/// A form's code as words where the app knows the form, the code itself
/// otherwise.
pub fn form_name(code: &str) -> String {
    let key = code.trim().to_uppercase();
    FORM_NAMES.iter().find(|(c, _)| *c == key).map(|(_, n)| n.to_string()).unwrap_or_else(|| code.trim().to_string())
}

/// What a tab shows when a regulator would not serve a document.
///
/// SEDAR+ mints a document's address inside a live session and puts a bot gate
/// in front of it, so a refusal is ordinary and a retry often works; the page
/// says that in words, names the document, and retries on a click.
pub fn document_error_page(app: &Arc<App>, source: Regulator, doc_id: &str, why: &str) -> String {
    // the document's record names it; one that cannot be read is said beside the source's refusal
    let (row, why) = match conn(app).and_then(|c| sf::filing(&c, source, doc_id).map_err(|e| e.to_string())) {
        Ok(row) => (row, why.to_string()),
        Err(e) => (None, format!("{} The document's record could not be read: {e}", if why.is_empty() { "The source did not answer." } else { why })),
    };
    let why = why.as_str();
    let name = row.as_ref().map(|r| {
        let s = r.subject.trim();
        if !s.is_empty() { s.to_string() } else if !r.doc.title.trim().is_empty() { r.doc.title.trim().to_string() } else { r.doc.form.trim().to_string() }
    }).filter(|s| !s.is_empty()).unwrap_or_else(|| "This document".to_string());
    let again = format!("/api/filings/doc?source={}&id={}", url_quote(source.as_str()), url_quote(doc_id));
    let source = source.as_str();
    let when = row.as_ref().map(|r| {
        let t = r.doc.date_text.trim();
        if !t.is_empty() { t.to_string() } else { r.doc.date.chars().take(10).collect() }
    }).unwrap_or_default();
    format!(
        "<!doctype html><meta charset=utf-8><title>{name}</title>\
<style>:root{{color-scheme:dark light}}body{{margin:0;min-height:100vh;display:grid;place-items:center;\
background:#0e1118;color:#e8ecf3;font:400 14px/1.6 -apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif}}\
main{{max-width:34rem;padding:2rem}}h1{{font:600 16px/1.4 inherit;margin:0 0 .75rem}}p{{margin:0 0 .75rem;color:#aab3c2}}\
b{{color:#e8ecf3;font-weight:500}}a{{color:#7aa2f7}}</style>\
<main><h1>{source} would not serve this document just now</h1>\
<p><b>{name}</b>{when}</p>\
<p>{why}</p>\
<p>SEDAR+ builds a document&rsquo;s address inside a live session and puts a bot gate in front of it, so a \
refusal is ordinary rather than a sign the document is gone. <a href=\"{again}\">Try again</a>.</p></main>",
        name = html_escape(&name),
        source = html_escape(&source),
        when = html_escape(&if when.is_empty() { String::new() } else { format!(" · {}", when) }),
        why = html_escape(if why.is_empty() { "The source did not answer." } else { why }),
        again = html_escape(&again),
    )
}

/// `html.escape`: the five characters that must not read as markup.
fn html_escape(t: &str) -> String {
    t.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#x27;")
}

/// `urllib.parse.quote`: everything but the unreserved characters and `/`.
fn url_quote(t: &str) -> String {
    let mut out = String::new();
    for b in t.bytes() {
        if b.is_ascii_alphanumeric() || b"_.-~/".contains(&b) {
            out.push(b as char);
        } else {
            out += &format!("%{:02X}", b);
        }
    }
    out
}

pub fn filings_sweep_loop(app: Arc<App>) {
    // The regulators publish no feed to subscribe to, so telling someone of a new
    // filing means asking; but only while they have asked to be told. With no
    // Disclosures or Releases set on, this waits for one to be switched on.
    let wanted = || {
        notices(&app).is_ok_and(|b| {
            notice_settings(&app, notify::disclosure_scopes(&b).and_then(|d| Ok(!d.is_empty() || !notify::release_scopes(&b)?.is_empty()))).unwrap_or(false)
        })
    };
    while app.events.park_until(&app, wanted) {
        sweep_filings(&app);
        if app.wait(Duration::from_secs(FILINGS_SWEEP_EVERY_SEC)) {
            return;
        }
    }
}

/// How long the reader waits between documents, and after a pass that found
/// nothing left to read.
pub const READ_GAP_SEC: u64 = 2;

/// Documents the app reads on its own, newest first, for the listings it
/// follows: a title and a sentence cost a download and a reading each, and a
/// list is no use standing still while someone waits for them. One at a time,
/// paced, and only while a model is up to do the reading.
pub fn disclosure_read_loop(app: Arc<App>) {
    // Reads while there is something unread and a model to read it; otherwise waits
    // for one of the two to change -- a filing stored (`FILINGS_STORED`) or the
    // model coming up (`localmodel::on_change`) -- and consults nothing in between.
    use bagholder_market::enrich::{summary_ensure, summary_ready};
    loop {
        let stored = FILINGS_STORED.load(Ordering::SeqCst);
        summary_ensure();
        if !app.events.park_until(&app, || summary_ready() || FILINGS_STORED.load(Ordering::SeqCst) != stored) {
            return;
        }
        if !summary_ready() {
            continue; // more was stored and there is still no model: try bringing one up again
        }
        // a row a read moved nothing on (the fetch failed, or the model went away
        // under it) is passed over until the loop is woken again, so it neither
        // holds the others back nor is fetched over and over
        let mut passed: HashSet<(Regulator, String)> = HashSet::new();
        loop {
            match read_one_unread(&app, &mut passed) {
                Ok(None) => break,
                // a document that could not be read is said in the header until one is, and passed over
                Ok(Some(read)) => {
                    went(&app, DISCLOSURES_READING, read);
                }
                // the store failing is said the same way, and the loop waits for a change rather than asking again
                Err(e) => {
                    feed_failed(&app, DISCLOSURES_READING, format!("The documents could not be read: {e}"));
                    break;
                }
            }
            if app.wait(Duration::from_secs(READ_GAP_SEC)) {
                return;
            }
        }
        if !app.events.park_until(&app, || FILINGS_STORED.load(Ordering::SeqCst) != stored || !summary_ready()) {
            return;
        }
    }
}

/// Counts the times filings were stored: what the reading loop waits on.
static FILINGS_STORED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The newest stored filing a reading could still add to, read: the listing on
/// screen first, then everything else. False when there is none left to try.
fn read_one_unread(app: &Arc<App>, passed: &mut HashSet<(Regulator, String)>) -> Result<Option<Result<(), bagholder_net::Failure>>, String> {
    let open = app.feeds.looking_at.lock().unwrap().clone();
    if let Ok(key) = open.parse::<ListingKey>() {
        if let Some(read) = read_one_of(app, Some(&key), passed)? {
            return Ok(Some(read));
        }
    }
    read_one_of(app, None, passed)
}

/// One document of `only`, or of every followed listing when it is none, that a
/// reading could still add to (`wants_reading`): one never read, and one holding a
/// title without its sentence or the other way round. True when there was one to
/// try, with how its read went; one the read moved nothing on, or whose read
/// failed, is added to `passed`. The store failing is the error.
fn read_one_of(app: &Arc<App>, only: Option<&ListingKey>, passed: &mut HashSet<(Regulator, String)>) -> Result<Option<Result<(), bagholder_net::Failure>>, String> {
    let c = conn(app)?;
    let listings: Vec<ListingKey> = match only {
        Some(k) => vec![k.clone()],
        None => known_filing_listings(app, &["held".to_string(), "watched".to_string()])?.into_iter().map(|i| i.key).collect(),
    };
    let mut filers: Vec<sf::Filer> = Vec::new();
    for k in &listings {
        for f in filers_kept(&c, k)? {
            if !filers.contains(&f) {
                filers.push(f);
            }
        }
    }
    read_one_in(&c, &filers, passed, &LiveReaders)
}

/// `read_one_of` on one connection, for the filers given, with the readers given.
fn read_one_in(c: &Connection, filers: &[sf::Filer], passed: &mut HashSet<(Regulator, String)>, readers: &dyn Readers) -> Result<Option<Result<(), bagholder_net::Failure>>, String> {
    let best = sf::filings_of(c, filers)
        .map_err(|e| e.to_string())?
        .into_iter()
        .filter(|r| wants_reading(r) && !passed.contains(&(r.doc.source, r.doc.id.clone())))
        .max_by(|a, b| a.doc.date.cmp(&b.doc.date));
    let Some(before) = best else { return Ok(None) };
    let (source, id) = (before.doc.source, before.doc.id.clone());
    if let Err(e) = filings_enrich_in(c, source, &id, readers) {
        passed.insert((source, id.clone()));
        return Ok(Some(Err(e.map(|e| format!("The {} document {id} could not be read: {e}", source.as_str())))));
    }
    let after = sf::filing(c, source, &id).map_err(|e| e.to_string())?;
    let moved = after.as_ref().map_or(false, |a| {
        (&a.subject, &a.summary, a.enrich_version, a.enrich_final, a.enrich_reads)
            != (&before.subject, &before.summary, before.enrich_version, before.enrich_final, before.enrich_reads)
    });
    if !moved {
        passed.insert((source, id));
    }
    Ok(Some(Ok(())))
}

/// Two reads that could have answered bound a document's reading: what they
/// cannot produce, the document does not have, and the row settles with
/// whichever half it has (SPEC §4 Disclosures).
pub const ENRICH_READS: i64 = 2;

/// Whether a reading could still add to a stored row: it was never read under the
/// current logic, or it holds a title without its sentence (or the other way
/// round) and has not settled. What every reader of documents picks rows by.
pub fn wants_reading(r: &Filing) -> bool {
    r.enrich_version.unwrap_or(0) < ENRICH_VERSION || (!r.enrich_final && (r.subject.is_empty() || r.summary.is_empty()))
}

/// A listing's disclosures read: its filers settled, and each filer's documents
/// read and kept in place of what was kept for it. The total written, or -1 when
/// no regulator could be reached; the store failing is the error. One read per
/// listing at a time.
pub fn refresh_filings(app: &Arc<App>, key: &ListingKey) -> Result<i64, String> {
    app.single_flight(&format!("filings:{key}"), Ok(0), || {
        let c = conn(app)?;
        refresh_filings_in(app, &c, key, &disclosures::Live)
    })
}

/// `refresh_filings` on one connection with the regulators given.
pub fn refresh_filings_in(app: &Arc<App>, c: &Connection, key: &ListingKey, regulators: &dyn disclosures::Regulators) -> Result<i64, String> {
    let mut outcomes: BTreeMap<Regulator, SourceOutcome> = BTreeMap::new();
    let filers = match settled_filers(app, c, key, regulators) {
        Ok(f) => f,
        Err(e) => {
            // no regulator could say who files the listing
            if let Some(source) = own_regulator(key) {
                outcomes.insert(source, SourceOutcome { available: false, matched: false, filer: false, count: 0, error: e });
            }
            vec![]
        }
    };
    let mut total = 0i64;
    let now = now_iso();
    // the profiles SEDAR+ settled by the listing's own exact name
    let by_name: Vec<sf::Filer> = sf::listing_filers(c, key).map_err(|e| e.to_string())?.into_iter().filter(|(_, by)| *by == sf::SettledBy::SedarName).map(|(f, _)| f).collect();
    let since = bagholder_model::dates::shift_date(&today(), -bagholder_market::sedar::LISTED_FILING_WINDOW_DAYS);
    for filer in &filers {
        let stored = sf::filings_of(c, std::slice::from_ref(filer)).map_err(|e| e.to_string())?;
        let held = stored.len();
        let read = regulators.documents(filer, 200);
        // one exact match that has filed nothing within the window a listed issuer
        // files in is a dormant profile, never a listed issuer's: a miss, as a tie
        // narrowed to none is (`sedar::pick_profile`), judged from the newest of the
        // documents just read and those stored (an answer that came back empty
        // leaves the stored ones standing, and says nothing of the profile)
        if let Ok(rows) = &read {
            let newest = rows.iter().map(|r| r.date.as_str()).chain(stored.iter().map(|r| r.doc.date.as_str())).max();
            if by_name.contains(filer) && newest.is_none_or(|d| d < since.as_str()) {
                let why = format!("The one SEDAR+ issuer under {}'s name has filed nothing in the last {} days: it is no listed issuer's profile.", key.named(), bagholder_market::sedar::LISTED_FILING_WINDOW_DAYS);
                sf::settle_listing_filers(c, key, filer.source, Err(&why), &now).map_err(|e| e.to_string())?;
                outcomes.insert(filer.source, SourceOutcome { available: true, matched: false, filer: false, count: 0, error: why });
                continue;
            }
        }
        let outcome = match read {
            Ok(rows) if rows.is_empty() && held > 0 => {
                log(&format!("bagholder disclosures: {key}: {} answered empty for {}; the stored rows stand", filer.source.as_str(), filer.id));
                SourceOutcome { available: true, matched: true, filer: true, count: held, error: String::new() }
            }
            Ok(rows) => {
                let n = sf::replace_filings(c, filer, &rows, &now).map_err(|e| format!("{}'s {} filings could not be stored: {e}", key.named(), filer.source.as_str()))?;
                total += n as i64;
                SourceOutcome { available: true, matched: n > 0, filer: true, count: n, error: String::new() }
            }
            Err(e) => match e {
                disclosures::SourceError::Unavailable(m) => SourceOutcome { available: false, matched: false, filer: true, count: 0, error: m },
                disclosures::SourceError::Other(m) | disclosures::SourceError::NotListed(m) => SourceOutcome { available: true, matched: false, filer: true, count: 0, error: m },
                disclosures::SourceError::Offline => SourceOutcome { available: false, matched: false, filer: true, count: 0, error: bagholder_net::client::OFFLINE.into() },
            },
        };
        outcomes.insert(filer.source, outcome);
    }
    // a regulator whose filer could not be settled says why
    for source in [Regulator::Sedar, Regulator::Sec] {
        if outcomes.contains_key(&source) {
            continue;
        }
        if let Some((why, _)) = sf::listing_filer_miss(c, key, source).map_err(|e| e.to_string())? {
            outcomes.insert(source, SourceOutcome { available: true, matched: false, filer: false, count: 0, error: why });
        }
    }
    FILINGS_STORED.fetch_add(1, Ordering::SeqCst);
    app.events.signal(); // the reading loop has something to look at
    let reached = outcomes.values().any(|o| o.available);
    let said = serde_json::to_string(&outcomes).expect("each regulator's outcome, keyed by its name, always serializes");
    set_meta(c, &key.meta(bagholder_core::listing::meta::FILINGS_SOURCES, &[]), &said).map_err(|e| e.to_string())?;
    Ok(if reached || outcomes.is_empty() { total } else { -1 })
}

/// A source's outcome the last time a listing's disclosures were refreshed.
#[derive(Clone, Debug, Default, Serialize, TS, Diff)]
#[serde(rename_all = "camelCase")]
pub struct SourceStatus {
    pub available: bool,
    pub matched: bool,
    pub filer: bool,
    pub error: String,
}

/// `#[derive(Diff)]` compares a map field as the model's own `BTreeMap<String,
/// V>`, so a source's status lives under its regulator's own JSON key
/// (`"SEDAR+"`, `"SEC"`) rather than under `Regulator` itself.
fn source_status(c: &Connection, key: &ListingKey) -> Result<BTreeMap<String, SourceStatus>, String> {
    // none stored before a first refresh; stored text that does not parse is an error
    let raw = get_meta(c, &key.meta(bagholder_core::listing::meta::FILINGS_SOURCES, &[]), "").map_err(|e| e.to_string())?;
    let stored: BTreeMap<Regulator, SourceOutcome> = if raw.is_empty() {
        BTreeMap::new()
    } else {
        serde_json::from_str(&raw).map_err(|e| format!("what {}'s regulators last answered could not be read: {e}", key.named()))?
    };
    let have: HashSet<Regulator> = sf::filings_of(c, &filers_kept(c, key)?).map_err(|e| e.to_string())?.iter().map(|r| r.doc.source).collect();
    let mut out = BTreeMap::new();
    for (source, dep) in [(Regulator::Sedar, sedar::available()), (Regulator::Sec, edgar::available())] {
        let st = stored.get(&source);
        // "available" is whether the last read actually reached the source, not merely
        // whether its dependency is installed: a stored outcome that recorded the source
        // unavailable (an outage, a maintenance page) means it could not be reached even
        // with the dependency present. The error it left travels with it, so the card can
        // say the source is unavailable rather than assert the listing has no filer.
        let reached = st.map(|s| s.available).unwrap_or(dep);
        let error = st.map(|s| s.error.clone()).unwrap_or_default();
        out.insert(source.as_str().to_string(), SourceStatus {
            available: dep && reached,
            matched: have.contains(&source),
            filer: st.map(|s| s.filer).unwrap_or(false) || have.contains(&source),
            error,
        });
    }
    Ok(out)
}

/// The stored disclosures, refreshed first when
/// forced or stale.
/// `GET /api/filings`.
#[derive(Serialize, TS)]
#[serde(untagged)]
pub enum FilingsAnswer {
    Ok(FilingsPayload),
    Refused(OkOr),
}

pub fn filings_payload(app: &Arc<App>, key: &ListingKey, refresh: bool) -> FilingsAnswer {
    let c = match conn(app) {
        Ok(c) => c,
        Err(e) => return FilingsAnswer::Refused(OkOr::err(e)),
    };
    match filings_payload_in(app, &c, key, refresh, &|| refresh_filings(app, key)) {
        Ok(p) => FilingsAnswer::Ok(p),
        Err(e) => FilingsAnswer::Refused(OkOr::err(e)),
    }
}

// --- a listing's disclosures while a page shows them ------------------------------
//
// The page used to drive the reading itself: ask for one document's title, wait, ask
// for the next, and re-kick the whole pass on timers of a second and a half to
// fifteen while the local model was coming up. The reading is the server's work. A
// page says it is showing a listing's disclosures (`docs`); the server brings the
// list up to date, then reads each document that has no title yet, newest first, for
// as long as some page is still showing them. Each title and sentence is committed
// as it is read and reaches its row as a change to that row.

fn set_reading(app: &Arc<App>, key: &ListingKey, ids: Vec<String>) {
    let changed = {
        let mut r = app.feeds.reading_now.lock().unwrap_or_else(|e| e.into_inner());
        let was = if ids.is_empty() { r.remove(&key.to_string()) } else { r.insert(key.to_string(), ids.clone()) };
        was.unwrap_or_default() != ids
    };
    // the rows still to be read wear the shimmer; said again only when they change
    if changed {
        app.events.signal();
    }
}

/// What `filings_stored` sends a page showing a listing's disclosures.
#[derive(Clone, Debug, Serialize, TS, Diff)]
#[serde(rename_all = "camelCase")]
pub struct FilingsDoc {
    #[ts(type = "true")]
    pub ok: bool,
    pub symbol: String,
    /// The listing (`SYMBOL@VENUE`).
    pub listing: String,
    pub available: bool,
    pub sources: BTreeMap<String, SourceStatus>,
    pub categories: Vec<String>,
    pub fetched_at: String,
    pub ever_read: bool,
    pub summary_status: String,
    pub reading: Vec<String>,
    pub filings: Vec<Filing>,
}

/// The disclosures as stored, never waiting on a source: what a page showing them
/// is sent. `reading` names the documents the pass has still to read, the first of
/// them the one being read now.
pub fn filings_stored(app: &Arc<App>, key: &ListingKey) -> Result<FilingsDoc, String> {
    let c = conn(app)?;
    let reading = app.feeds.reading_now.lock().unwrap_or_else(|e| e.into_inner()).get(&key.to_string()).cloned().unwrap_or_default();
    let fetched_at = fetched_at(&c, key)?;
    Ok(FilingsDoc {
        ok: true,
        symbol: key.symbol().to_string(),
        listing: key.to_string(),
        available: disclosures::available(),
        sources: source_status(&c, key)?,
        categories: disclosures::CATEGORIES.iter().map(|s| s.to_string()).collect(),
        ever_read: !fetched_at.is_empty(),
        fetched_at,
        summary_status: enrich::summary_status().to_string(),
        reading,
        filings: fresh_filings(&c, &filers_kept(&c, key)?)?,
    })
}

/// A page has started showing a listing's disclosures (`doc` is the document's
/// key): bring the list up to date if it is stale, then read what has no title,
/// until no page shows them any more. One pass per listing at a time.
pub fn filings_shown(app: Arc<App>, doc: String, key: ListingKey) {
    spawn("bagholder-filings-shown", move || {
        app.single_flight(&format!("filings-shown:{key}"), (), || {
            *app.feeds.looking_at.lock().unwrap() = key.to_string();
            // the list brought up to date: a store that fails is said in the header until it answers
            let refreshed = conn(&app).and_then(|c| filings_stale(&c, &key, None)).and_then(|stale| {
                if stale { refresh_filings(&app, &key).map(|_| ()) } else { Ok(()) }
            });
            went(&app, DISCLOSURES, refreshed.map_err(|e| format!("{}'s disclosures could not be brought up to date: {e}", key.named())));
            read_while_shown(&app, &doc, &key, &|source, id| filings_enrich_result(&app, source, id));
        });
    });
}

/// Read each of `key`'s documents that has no title, newest first, each once, while
/// a page shows `doc`. A document that could not be read is said in the header
/// until one is, by its kind: one refused offline asked no one and is not said.
fn read_while_shown(app: &Arc<App>, doc: &str, key: &ListingKey, read: &dyn Fn(Regulator, &str) -> Result<Enriched, bagholder_net::Failure>) {
    let mut tried: HashSet<(Regulator, String)> = HashSet::new();
    while app.events.watched(doc) && !app.stopping() {
        let left = conn(app).and_then(|c| filers_kept(&c, key).and_then(|f| sf::filings_of(&c, &f).map_err(|e| e.to_string())));
        let Some(left) = went(app, DISCLOSURES_STORED, left.map_err(|e| format!("The disclosures held could not be read: {e}"))) else { break };
        let mut left: Vec<Filing> = left.into_iter()
            .filter(|r| wants_reading(r) && !tried.contains(&(r.doc.source, r.doc.id.clone())))
            .collect();
        left.sort_by(|a, b| b.doc.date.cmp(&a.doc.date));
        let Some(next) = left.first().map(|r| (r.doc.source, r.doc.id.clone())) else { break };
        // the documents are said to be reading only once this one's read has actually
        // been sent: one refused before it left (offline, its host resting, nothing to
        // read it with) asked no one, and a page draws no placeholders that come and go
        // (#534)
        let reading: Vec<String> = left.iter().map(|r| r.doc.id.clone()).collect();
        let sent = std::rc::Rc::new(std::cell::Cell::new(false));
        let (a, k, said) = (app.clone(), key.clone(), sent.clone());
        // waits for the local model itself when one is coming up
        let read = bagholder_net::client::while_leaving(
            move || {
                if !said.replace(true) {
                    set_reading(&a, &k, reading.clone());
                }
            },
            || read(next.0, &next.1),
        );
        if !sent.get() {
            set_reading(app, key, vec![]);
        }
        went(app, DISCLOSURES_READING, read.map_err(|e| e.map(|e| format!("{}'s document {} could not be read: {e}", key.named(), next.1))));
        tried.insert(next);
        if app.wait(Duration::from_millis(150)) { // a person's pace at the source
            break;
        }
    }
    set_reading(app, key, vec![]);
}

/// What `filings_payload` returns: the `GET /api/filings?…&refresh=1` answer.
#[derive(Clone, Debug, Serialize, TS, Diff)]
#[serde(rename_all = "camelCase")]
pub struct FilingsPayload {
    #[ts(type = "true")]
    pub ok: bool,
    pub symbol: String,
    /// The listing (`SYMBOL@VENUE`).
    pub listing: String,
    pub available: bool,
    pub sources: BTreeMap<String, SourceStatus>,
    pub categories: Vec<String>,
    pub profile_no: String,
    pub fetched_at: String,
    pub refreshed: bool,
    pub source_unavailable: bool,
    pub filings: Vec<Filing>,
}

/// `filings_payload` on one connection with the refresh given.
pub fn filings_payload_in(app: &Arc<App>, c: &Connection, key: &ListingKey, refresh: bool, refresh_filings: &dyn Fn() -> Result<i64, String>) -> Result<FilingsPayload, String> {
    *app.feeds.looking_at.lock().unwrap() = key.to_string();
    let mut wrote: Option<i64> = None;
    if refresh || filings_stale(c, key, None)? {
        wrote = Some(refresh_filings()?);
    }
    let filers = filers_kept(c, key)?;
    Ok(FilingsPayload {
        ok: true,
        available: disclosures::available(),
        sources: source_status(c, key)?,
        categories: disclosures::CATEGORIES.iter().map(|s| s.to_string()).collect(),
        profile_no: filers.iter().find(|f| f.source == Regulator::Sedar).map(|f| f.id.clone()).unwrap_or_default(),
        fetched_at: fetched_at(c, key)?,
        refreshed: wrote.map(|w| w > 0).unwrap_or(false),
        source_unavailable: wrote == Some(-1),
        filings: fresh_filings(c, &filers)?,
        symbol: key.symbol().to_string(),
        listing: key.to_string(),
    })
}

/// The filers' documents, newest first: rows read by an older logic blanked,
/// categories re-derived.
fn fresh_filings(c: &Connection, filers: &[sf::Filer]) -> Result<Vec<Filing>, String> {
    let mut rows = sf::filings_of(c, filers).map_err(|e| e.to_string())?;
    for r in rows.iter_mut() {
        if r.enrich_version.unwrap_or(0) < ENRICH_VERSION {
            r.subject = String::new();
            r.summary = String::new();
        }
        r.doc.category = disclosures::categorize(&r.doc);
        // the list arrives named: a form and a named document say what they are
        // without being fetched, so only the rest wait on a reading
        if r.subject.is_empty() {
            if let Some(t) = disclosures::quick_title(&r.doc) {
                sf::set_filing_enrichment(c, r.doc.source, &r.doc.id, Some(&t), None, None, None, &now_iso()).map_err(|e| e.to_string())?;
                r.subject = t;
            }
        }
    }
    Ok(rows)
}

/// A stored document, by its source and its source's own id: (bytes, content
/// type), or the error.
pub fn filings_document(app: &Arc<App>, source: Regulator, doc_id: &str) -> Result<(Vec<u8>, String), String> {
    let c = conn(app)?;
    let row = sf::filing(&c, source, doc_id).map_err(|e| e.to_string())?.ok_or_else(|| format!("no {} document {doc_id} is kept", source.as_str()))?;
    let (data, ct) = disclosures::document(&row.doc).map_err(|e| e.to_string())?;
    if data.is_empty() {
        return Err("empty document".into());
    }
    Ok((data, if ct.is_empty() { "application/octet-stream".into() } else { ct }))
}

/// `filings_enrich` on the app's own connection, typed.
fn filings_enrich_result(app: &Arc<App>, source: Regulator, doc_id: &str) -> Result<Enriched, bagholder_net::Failure> {
    let c = conn(app)?;
    filings_enrich_in(&c, source, doc_id, &LiveReaders)
}

/// One document read for its subject and, with a
/// local model, a one-sentence summary; both cached on the row.
/// `GET /api/filings/enrich`.
#[derive(Serialize, TS)]
#[serde(untagged)]
pub enum EnrichAnswer {
    Ok(Enriched),
    Refused(OkOr),
}

pub fn filings_enrich(app: &Arc<App>, source: Regulator, doc_id: &str) -> EnrichAnswer {
    match filings_enrich_result(app, source, doc_id) {
        Ok(e) => EnrichAnswer::Ok(e),
        Err(e) => EnrichAnswer::Refused(OkOr::err(e.to_string())),
    }
}

/// What `filings_enrich_in` reads a document with: the local model and the
/// disclosure sources.
pub trait Readers {
    fn summary_available(&self) -> bool { enrich::summary_available() }
    fn summary_status(&self) -> &'static str { enrich::summary_status() }
    fn wait_for_summary(&self, seconds: f64) -> bool { enrich::wait_for_summary(seconds) }
    fn disclosures_available(&self) -> bool { disclosures::available() }
    fn enrichment(&self, row: &FiledDocument) -> Option<disclosures::Enrichment> { disclosures::enrichment(row) }
    fn content(&self, row: &FiledDocument) -> disclosures::Fetched<(Vec<u8>, String)> { disclosures::content(row) }
    fn enrich_document_of(&self, code: &str, source: &str, data: &[u8], ct: &str) -> disclosures::Enrichment { enrich::enrich_document_of(code, source, data, ct) }
}

struct LiveReaders;
impl Readers for LiveReaders {}

/// One document's title and summary, as `filings_enrich` answers.
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct Enriched {
    #[ts(type = "true")]
    pub ok: bool,
    pub id: String,
    pub subject: String,
    pub summary: String,
    pub summary_available: bool,
    pub summary_status: String,
}

/// `filings_enrich` on one connection with the readers given.
///
/// A row is read until it holds both a title and a sentence, or until two reads
/// that could have answered have been made: a read counts when it reached the
/// document with a model up to write the sentence. A read with no model up only
/// failed to ask; it keeps the title it found and the row waits, fetched again
/// only once a model is up (SPEC §4 Disclosures).
pub fn filings_enrich_in(c: &Connection, source: Regulator, doc_id: &str, r: &dyn Readers) -> Result<Enriched, bagholder_net::Failure> {
    let row = sf::filing(c, source, doc_id).map_err(|e| e.to_string())?.ok_or_else(|| "no such document".to_string())?;
    let mut subject = row.subject.clone();
    let mut summary = row.summary.clone();
    let mut model = r.summary_available();
    let fresh = row.enrich_version.unwrap_or(0) >= ENRICH_VERSION;
    let attempted = !row.enriched_at.is_empty() && fresh;
    // reads under an older logic say nothing about what this one can find
    let reads = if fresh { row.enrich_reads } else { 0 };
    let answer = |subject: &str, summary: &str, avail: bool| Enriched {
        ok: true, id: doc_id.to_string(), subject: subject.to_string(), summary: summary.to_string(),
        summary_available: avail, summary_status: r.summary_status().to_string(),
    };
    if attempted && (row.enrich_final || (!subject.is_empty() && !summary.is_empty()) || !model || reads >= ENRICH_READS) {
        return Ok(answer(&subject, &summary, model));
    }
    if !r.disclosures_available() {
        return Ok(answer(&subject, &summary, model));
    }
    let now = now_iso();
    // No model to write the sentence: what was found is kept under the current
    // logic's stamp, so the row waits for a model rather than being fetched again,
    // and the read is not counted against the document.
    let waits = |subject: &str, summary: &str| -> Result<(), String> {
        sf::set_filing_enrichment(c, source, doc_id, Some(subject), Some(summary), Some(ENRICH_VERSION), Some(false), &now).map_err(|e| e.to_string())?;
        sf::set_filing_reads(c, source, doc_id, reads).map_err(|e| e.to_string())?;
        Ok(())
    };
    if !fresh {
        // only the first read under the current logic replaces both halves
        subject = String::new();
        summary = String::new();
    }
    if let Some(exact) = r.enrichment(&row.doc) {
        // What the source can say exactly: the form's own name, and for the
        // forms it can read in full, the sentence too. A name on its own is
        // kept and the document still read, so the sentence follows it.
        let (sj, sm) = (exact.subject.clone(), exact.summary.clone());
        if !sj.is_empty() && subject.is_empty() {
            subject = sj.clone();
            sf::set_filing_enrichment(c, source, doc_id, Some(&subject), None, None, None, &now).map_err(|e| e.to_string())?;
        }
        if !sm.is_empty() {
            sf::set_filing_enrichment(c, source, doc_id, Some(&sj), Some(&sm), Some(ENRICH_VERSION), None, &now).map_err(|e| e.to_string())?;
            return Ok(answer(&sj, &sm, model));
        }
        if exact.final_ {
            // a named document: nothing a reading would add
            sf::set_filing_enrichment(c, source, doc_id, Some(&subject), Some(""), Some(ENRICH_VERSION), Some(true), &now).map_err(|e| e.to_string())?;
            return Ok(answer(&subject, "", model));
        }
        if !model {
            waits(&subject, &summary)?;
            return Ok(answer(&subject, &summary, model));
        }
    }
    let (data, ct) = match r.content(&row.doc) {
        Ok(x) => x,
        // no longer listed: a fact kept on its row, read for good as what it is, and
        // never a failure that stands in the header (brief 69)
        Err(disclosures::SourceError::NotListed(_)) => {
            sf::set_filing_unlisted(c, source, doc_id, &now).map_err(|e| e.to_string())?;
            let named = if subject.is_empty() { disclosures::quick_title(&row.doc).unwrap_or_else(|| row.doc.form.clone()).chars().take(90).collect() } else { subject.clone() };
            sf::set_filing_enrichment(c, source, doc_id, Some(&named), Some(&summary), Some(ENRICH_VERSION), Some(true), &now).map_err(|e| e.to_string())?;
            return Ok(answer(&named, &summary, model));
        }
        Err(e) => return Err(e.failure()),
    };
    if data.is_empty() {
        return Err("the document could not be read".into());
    }
    if !model {
        model = r.wait_for_summary(enrich::SUMMARY_WAIT_SEC);
    }
    let info = r.enrich_document_of(&row.doc.form, row.doc.source.as_str(), &data, &ct);
    let new_subject = info.subject.clone();
    let got_summary = info.summary.clone();
    if info.final_ {
        sf::set_filing_enrichment(c, source, doc_id, Some(&new_subject), Some(&got_summary), Some(ENRICH_VERSION), Some(true), &now).map_err(|e| e.to_string())?;
        return Ok(answer(&new_subject, &got_summary, model));
    }
    if model && new_subject.is_empty() && got_summary.is_empty() && subject.is_empty() && summary.is_empty() {
        // a document with no text in it -- a release filed as a picture -- has
        // nothing for a reading to find, now or later: it is named by what it
        // is and never read again
        let named = disclosures::quick_title(&row.doc).unwrap_or_else(|| row.doc.form.clone());
        let named: String = named.chars().take(90).collect();
        sf::set_filing_enrichment(c, source, doc_id, Some(&named), Some(""), Some(ENRICH_VERSION), Some(true), &now).map_err(|e| e.to_string())?;
        return Ok(answer(&named, "", model));
    }
    // reading again fills what is missing and never empties what is there
    if !new_subject.is_empty() {
        subject = new_subject;
    }
    if model {
        if !got_summary.is_empty() {
            summary = got_summary;
        }
        // a read that could have answered: two of them settle the row
        let reads = reads + 1;
        let settled = (subject.is_empty() || summary.is_empty()) && reads >= ENRICH_READS;
        sf::set_filing_enrichment(c, source, doc_id, Some(&subject), Some(&summary), Some(ENRICH_VERSION), Some(settled), &now).map_err(|e| e.to_string())?;
        sf::set_filing_reads(c, source, doc_id, reads).map_err(|e| e.to_string())?;
    } else {
        waits(&subject, &summary)?;
    }
    Ok(answer(&subject, &summary, r.summary_available()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::f;
    use serde_json::{json, Value};
    use std::cell::Cell;

    /// An app of its own on a home of its own, the recorded month in its book and
    /// its figures built.
    fn built_app() -> (tempfile::TempDir, Arc<App>) {
        crate::tests_common::home(); // offline, dry orders
        let home = tempfile::tempdir().unwrap();
        crate::tests_common::pulled_book(home.path());
        let app = App::new(home.path().to_path_buf(), std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.."), "127.0.0.1".into());
        let at: bagholder_core::jiff::Timestamp = "2025-11-19T21:00:00Z".parse().unwrap();
        let f = crate::figures::Figures::open(home.path(), at).unwrap();
        f.state_zone("America/Toronto", at).unwrap();
        app.set_figures(f);
        (home, app)
    }

    /// The market cache's schema, where the filings are kept.
    fn store() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        for m in &bagholder_sources::cache::MIGRATIONS {
            c.execute_batch(m.sql).unwrap();
        }
        c
    }

    fn item(source: Regulator, i: i64, profile: &str) -> FiledDocument {
        let sec = source == Regulator::Sec;
        let tag = if sec { "sec" } else { "sedar" };
        FiledDocument {
            id: format!("{}:{}", tag, i),
            source,
            category: "Financials".into(),
            profile_no: profile.into(),
            issuer: String::new(),
            form: if sec { "10-Q" } else { "Interim MD&A" }.into(),
            title: if sec { "Quarterly report" } else { "" }.into(),
            date: format!("2026-08-{:02}", 10 + i),
            date_text: format!("2026-08-{:02}", 10 + i),
            size: if sec { "" } else { "292 KB" }.into(),
            url: if sec { format!("https://www.sec.gov/x/{}", i) } else { format!("https://www.sedarplus.ca/x?drmKey={}", i) },
        }
    }

    /// A filer standing in by name: `QNC`'s SEDAR+ profile, its SEC CIK.
    fn filer(name: &str, source: Regulator) -> sf::Filer {
        sf::Filer { source, id: name.into() }
    }

    fn replace(c: &Connection, name: &str, src: Regulator, items: &[FiledDocument]) {
        sf::replace_filings(c, &filer(name, src), items, &now_iso()).unwrap();
    }

    /// The source a stand-in document's id names (`sedar:1`, `sec:2`).
    fn src_of(id: &str) -> Regulator {
        if id.starts_with("sec:") { Regulator::Sec } else { Regulator::Sedar }
    }

    fn enrichment(c: &Connection, id: &str, subject: &str, summary: &str, version: i64) {
        sf::set_filing_enrichment(c, src_of(id), id, Some(subject), Some(summary), Some(version), None, &now_iso()).unwrap();
    }

    /// A typed result as `filings_payload_in`/`filings_enrich_in` serve it: the
    /// success shape, or `{"ok": false, "error": …}`.
    fn v<T: serde::Serialize, E: std::fmt::Display>(r: Result<T, E>) -> Value {
        match r {
            Ok(x) => serde_json::to_value(x).unwrap(),
            Err(e) => json!({"ok": false, "error": e.to_string()}),
        }
    }


    // --- the store: documents per filer

    #[test]
    fn test_rows_of_two_filers_merge_newest_first() {
        let c = store();
        replace(&c, "SHOP", Regulator::Sedar, &[item(Regulator::Sedar, 1, ""), item(Regulator::Sedar, 3, "")]);
        replace(&c, "SHOP", Regulator::Sec, &[item(Regulator::Sec, 2, ""), item(Regulator::Sec, 4, "")]);
        let rows = sf::filings_of(&c, &[filer("SHOP", Regulator::Sedar), filer("SHOP", Regulator::Sec)]).unwrap();
        let dates: Vec<String> = rows.iter().map(|r| r.doc.date.clone()).collect();
        assert_eq!(dates, ["2026-08-14", "2026-08-13", "2026-08-12", "2026-08-11"]);
    }

    #[test]
    fn test_replacing_one_filer_leaves_the_others() {
        let c = store();
        replace(&c, "SHOP", Regulator::Sedar, &[item(Regulator::Sedar, 1, ""), item(Regulator::Sedar, 2, "")]);
        replace(&c, "ATD", Regulator::Sedar, &[item(Regulator::Sedar, 5, "")]);
        replace(&c, "SHOP", Regulator::Sedar, &[item(Regulator::Sedar, 9, "")]);
        assert_eq!(sf::filings_of(&c, &[filer("SHOP", Regulator::Sedar)]).unwrap().len(), 1, "replaced, not appended");
        assert_eq!(sf::filings_of(&c, &[filer("ATD", Regulator::Sedar)]).unwrap().len(), 1, "another filer untouched");
    }

    #[test]
    fn test_a_single_row_is_fetchable_by_its_source_and_id_for_download() {
        let c = store();
        replace(&c, "SHOP", Regulator::Sec, &[item(Regulator::Sec, 7, "")]);
        let row = sf::filing(&c, Regulator::Sec, "sec:7").unwrap().unwrap();
        assert!(row.doc.url.starts_with("https://www.sec.gov/"));
        assert!(sf::filing(&c, Regulator::Sec, "sec:999").unwrap().is_none());
        assert!(sf::filing(&c, Regulator::Sedar, "sec:7").unwrap().is_none(), "the source is part of the document's name");
    }

    // --- which filers a listing's filings come from

    /// The regulators standing in: who files each listing, each filer's documents,
    /// and the TSX's interlisted list.
    struct Regs {
        filers: Vec<(ListingKey, Result<String, String>)>,
        documents: Vec<(sf::Filer, disclosures::Fetched<Vec<FiledDocument>>)>,
        pairs: Vec<(ListingKey, ListingKey)>,
        asked: Cell<usize>,
        /// The interlisted list's reads, and whether it, or who files, cannot be reached.
        lists_read: Cell<usize>,
        list_down: bool,
        filers_down: bool,
    }

    impl Regs {
        fn new() -> Regs {
            Regs { filers: vec![], documents: vec![], pairs: vec![], asked: Cell::new(0), lists_read: Cell::new(0), list_down: false, filers_down: false }
        }
    }

    impl disclosures::Regulators for Regs {
        fn filer(&self, listing: &ListingKey, _: &str) -> Option<(Regulator, disclosures::Fetched<Result<String, String>>)> {
            self.asked.set(self.asked.get() + 1);
            let source = own_regulator(listing)?;
            if self.filers_down {
                return Some((source, Err(disclosures::SourceError::Unavailable("SEDAR+ could not be reached".into()))));
            }
            let found = self.filers.iter().find(|(k, _)| k == listing).map(|(_, f)| f.clone()).unwrap_or_else(|| Err(format!("no filer for {listing}")));
            Some((source, Ok(found)))
        }
        fn documents(&self, filer: &sf::Filer, _: usize) -> disclosures::Fetched<Vec<FiledDocument>> {
            self.documents.iter().find(|(f, _)| f == filer).map(|(_, d)| d.clone()).unwrap_or(Ok(vec![]))
        }
        fn interlisted(&self) -> disclosures::Fetched<(String, Vec<sf::Interlisted>)> {
            self.lists_read.set(self.lists_read.get() + 1);
            if self.list_down {
                return Err(disclosures::SourceError::Unavailable("the TSX could not be reached".into()));
            }
            Ok(("2026-10-01".into(), self.pairs.iter().map(|(ca, us)| sf::Interlisted { listing: ca.clone(), us_listing: us.clone(), name: String::new() }).collect()))
        }
    }

    fn on(symbol: &str, venue: &str) -> ListingKey {
        ListingKey::on(symbol, venue).unwrap()
    }

    fn shown(c: &Connection, key: &ListingKey) -> Vec<String> {
        let mut ids: Vec<String> = sf::filings_of(c, &filers_kept(c, key).unwrap()).unwrap().into_iter().map(|r| r.doc.id).collect();
        ids.sort();
        ids
    }

    /// For generated pairs of a Canadian and a US listing sharing a ticker, with
    /// regulators' answers and the interlisted list: a listing shows exactly its
    /// own filer's documents, and its partner's where the list pairs them; never
    /// another company's, whatever the two names share (`Energy Fuels` and `Energy
    /// Transfer`), and the outcome is the same in either order of reads.
    #[test]
    fn a_listings_filings_are_its_own_filers_and_its_interlisted_partners_and_never_another_companys() {
        let (_home, app) = built_app();
        for symbol in ["T", "EF", "BRK.B", "AB1"] {
            for paired in [false, true] {
                for us_first in [false, true] {
                    let c = store();
                    let (ca, us) = (on(symbol, "TSX"), on(symbol, "NYSE"));
                    let mut r = Regs::new();
                    r.filers = vec![(ca.clone(), Ok("000000001".into())), (us.clone(), Ok("1001".into()))];
                    r.documents = vec![
                        (filer("000000001", Regulator::Sedar), Ok(vec![item(Regulator::Sedar, 1, "000000001")])),
                        (filer("1001", Regulator::Sec), Ok(vec![item(Regulator::Sec, 2, "")])),
                    ];
                    if paired {
                        // the TSX's list pairs the Canadian listing with a US one under another symbol too
                        r.pairs = vec![(ca.clone(), us.clone())];
                    }
                    let order = if us_first { [us.clone(), ca.clone()] } else { [ca.clone(), us.clone()] };
                    for k in &order {
                        refresh_filings_in(&app, &c, k, &r).unwrap();
                    }
                    let (ca_shows, us_shows) = (shown(&c, &ca), shown(&c, &us));
                    if paired {
                        assert_eq!(ca_shows, ["sec:2", "sedar:1"], "{symbol}: a paired listing shows both regulators'");
                        assert_eq!(us_shows, ["sec:2", "sedar:1"], "{symbol}: either way round");
                    } else {
                        assert_eq!(ca_shows, ["sedar:1"], "{symbol}: unpaired, the TSX listing shows its own regulator's alone");
                        assert_eq!(us_shows, ["sec:2"], "{symbol}: and the US listing its own");
                    }
                }
            }
        }
    }

    /// The other country's filer is what the interlisted list pairs a listing with at
    /// each settle: once the list stops pairing two listings, neither shows the
    /// other's, and a company that lists under the ticker later is its own.
    #[test]
    fn a_pair_the_interlisted_list_drops_leaves_neither_listing_the_others_filer() {
        let (_home, app) = built_app();
        let c = store();
        let (ca, us) = (on("X", "TSX"), on("X", "NYSE"));
        let mut r = Regs::new();
        r.filers = vec![(ca.clone(), Ok("000000001".into())), (us.clone(), Ok("1001".into()))];
        r.documents = vec![
            (filer("000000001", Regulator::Sedar), Ok(vec![item(Regulator::Sedar, 1, "000000001")])),
            (filer("1001", Regulator::Sec), Ok(vec![item(Regulator::Sec, 2, "")])),
            (filer("2002", Regulator::Sec), Ok(vec![item(Regulator::Sec, 3, "")])),
        ];
        r.pairs = vec![(ca.clone(), us.clone())];
        for k in [&ca, &us] {
            refresh_filings_in(&app, &c, k, &r).unwrap();
        }
        assert_eq!((shown(&c, &ca), shown(&c, &us)), (vec!["sec:2".to_string(), "sedar:1".into()], vec!["sec:2".to_string(), "sedar:1".into()]));
        // a day on, the list no longer pairs them, and another company lists as X in the US
        r.pairs = vec![];
        r.filers = vec![(ca.clone(), Ok("000000001".into())), (us.clone(), Ok("2002".into()))];
        let old = crate::app::stamp_of(now_unix() as i64 - 25 * 3600);
        c.execute("UPDATE listing_filers SET checked_at = ?", [&old]).unwrap();
        set_meta(&c, "interlisted:read_at", &old).unwrap();
        for k in [&ca, &us] {
            refresh_filings_in(&app, &c, k, &r).unwrap();
        }
        assert_eq!(shown(&c, &ca), ["sedar:1"], "the TSX listing shows its own filer's alone");
        assert_eq!(shown(&c, &us), ["sec:3"], "the US listing shows the new company's alone");
    }

    /// A read that fails rests before it is asked again, on a rest that grows with
    /// each failure in a row, and is said until a read goes through: the TSX's
    /// interlisted list, and a regulator asked who files a listing never settled.
    #[test]
    fn a_failed_read_rests_and_is_never_said_in_the_header() {
        let (_home, app) = built_app();
        let c = store();
        let key = on("XYZ", "TSX");
        let mut r = Regs::new();
        r.list_down = true;
        r.filers_down = true;
        let age = |table: &str, column: &str, secs: u64| {
            let then = crate::app::stamp_of(now_unix() as i64 - secs as i64 - 1);
            c.execute(&format!("UPDATE {table} SET {column} = ?"), [&then]).unwrap();
        };
        let said = || crate::feeds::feed_failures(&app).iter().any(|f| f.contains("interlisted"));
        for _ in 0..6 {
            let _ = refresh_filings_in(&app, &c, &key, &r);
        }
        assert_eq!((r.lists_read.get(), r.asked.get()), (1, 1), "each failure asked once over six passes");
        let out = v(filings_payload_in(&app, &c, &key, false, &|| refresh_filings_in(&app, &c, &key, &r)));
        assert!(f(&out["sources"]["SEDAR+"], "error").contains("could not be asked"), "the failure said while it rests: {out}");
        // past the first rest: asked again, and failing twice in a row, never said in
        // the header: a market source's failure goes to the log (brief 79, amended
        // 2026-10-28)
        age("read_failures", "failed_at", FAILED_READ_REST.as_secs());
        let _ = refresh_filings_in(&app, &c, &key, &r);
        assert_eq!((r.lists_read.get(), r.asked.get()), (2, 2));
        assert!(!said(), "{:?}", crate::feeds::feed_failures(&app));
        // the rest grows: past the first rest again, still resting
        age("read_failures", "failed_at", FAILED_READ_REST.as_secs());
        let _ = refresh_filings_in(&app, &c, &key, &r);
        assert_eq!((r.lists_read.get(), r.asked.get()), (2, 2), "twice the rest after a second failure");
        assert!(!said(), "never said while it rests");
        // past the grown rest, the sources answer: nothing stands, nothing said
        age("read_failures", "failed_at", bagholder_sources::market::grown_rest(FAILED_READ_REST, 2).as_secs());
        r.list_down = false;
        r.filers_down = false;
        r.filers = vec![(key.clone(), Ok("000000001".into()))];
        refresh_filings_in(&app, &c, &key, &r).unwrap();
        assert_eq!((r.lists_read.get(), r.asked.get()), (3, 3));
        assert!(!said());
        let left: i64 = c.query_row("SELECT COUNT(*) FROM read_failures", [], |r| r.get(0)).unwrap();
        assert_eq!(left, 0);
    }

    /// What was told of a filer's documents is never told again: a document met
    /// before stands as told even where the market cache no longer holds it and
    /// the stream's mark is older than it.
    #[test]
    fn a_document_told_once_is_never_told_again() {
        let (_home, app) = built_app();
        let c = store();
        let book = notices(&app).unwrap();
        notify::set_settings(&book, &serde_json::from_value(json!({"disclosuresHeld": true})).unwrap()).unwrap();
        let inst = known_filing_listings(&app, &["held".into()]).unwrap().into_iter().find(|l| own_regulator(&l.key).is_some()).expect("a held listing a regulator covers");
        let source = own_regulator(&inst.key).unwrap();
        let mut r = Regs::new();
        r.filers = vec![(inst.key.clone(), Ok("000000001".into()))];
        let docs = |n: i64| (1..=n).map(|i| item(source, i, "000000001")).collect::<Vec<_>>();
        let stale = || c.execute("UPDATE filer_fetches SET fetched_at = ?", [crate::app::stamp_of(now_unix() as i64 - 3600)]).unwrap();
        r.documents = vec![(filer("000000001", source), Ok(docs(1)))];
        let sweep = |r: &Regs| sweep_filings_in(&app, &c, &book, &|k| refresh_filings_in(&app, &c, k, r)).unwrap();
        assert_eq!(sweep(&r), 0, "the first read is where the stream starts: nothing is new yet");
        r.documents = vec![(filer("000000001", source), Ok(docs(2)))];
        stale();
        assert_eq!(sweep(&r), 1, "the document filed since is told");
        // the cache forgets the filer's documents and the stream's mark is an older one
        c.execute_batch("DELETE FROM filings; DELETE FROM filer_fetches;").unwrap();
        let scope = format!("filings:{}:000000001", source.as_str());
        book.set_setting(&format!("{}{scope}", notify::WATERMARK), Some("2026-08-01|"), bagholder_core::jiff::Timestamp::now()).unwrap();
        assert_eq!(sweep(&r), 0, "what was told is not told again");
    }

    /// A listing's filers stand for a day: within it no regulator is asked again,
    /// and past it the regulator is asked and its answer replaces the one before
    /// (a tie settled one day is asked again the next); the interlisted list is
    /// read once a day too.
    #[test]
    fn a_listings_filers_are_asked_again_each_day_and_the_interlisted_list_read_once_a_day() {
        let (_home, app) = built_app();
        let c = store();
        let key = on("XYZ", "TSX");
        let mut r = Regs::new();
        r.filers = vec![(key.clone(), Err("SEDAR+ lists two issuers named Xyz Corp, each with filings in the last 485 days.".into()))];
        for _ in 0..4 {
            refresh_filings_in(&app, &c, &key, &r).unwrap();
        }
        assert_eq!((r.asked.get(), r.lists_read.get()), (1, 1), "within the day: neither asked again");
        let old = crate::app::stamp_of(now_unix() as i64 - (FILERS_RECHECK_HOURS * 3600.0) as i64 - 1);
        c.execute("UPDATE listing_filer_misses SET checked_at = ?", [&old]).unwrap();
        set_meta(&c, "interlisted:read_at", &old).unwrap();
        r.filers = vec![(key.clone(), Ok("000000001".into()))];
        r.documents = vec![(filer("000000001", Regulator::Sedar), Ok(vec![item(Regulator::Sedar, 1, "000000001")]))];
        refresh_filings_in(&app, &c, &key, &r).unwrap();
        assert_eq!((r.asked.get(), r.lists_read.get()), (2, 2), "a day on: both asked again");
        assert_eq!(filers_kept(&c, &key).unwrap(), [filer("000000001", Regulator::Sedar)], "the new answer replaces the tie");
    }

    /// One exact SEDAR+ match is the listing's filer only where it has filed within
    /// the window a listed issuer files in, read from its documents with no request
    /// of its own; a dormant one is a miss, said, and none of its documents shown.
    #[test]
    fn one_exact_sedar_match_that_has_not_filed_within_the_window_is_a_miss() {
        let (_home, app) = built_app();
        let window = bagholder_market::sedar::LISTED_FILING_WINDOW_DAYS;
        for (days_ago, kept) in [(0, true), (window, true), (window + 1, false), (4000, false)] {
            let c = store();
            let key = on("XYZ", "TSX");
            let mut r = Regs::new();
            r.filers = vec![(key.clone(), Ok("000000001".into()))];
            let mut doc = item(Regulator::Sedar, 1, "000000001");
            doc.date = bagholder_model::dates::shift_date(&today(), -days_ago);
            r.documents = vec![(filer("000000001", Regulator::Sedar), Ok(vec![doc]))];
            refresh_filings_in(&app, &c, &key, &r).unwrap();
            assert_eq!(!filers_kept(&c, &key).unwrap().is_empty(), kept, "{days_ago} days ago");
            assert_eq!(shown(&c, &key).len(), usize::from(kept), "{days_ago} days ago");
            if !kept {
                let why = sf::listing_filer_miss(&c, &key, Regulator::Sedar).unwrap().expect("a miss").0;
                assert!(why.contains(&format!("{window} days")), "{why}");
            }
        }
        // a filer with no documents at all has filed nothing in the window either
        let c = store();
        let key = on("XYZ", "TSX");
        let mut r = Regs::new();
        r.filers = vec![(key.clone(), Ok("000000001".into()))];
        refresh_filings_in(&app, &c, &key, &r).unwrap();
        assert!(filers_kept(&c, &key).unwrap().is_empty());
    }

    /// An answer that comes back empty for a profile with recent documents stored
    /// leaves them standing: it is no proof the profile filed nothing (brief 44, 1).
    #[test]
    fn an_empty_answer_leaves_a_profiles_stored_documents_standing() {
        let (_home, app) = built_app();
        let c = store();
        let key = on("XYZ", "TSX");
        let mut r = Regs::new();
        r.filers = vec![(key.clone(), Ok("000000001".into()))];
        let mut doc = item(Regulator::Sedar, 1, "000000001");
        doc.date = today();
        r.documents = vec![(filer("000000001", Regulator::Sedar), Ok(vec![doc]))];
        refresh_filings_in(&app, &c, &key, &r).unwrap();
        assert_eq!(shown(&c, &key).len(), 1);
        // the next read answers with nothing
        r.documents = vec![(filer("000000001", Regulator::Sedar), Ok(vec![]))];
        c.execute("UPDATE filer_fetches SET fetched_at = ?", [crate::app::stamp_of(now_unix() as i64 - 25 * 3600)]).unwrap();
        refresh_filings_in(&app, &c, &key, &r).unwrap();
        assert_eq!(filers_kept(&c, &key).unwrap(), [filer("000000001", Regulator::Sedar)], "still the listing's filer");
        assert_eq!(shown(&c, &key).len(), 1, "its document still shown");
        assert!(sf::listing_filer_miss(&c, &key, Regulator::Sedar).unwrap().is_none());
    }

    /// A document read while a page shows a listing's disclosures is said in the
    /// header only for a failure of the source's: one refused offline asked no
    /// one, and stays unsaid however often it comes back (brief 45, 4).
    #[test]
    fn a_document_read_that_fails_is_never_said_in_the_header() {
        let (_home, app) = built_app();
        let key = on("XYZ", "NYSE");
        let mut r = Regs::new();
        r.filers = vec![(key.clone(), Ok("0000000001".into()))];
        let mut unread = item(Regulator::Sec, 1, "0000000001");
        unread.title = String::new();
        r.documents = vec![(filer("0000000001", Regulator::Sec), Ok(vec![unread]))];
        refresh_filings_in(&app, &crate::feeds::conn(&app).unwrap(), &key, &r).unwrap();
        let feed = crate::events::Feed::open(app.clone());
        let doc = "disclosures-shown";
        assert!(app.events.watch(&app, feed.id(), [(doc.to_string(), crate::events::Want { params: json!({}), have: None })].into_iter().collect()));
        let said = || crate::status::status(&app).error;
        // read twice over: a failure said once the next read fails too
        for _ in 0..2 {
            read_while_shown(&app, doc, &key, &|_, _| Err(bagholder_net::Failure::Offline));
        }
        assert!(!said().contains("could not be read"), "{}", said());
        for _ in 0..2 {
            read_while_shown(&app, doc, &key, &|_, _| Err("the SEC could not be reached".into()));
        }
        // a market source's failure goes to the log, never to the header (brief 79,
        // amended 2026-10-28)
        assert!(!said().contains("could not be read"), "{}", said());
        drop(feed);
    }

    /// A document is said to be under way only once its read has actually been sent
    /// (#534). Over generated passes, each document's read is refused before it
    /// leaves by one of the ways a read is (offline at the door, its host resting,
    /// nothing to read it with, any refusal of a kind of its own), or sent and then
    /// failed: while a refused read runs, and after it, the listing's disclosures name
    /// nothing as reading; once a read is sent, they name the documents still to read.
    #[test]
    fn a_document_is_said_to_be_reading_only_once_its_read_is_sent() {
        #[derive(Clone, Copy, Debug)]
        enum Read {
            Offline,
            Resting,
            NoSummarizer,
            Refused,
            SentAndFailed,
        }
        const READS: [Read; 5] = [Read::Offline, Read::Resting, Read::NoSummarizer, Read::Refused, Read::SentAndFailed];
        let mut seed = 0x9E37_79B9u64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for case in 0..12 {
            let (_home, app) = built_app();
            let key = on("XYZ", "NYSE");
            let n = 2 + (next() % 4) as i64;
            let mut r = Regs::new();
            r.filers = vec![(key.clone(), Ok("0000000001".into()))];
            let unread: Vec<_> = (1..=n).map(|k| { let mut d = item(Regulator::Sec, k, "0000000001"); d.title = String::new(); d }).collect();
            r.documents = vec![(filer("0000000001", Regulator::Sec), Ok(unread))];
            refresh_filings_in(&app, &crate::feeds::conn(&app).unwrap(), &key, &r).unwrap();
            let feed = crate::events::Feed::open(app.clone());
            let doc = "disclosures-shown";
            assert!(app.events.watch(&app, feed.id(), [(doc.to_string(), crate::events::Want { params: json!({}), have: None })].into_iter().collect()));
            let reads: Vec<Read> = (0..n).map(|_| READS[(next() % READS.len() as u64) as usize]).collect();
            let at = std::cell::Cell::new(0usize);
            // whether the read before this one was sent: the pass is then under way, and
            // what it names stands until a read is refused (cleared at once then, never
            // cleared and named again between two reads sent, which would come and go)
            let after_sent = std::cell::Cell::new(false);
            let reading = || filings_stored(&app, &key).unwrap().reading;
            read_while_shown(&app, doc, &key, &|_, _| {
                let kind = reads[at.get()];
                at.set(at.get() + 1);
                if !after_sent.get() {
                    assert!(reading().is_empty(), "case {case}, {kind:?}: said to be reading before any read was sent");
                }
                after_sent.set(matches!(kind, Read::SentAndFailed));
                match kind {
                    Read::Offline => Err(bagholder_net::Failure::Offline),
                    Read::Resting => Err("sec.gov refused a request and is not asked again before 10:00".into()),
                    Read::NoSummarizer => Err("no local model is up to read it with".into()),
                    Read::Refused => Err(bagholder_net::Failure::Said("refused before it was sent".into())),
                    Read::SentAndFailed => {
                        // sent: it leaves through the door, to a host on this machine
                        assert!(bagholder_net::client::leave("GET", "http://127.0.0.1:9/document").is_ok());
                        let now = reading();
                        assert!(!now.is_empty(), "case {case}: a read sent is said to be under way");
                        Err("the SEC could not be reached".into())
                    }
                }
            });
            assert!(at.get() > 0, "case {case}: every document was asked");
            assert!(reading().is_empty(), "case {case}: nothing is said to be reading once the pass ends");
            drop(feed);
        }
    }

    /// The same through the live readers, for a SEDAR+ document: the process is
    /// offline, so SEDAR+ is never asked and its document's read is never said
    /// (brief 46, 4).
    #[test]
    fn a_sedar_document_read_offline_through_the_live_readers_is_never_said() {
        let (_home, app) = built_app();
        let key = on("ZQRW", "TSX");
        let mut r = Regs::new();
        r.filers = vec![(key.clone(), Ok("000000001".into()))];
        // a form the source names nothing of, so the document itself is read
        let mut release = item(Regulator::Sedar, 1, "000000001");
        release.form = "News release".into();
        r.documents = vec![(filer("000000001", Regulator::Sedar), Ok(vec![release]))];
        refresh_filings_in(&app, &crate::feeds::conn(&app).unwrap(), &key, &r).unwrap();
        let feed = crate::events::Feed::open(app.clone());
        let doc = "disclosures-shown";
        assert!(app.events.watch(&app, feed.id(), [(doc.to_string(), crate::events::Want { params: json!({}), have: None })].into_iter().collect()));
        for _ in 0..2 {
            read_while_shown(&app, doc, &key, &|source, id| filings_enrich_result(&app, source, id));
        }
        let said = crate::status::status(&app).error;
        assert!(!said.contains("could not be read"), "{said}");
        drop(feed);
    }

    /// Each live reader sends from the thread that reads the document, so its read is
    /// named as under way once it leaves (#534): the SEC's through this client, the
    /// local model on loopback through this client too, and SEDAR+'s through the
    /// browser helper, whose `Session::request` passes the same door on the caller's
    /// thread before writing to the helper (not held here: the helper is a process of
    /// its own). Held with a host on this machine standing in for the SEC and for the
    /// model: a read sent names the documents, a refused one names none.
    #[test]
    fn each_live_reader_names_a_read_once_it_is_sent() {
        use bagholder_market::localmodel::hooks;
        use std::io::{Read as _, Write as _};
        // one host on 127.0.0.1 standing in for both: a document, and the model's answer
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for c in l.incoming() {
                let Ok(mut c) = c else { return };
                let mut buf = [0u8; 8192];
                let n = c.read(&mut buf).unwrap_or(0);
                let body = if String::from_utf8_lossy(&buf[..n]).starts_with("POST") {
                    r#"{"choices": [{"message": {"content": "The company reported its quarter."}}]}"#
                } else {
                    "<html><body>The company reported its quarter.</body></html>"
                };
                let _ = c.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes());
            }
        });
        #[derive(Clone, Copy, Debug, PartialEq)]
        enum Way {
            /// the SEC's reader: the document asked of its host through this client
            Sec,
            /// a document already in hand: only the model's request leaves
            ModelOnly,
            /// refused before anything leaves
            Refused,
        }
        struct StandIn<'a> {
            way: Way,
            port: u16,
            app: &'a Arc<App>,
            key: &'a ListingKey,
            named: Cell<Vec<bool>>,
        }
        impl StandIn<'_> {
            fn note(&self) {
                let mut v = self.named.take();
                v.push(!filings_stored(self.app, self.key).unwrap().reading.is_empty());
                self.named.set(v);
            }
        }
        impl Readers for StandIn<'_> {
            fn summary_available(&self) -> bool { true }
            fn summary_status(&self) -> &'static str { "ready" }
            fn wait_for_summary(&self, _: f64) -> bool { true }
            fn disclosures_available(&self) -> bool { true }
            fn enrichment(&self, _: &FiledDocument) -> Option<disclosures::Enrichment> { None }
            fn content(&self, _: &FiledDocument) -> disclosures::Fetched<(Vec<u8>, String)> {
                self.note();
                match self.way {
                    Way::Sec => {
                        let r = bagholder_net::client::request("GET", &format!("http://127.0.0.1:{}/doc", self.port), &[], None, Duration::from_secs(5))
                            .map_err(|e| disclosures::SourceError::Other(e.to_string()))?;
                        self.note();
                        Ok((r.body, "text/html".into()))
                    }
                    Way::ModelOnly => Ok((b"<html><body>The company reported its quarter.</body></html>".to_vec(), "text/html".into())),
                    Way::Refused => Err(disclosures::SourceError::Offline),
                }
            }
            fn enrich_document_of(&self, code: &str, source: &str, data: &[u8], ct: &str) -> disclosures::Enrichment {
                // the live reading, its model asked on loopback
                let port = self.port;
                hooks::ENDPOINT.with(|h| *h.borrow_mut() = Some(Box::new(move || format!("http://127.0.0.1:{port}"))));
                let got = bagholder_market::enrich::enrich_document_of(code, source, data, ct);
                hooks::ENDPOINT.with(|h| *h.borrow_mut() = None);
                self.note();
                got
            }
        }
        for way in [Way::Sec, Way::ModelOnly, Way::Refused] {
            let (_home, app) = built_app();
            let key = on("XYZ", "NYSE");
            let mut r = Regs::new();
            r.filers = vec![(key.clone(), Ok("0000000001".into()))];
            let mut unread = item(Regulator::Sec, 1, "0000000001");
            unread.title = String::new();
            r.documents = vec![(filer("0000000001", Regulator::Sec), Ok(vec![unread]))];
            refresh_filings_in(&app, &crate::feeds::conn(&app).unwrap(), &key, &r).unwrap();
            let feed = crate::events::Feed::open(app.clone());
            let doc = "disclosures-shown";
            assert!(app.events.watch(&app, feed.id(), [(doc.to_string(), crate::events::Want { params: json!({}), have: None })].into_iter().collect()));
            let readers = StandIn { way, port, app: &app, key: &key, named: Cell::new(vec![]) };
            read_while_shown(&app, doc, &key, &|source, id| filings_enrich_in(&crate::feeds::conn(&app).unwrap(), source, id, &readers));
            let named = readers.named.take();
            let want = match way {
                // before the document is asked for, nothing; once it was sent, and after the model
                Way::Sec => vec![false, true, true],
                // nothing until the model's request left
                Way::ModelOnly => vec![false, true],
                Way::Refused => vec![false],
            };
            assert_eq!(named, want, "{way:?}");
            assert!(filings_stored(&app, &key).unwrap().reading.is_empty(), "{way:?}: nothing is said to be reading once the pass ends");
            drop(feed);
        }
    }

    /// A filer that could not be settled is said, and nothing is guessed in its place.
    #[test]
    fn a_listing_whose_filer_is_not_settled_shows_why_and_nothing_else() {
        let (_home, app) = built_app();
        let c = store();
        let key = on("XYZ", "TSX");
        let mut r = Regs::new();
        r.filers = vec![(key.clone(), Err("SEDAR+ lists no single issuer named Xyz Corp.".into()))];
        refresh_filings_in(&app, &c, &key, &r).unwrap();
        assert!(shown(&c, &key).is_empty());
        let out = v(filings_payload_in(&app, &c, &key, false, &|| panic!("settled a moment ago: not read again")));
        assert_eq!(out["sources"]["SEDAR+"]["filer"], false);
        assert_eq!(out["sources"]["SEDAR+"]["error"], "SEDAR+ lists no single issuer named Xyz Corp.");
    }

    #[test]
    fn test_stale_until_its_filers_are_read_then_fresh_within_a_day() {
        let (_home, app) = built_app();
        let c = store();
        let key = on("SHOP", "TSX");
        assert!(filings_stale(&c, &key, None).unwrap(), "never settled");
        let mut r = Regs::new();
        r.filers = vec![(key.clone(), Ok("000000001".into()))];
        r.documents = vec![(filer("000000001", Regulator::Sedar), Ok(vec![item(Regulator::Sedar, 1, "000000001")]))];
        refresh_filings_in(&app, &c, &key, &r).unwrap();
        assert!(!filings_stale(&c, &key, None).unwrap());
        let old = crate::app::stamp_of(now_unix() as i64 - 25 * 3600);
        c.execute("UPDATE filer_fetches SET fetched_at = ?", [&old]).unwrap();
        assert!(filings_stale(&c, &key, None).unwrap(), "a filer read a day ago");
    }

    /// A regulator that could not be reached reads as unavailable and carries its
    /// reason; the other's documents still stand.
    #[test]
    fn test_an_unreachable_regulator_reads_unavailable_and_carries_its_error() {
        let (_home, app) = built_app();
        let c = store();
        let (ca, us) = (on("SHOP", "TSX"), on("SHOP", "NYSE"));
        let mut r = Regs::new();
        r.filers = vec![(ca.clone(), Ok("000000001".into())), (us.clone(), Ok("1001".into()))];
        r.pairs = vec![(ca.clone(), us.clone())];
        r.documents = vec![
            (filer("000000001", Regulator::Sedar), Err(disclosures::SourceError::Unavailable("SEDAR+ did not return the searchReportingIssuers form".into()))),
            (filer("1001", Regulator::Sec), Ok(vec![item(Regulator::Sec, 1, "")])),
        ];
        let out = v(filings_payload_in(&app, &c, &ca, true, &|| refresh_filings_in(&app, &c, &ca, &r)));
        assert_eq!(out["sources"]["SEDAR+"]["available"], false, "a source that failed to answer is unavailable, whatever its dependency");
        assert_eq!(out["sources"]["SEDAR+"]["error"], "SEDAR+ did not return the searchReportingIssuers form");
        assert_eq!(out["sources"]["SEC"]["error"], "", "the source that answered carries no error");
        let srcs: Vec<String> = out["filings"].as_array().unwrap().iter().map(|r| f(r, "source")).collect();
        assert_eq!(srcs, ["SEC"]);
    }

    #[test]
    fn test_every_regulator_unreachable_is_reported() {
        let (_home, app) = built_app();
        let c = store();
        let key = on("SHOP", "TSX");
        let mut r = Regs::new();
        r.filers = vec![(key.clone(), Ok("000000001".into()))];
        r.documents = vec![(filer("000000001", Regulator::Sedar), Err(disclosures::SourceError::Unavailable("the browser helper is missing".into())))];
        let out = v(filings_payload_in(&app, &c, &key, true, &|| refresh_filings_in(&app, &c, &key, &r)));
        assert_eq!(out["ok"], true, "the endpoint still answers cleanly");
        assert_eq!(out["sourceUnavailable"], true);
        assert_eq!(out["filings"], json!([]));
    }

    /// What the page is sent of a listing's disclosures, pinned in
    /// `tests/golden/filings_payload.json` with what depends on this machine or
    /// on the clock (whether a source is reachable here, the stamps, the local
    /// model's state) left out. After an intended change:
    /// `BAGHOLDER_BLESS=1 cargo test -p bagholder-server the_disclosures_the_page_is_sent`.
    #[test]
    fn test_the_disclosures_the_page_is_sent_are_what_they_were() {
        let _g = crate::tests_common::guard();
        fn mask(v: &mut Value) {
            match v {
                Value::Object(m) => {
                    for k in ["available", "fetchedAt", "enrichedAt", "summaryStatus", "summaryAvailable"] {
                        if m.contains_key(k) {
                            m.insert(k.into(), json!("*"));
                        }
                    }
                    m.values_mut().for_each(mask);
                }
                Value::Array(a) => a.iter_mut().for_each(mask),
                _ => {}
            }
        }
        let (_home, app) = built_app();
        let c = store();
        let (ca, us) = (on("SHOP", "TSX"), on("SHOP", "NYSE"));
        let mut form4 = item(Regulator::Sec, 5, "");
        form4.form = "4".into();
        form4.title = String::new();
        let mut r = Regs::new();
        r.filers = vec![(ca.clone(), Ok("000037100".into())), (us.clone(), Ok("1594805".into()))];
        r.pairs = vec![(ca.clone(), us.clone())];
        r.documents = vec![
            (filer("000037100", Regulator::Sedar), Ok(vec![item(Regulator::Sedar, 1, "000037100"), item(Regulator::Sedar, 3, "000037100")])),
            (filer("1594805", Regulator::Sec), Ok(vec![item(Regulator::Sec, 2, ""), item(Regulator::Sec, 4, ""), form4])),
        ];
        let first = v(filings_payload_in(&app, &c, &ca, true, &|| refresh_filings_in(&app, &c, &ca, &r)));
        enrichment(&c, "sec:2", "Quarterly report", "Revenue rose.", ENRICH_VERSION);
        enrichment(&c, "sedar:1", "An old reading", "Read by an older logic.", ENRICH_VERSION - 1);
        sf::set_filing_enrichment(&c, Regulator::Sedar, "sedar:3", Some("Interim MD&A"), None, Some(ENRICH_VERSION), Some(true), &now_iso()).unwrap();
        let again = v(filings_payload_in(&app, &c, &ca, false, &|| panic!("fresh: not read again")));
        let enriched = v(filings_enrich_in(&c, Regulator::Sec, "sec:2", &fake(true, ("x", "y"), false)));
        let mut have = json!({"first": first, "again": again, "enriched": enriched});
        mask(&mut have);
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/filings_payload.json");
        if std::env::var("BAGHOLDER_BLESS").map_or(false, |v| v == "1") {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, serde_json::to_string_pretty(&have).unwrap() + "\n").unwrap();
            return;
        }
        let want: Value = serde_json::from_str(&std::fs::read_to_string(&path).expect("tests/golden/filings_payload.json")).unwrap();
        assert_eq!(have, want, "the disclosures the page is sent are not what they were");
    }

    // --- EnrichTest

    struct Fake {
        model: bool,
        /// The regulator no longer lists the document.
        unlisted: bool,
        status: &'static str,
        wait: bool,
        read: (&'static str, &'static str),
        final_: bool,
        reads: Cell<usize>,
        waits: Cell<usize>,
    }

    impl Readers for Fake {
        fn summary_available(&self) -> bool { self.model }
        fn summary_status(&self) -> &'static str { self.status }
        fn wait_for_summary(&self, _: f64) -> bool { self.waits.set(self.waits.get() + 1); self.wait }
        fn disclosures_available(&self) -> bool { true }
        fn enrichment(&self, _: &FiledDocument) -> Option<disclosures::Enrichment> { None }
        fn content(&self, _: &FiledDocument) -> disclosures::Fetched<(Vec<u8>, String)> {
            self.reads.set(self.reads.get() + 1);
            if self.unlisted {
                return Err(disclosures::SourceError::NotListed("SEDAR+ no longer lists the document 'sedar:1' in profile 1".into()));
            }
            Ok((b"%PDF-1.4 body".to_vec(), "application/pdf".into()))
        }
        fn enrich_document_of(&self, _: &str, _: &str, _: &[u8], _: &str) -> disclosures::Enrichment {
            disclosures::Enrichment { subject: self.read.0.to_string(), summary: self.read.1.to_string(), final_: self.final_ }
        }
    }

    fn fake(model: bool, read: (&'static str, &'static str), final_: bool) -> Fake {
        Fake { model, unlisted: false, status: if model { "ready" } else { "off" }, wait: false, read, final_, reads: Cell::new(0), waits: Cell::new(0) }
    }

    /// A document its regulator no longer lists is a fact kept on its row: named by
    /// what it is, read for good, never fetched again, and no failure (brief 69).
    /// A transport failure is still a failure, asked again.
    #[test]
    fn a_document_no_longer_listed_is_kept_as_a_fact_and_never_a_failure() {
        let c = enrich_store();
        let gone = Fake { unlisted: true, ..fake(true, ("", ""), false) };
        assert!(filings_enrich_in(&c, src_of(DOC), DOC, &gone).is_ok(), "no failure");
        let row = sf::filing(&c, src_of(DOC), DOC).unwrap().unwrap();
        assert!(!row.unlisted_at.is_empty(), "kept with when it was found");
        assert!(row.enrich_final, "read for good");
        assert!(!row.subject.is_empty(), "named by what it is");
        assert!(filings_enrich_in(&c, src_of(DOC), DOC, &gone).is_ok());
        assert_eq!(gone.reads.get(), 1, "never fetched again");
        // the regulator not answering is no such fact
        struct Down;
        impl Readers for Down {
            fn summary_available(&self) -> bool { true }
            fn disclosures_available(&self) -> bool { true }
            fn enrichment(&self, _: &FiledDocument) -> Option<disclosures::Enrichment> { None }
            fn content(&self, _: &FiledDocument) -> disclosures::Fetched<(Vec<u8>, String)> { Err(disclosures::SourceError::Unavailable("SEDAR+ did not answer".into())) }
        }
        let c = enrich_store();
        assert!(filings_enrich_in(&c, src_of(DOC), DOC, &Down).is_err(), "a transport failure is a failure");
        assert!(sf::filing(&c, src_of(DOC), DOC).unwrap().unwrap().unlisted_at.is_empty());
    }

    const DOC: &str = "sedar:1";

    fn enrich_store() -> Connection {
        let c = store();
        replace(&c, "QNC", Regulator::Sedar, &[item(Regulator::Sedar, 1, "")]);
        c
    }

    fn stored(c: &Connection) -> (String, String, i64) {
        let row = sf::filing(c, src_of(DOC), DOC).unwrap().unwrap();
        (row.subject, row.summary, row.enrich_version.unwrap_or(0))
    }

    fn run(c: &Connection, model: bool, read: (&'static str, &'static str), final_: bool) -> (Value, usize) {
        let r = fake(model, read, final_);
        let out = v(filings_enrich_in(c, src_of(DOC), DOC, &r));
        (out, r.reads.get())
    }

    fn halves(c: &Connection) -> (String, String) {
        let s = stored(c);
        (s.0, s.1)
    }

    fn pair(a: &str, b: &str) -> (String, String) {
        (a.to_string(), b.to_string())
    }

    #[test]
    fn test_a_row_with_both_halves_is_not_read_again() {
        let c = enrich_store();
        run(&c, true, ("A title", "A sentence."), false);
        assert_eq!(halves(&c), pair("A title", "A sentence."));
        let (out, reads) = run(&c, true, ("other", "other."), false);
        assert_eq!(reads, 0);
        assert_eq!(out["subject"], "A title");
    }

    #[test]
    fn test_a_document_read_for_good_is_never_fetched_again_even_with_nothing_to_show() {
        let c = enrich_store();
        let (out, _) = run(&c, true, ("", ""), true);
        assert_eq!((f(&out, "subject"), f(&out, "summary")), pair("", ""));
        assert_eq!(stored(&c).2, ENRICH_VERSION, "stamped, so the row is done");
        for _ in 0..3 {
            let (_, reads) = run(&c, true, ("", ""), true);
            assert_eq!(reads, 0, "the document is not fetched again");
        }
    }

    #[test]
    fn test_a_document_read_for_good_while_no_model_was_up_is_still_done() {
        let c = enrich_store();
        run(&c, false, ("Exempt distribution of $1,500,000", "$1,500,000 distributed."), true);
        assert_eq!(halves(&c), pair("Exempt distribution of $1,500,000", "$1,500,000 distributed."));
        let (_, reads) = run(&c, true, ("other", "other."), true);
        assert_eq!(reads, 0, "a form needs no model, so a model arriving later changes nothing");
    }

    #[test]
    fn test_a_row_holding_only_a_title_is_read_again_for_its_summary() {
        let c = enrich_store();
        run(&c, true, ("A title", ""), false);
        assert_eq!(halves(&c), pair("A title", ""));
        let (out, reads) = run(&c, true, ("A title", "The sentence."), false);
        assert_eq!(reads, 1);
        assert_eq!(out["summary"], "The sentence.");
    }

    #[test]
    fn test_a_row_holding_only_a_summary_is_read_again_for_its_title() {
        let c = enrich_store();
        run(&c, true, ("", "A sentence."), false);
        assert_eq!(halves(&c), pair("", "A sentence."));
        let (out, reads) = run(&c, true, ("The title", "A sentence."), false);
        assert_eq!(reads, 1);
        assert_eq!(out["subject"], "The title");
    }

    #[test]
    fn test_reading_again_fills_what_is_missing_and_empties_nothing() {
        let c = enrich_store();
        run(&c, true, ("A title", ""), false);
        let (out, reads) = run(&c, true, ("", ""), false);
        assert_eq!(reads, 1);
        assert_eq!(out["subject"], "A title");
        assert_eq!(stored(&c).0, "A title");
    }

    #[test]
    fn test_the_first_read_under_the_current_logic_still_clears_an_older_junk_title() {
        let c = enrich_store();
        enrichment(&c, DOC, "00012345.pdf", "", 1);
        let (out, reads) = run(&c, true, ("", "A sentence."), false);
        assert_eq!(reads, 1);
        assert_eq!(out["subject"], "");
        assert_eq!(out["summary"], "A sentence.");
    }

    #[test]
    fn test_with_no_model_up_a_row_already_read_is_not_fetched_again() {
        let c = enrich_store();
        run(&c, true, ("A title", ""), false);
        let (out, reads) = run(&c, false, ("A title", "never asked"), false);
        assert_eq!(reads, 0);
        assert_eq!(out["summary"], "");
    }

    #[test]
    fn test_a_row_never_read_is_read_even_with_no_model() {
        let c = enrich_store();
        let (out, reads) = run(&c, false, ("A title", ""), false);
        assert_eq!(reads, 1);
        assert_eq!(out["subject"], "A title");
        assert_eq!(halves(&c), pair("A title", ""), "the title found is kept and shown");
    }

    #[test]
    fn test_a_row_that_found_no_model_waits_and_is_not_fetched_again_until_one_is_up() {
        let c = enrich_store();
        run(&c, false, ("A title", ""), false);
        for _ in 0..3 {
            let (_, reads) = run(&c, false, ("A title", ""), false);
            assert_eq!(reads, 0, "with no model up, the document is not fetched from the regulator again");
        }
        let row = sf::filing(&c, src_of(DOC), DOC).unwrap().unwrap();
        assert!(wants_reading(&row) && !row.enrich_final, "the row waits rather than finishing");
        let (out, reads) = run(&c, true, ("A title", "The sentence."), false);
        assert_eq!(reads, 1, "a model coming up is what reads it again");
        assert_eq!(out["summary"], "The sentence.");
    }

    #[test]
    fn test_a_half_filled_row_is_read_at_most_twice_with_a_model_up_then_settles() {
        for half in [("A title", ""), ("", "A sentence.")] {
            let c = enrich_store();
            let mut fetched = 0;
            for _ in 0..6 {
                fetched += run(&c, true, half, false).1;
            }
            assert_eq!(fetched, ENRICH_READS as usize, "two reads that could have answered, then no more");
            let row = sf::filing(&c, src_of(DOC), DOC).unwrap().unwrap();
            assert_eq!((row.subject.as_str(), row.summary.as_str()), half, "it settles with whichever half it has");
            assert!(row.enrich_final && !wants_reading(&row));
        }
    }

    #[test]
    fn test_reads_with_no_model_up_do_not_count_against_the_document() {
        let c = enrich_store();
        let mut fetched = 0;
        for _ in 0..4 {
            fetched += run(&c, false, ("A title", ""), false).1;
        }
        assert_eq!(fetched, 1);
        let mut with_model = 0;
        for _ in 0..4 {
            with_model += run(&c, true, ("A title", ""), false).1;
        }
        assert_eq!(with_model, ENRICH_READS as usize, "the bound is reads a model could have answered, however many came before");
    }

    #[test]
    fn test_reads_under_an_older_logic_do_not_count_under_this_one() {
        let c = enrich_store();
        run(&c, true, ("A title", ""), false);
        run(&c, true, ("A title", ""), false);
        assert!(sf::filing(&c, src_of(DOC), DOC).unwrap().unwrap().enrich_final);
        enrichment(&c, DOC, "A title", "", ENRICH_VERSION - 1);
        let mut fetched = 0;
        for _ in 0..4 {
            fetched += run(&c, true, ("A title", ""), false).1;
        }
        assert_eq!(fetched, ENRICH_READS as usize);
    }

    #[test]
    fn test_a_count_survives_the_list_being_read_again() {
        let c = enrich_store();
        run(&c, true, ("A title", ""), false);
        replace(&c, "QNC", Regulator::Sedar, &[item(Regulator::Sedar, 1, "")]);
        assert_eq!(sf::filing(&c, src_of(DOC), DOC).unwrap().unwrap().enrich_reads, 1);
    }

    // --- the reader that works behind the page

    #[test]
    fn test_the_background_reader_picks_a_titled_row_without_its_sentence() {
        let c = enrich_store();
        run(&c, false, ("A title", ""), false);
        let r = fake(true, ("A title", "The sentence."), false);
        let mut passed = HashSet::new();
        assert_eq!(read_one_in(&c, &[filer("QNC", Regulator::Sedar)], &mut passed, &r), Ok(Some(Ok(()))));
        assert_eq!(r.reads.get(), 1);
        assert_eq!(halves(&c), pair("A title", "The sentence."));
        assert_eq!(read_one_in(&c, &[filer("QNC", Regulator::Sedar)], &mut passed, &r), Ok(None), "nothing is left to read");
    }

    #[test]
    fn test_the_background_reader_ends_on_rows_it_cannot_move() {
        let c = enrich_store();
        run(&c, false, ("A title", ""), false);
        let r = fake(false, ("A title", ""), false);
        let mut passed = HashSet::new();
        let mut tries = 0;
        while read_one_in(&c, &[filer("QNC", Regulator::Sedar)], &mut passed, &r).unwrap().is_some() {
            tries += 1;
            assert!(tries < 5, "a row a read cannot move is passed over, not read in a loop");
        }
        assert_eq!(r.reads.get(), 0);
    }

    #[test]
    fn test_what_wants_reading() {
        let c = enrich_store();
        let row = || sf::filing(&c, src_of(DOC), DOC).unwrap().unwrap();
        assert!(wants_reading(&row()), "never read");
        enrichment(&c, DOC, "A title", "", ENRICH_VERSION);
        assert!(wants_reading(&row()), "a title without its sentence");
        enrichment(&c, DOC, "", "A sentence.", ENRICH_VERSION);
        assert!(wants_reading(&row()), "a sentence without its title");
        enrichment(&c, DOC, "A title", "A sentence.", ENRICH_VERSION);
        assert!(!wants_reading(&row()), "both halves");
        enrichment(&c, DOC, "A title", "A sentence.", ENRICH_VERSION - 1);
        assert!(wants_reading(&row()), "read under an older logic");
        sf::set_filing_enrichment(&c, src_of(DOC), DOC, Some("A title"), Some(""), Some(ENRICH_VERSION), Some(true), &now_iso()).unwrap();
        assert!(!wants_reading(&row()), "settled");
    }

    // --- WaitingForTheModelTest

    #[test]
    fn test_a_model_that_is_starting_is_waited_for_rather_than_the_read_wasted() {
        let c = enrich_store();
        let r = Fake { model: false, unlisted: false, status: "ready", wait: true, read: ("A title", "A sentence."), final_: false, reads: Cell::new(0), waits: Cell::new(0) };
        let out = v(filings_enrich_in(&c, src_of(DOC), DOC, &r));
        assert_eq!(r.waits.get(), 1);
        assert_eq!(out["summary"], "A sentence.");
        assert_eq!(stored(&c), ("A title".to_string(), "A sentence.".to_string(), ENRICH_VERSION));
    }

    #[test]
    fn test_a_model_that_never_comes_up_leaves_the_row_to_be_read_again() {
        let c = enrich_store();
        let r = Fake { model: false, unlisted: false, status: "off", wait: false, read: ("A title", ""), final_: false, reads: Cell::new(0), waits: Cell::new(0) };
        let out = v(filings_enrich_in(&c, src_of(DOC), DOC, &r));
        assert_eq!(out["subject"], "A title");
        let row = sf::filing(&c, src_of(DOC), DOC).unwrap().unwrap();
        assert!(wants_reading(&row) && !row.enrich_final && row.enrich_reads == 0, "the read did not count, and the row waits");
    }

    // --- RefreshKeepsWhatWasReadTest

    #[test]
    fn test_a_row_the_source_still_lists_keeps_its_subject_and_summary() {
        let c = store();
        replace(&c, "QNC", Regulator::Sedar, &[item(Regulator::Sedar, 1, ""), item(Regulator::Sedar, 2, "")]);
        enrichment(&c, "sedar:1", "A title", "A sentence.", 9);
        replace(&c, "QNC", Regulator::Sedar, &[item(Regulator::Sedar, 1, ""), item(Regulator::Sedar, 2, ""), item(Regulator::Sedar, 3, "")]);
        let row = sf::filing(&c, src_of("sedar:1"), "sedar:1").unwrap().unwrap();
        assert_eq!((row.subject, row.summary, row.enrich_version), ("A title".into(), "A sentence.".into(), Some(9)));
    }

    #[test]
    fn test_what_the_source_says_about_a_row_is_still_refreshed() {
        let c = store();
        replace(&c, "QNC", Regulator::Sedar, &[item(Regulator::Sedar, 1, "")]);
        enrichment(&c, "sedar:1", "A title", "A sentence.", 9);
        let mut moved = item(Regulator::Sedar, 1, "");
        moved.url = "https://www.sedarplus.ca/x?drmKey=fresh".into();
        replace(&c, "QNC", Regulator::Sedar, &[moved]);
        let row = sf::filing(&c, src_of("sedar:1"), "sedar:1").unwrap().unwrap();
        assert_eq!(row.doc.url, "https://www.sedarplus.ca/x?drmKey=fresh");
        assert_eq!(row.summary, "A sentence.");
    }

    #[test]
    fn test_a_row_the_source_no_longer_lists_goes() {
        let c = store();
        replace(&c, "QNC", Regulator::Sedar, &[item(Regulator::Sedar, 1, ""), item(Regulator::Sedar, 2, "")]);
        replace(&c, "QNC", Regulator::Sedar, &[item(Regulator::Sedar, 2, "")]);
        assert!(sf::filing(&c, src_of("sedar:1"), "sedar:1").unwrap().is_none());
        assert!(sf::filing(&c, src_of("sedar:2"), "sedar:2").unwrap().is_some());
    }

    #[test]
    fn test_another_sources_rows_are_untouched() {
        let c = store();
        replace(&c, "QNC", Regulator::Sedar, &[item(Regulator::Sedar, 1, "")]);
        replace(&c, "QNC", Regulator::Sec, &[item(Regulator::Sec, 1, "")]);
        enrichment(&c, "sec:1", "From EDGAR", "A sentence.", 9);
        replace(&c, "QNC", Regulator::Sedar, &[item(Regulator::Sedar, 1, "")]);
        assert_eq!(sf::filing(&c, src_of("sec:1"), "sec:1").unwrap().unwrap().subject, "From EDGAR");
    }
}
