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
use bagholder_store::feeds::{self as sf, FiledDocument, Filing, NewsItem, Regulator};
use bagholder_store::tables::{get_meta, set_meta};

use crate::app::{log, now_iso, now_unix, parse_instant, spawn, App, ENRICH_VERSION};
use crate::feeds::{base, conn, feed_failed, notice_settings, notices, sha1_hex12, today, went, DISCLOSURES, DISCLOSURES_READING, DISCLOSURES_STORED, RELEASES};
use crate::http::OkOr;
use crate::notify;

// ---------------------------------------------------------------------------
// disclosures
// ---------------------------------------------------------------------------

pub const FILINGS_STALE_HOURS: f64 = 24.0;
pub const FILINGS_SWEEP_EVERY_SEC: u64 = 300;
pub const FILINGS_HOLD_MAX_MIN: f64 = 120.0;
pub const FILINGS_HOLD_KEY: &str = "filings:held-since";
pub const FILINGS_SWEEP_AGE_MIN: f64 = 30.0;

/// How long a listing's filers stand before they are settled again: the SEC's
/// list and SEDAR+'s answer change when an issuer lists, renames or delists.
pub const FILERS_RECHECK_HOURS: f64 = 24.0;

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

/// The regulator a listing files with: the SEC for a US listing, SEDAR+ for a
/// Canadian one; none for any other (a coin, an index).
pub fn own_regulator(key: &ListingKey) -> Option<Regulator> {
    if key.us() {
        Some(Regulator::Sec)
    } else if key.canadian() {
        Some(Regulator::Sedar)
    } else {
        None
    }
}

/// The TSX's interlisted list, read again when the copy kept is a day old; a read
/// that fails keeps the last list and is said in the header until one goes through.
fn interlisted_current(app: &Arc<App>, c: &Connection, regulators: &dyn disclosures::Regulators) -> Result<(), String> {
    let read = sf::interlisted_read_at(c).map_err(|e| e.to_string())?;
    if age_hours(&read).is_some_and(|h| h < bagholder_market::interlisted::READ_EVERY_HOURS) {
        return Ok(());
    }
    match regulators.interlisted() {
        Ok((as_of, rows)) => {
            sf::replace_interlisted(c, &rows, &as_of, &now_iso()).map_err(|e| e.to_string())?;
            crate::feeds::feed_answered(app, INTERLISTED);
        }
        Err(e) => feed_failed(app, INTERLISTED, format!("The TSX's list of interlisted companies could not be read: {e}")),
    }
    Ok(())
}

/// The header's name for the interlisted list's failure.
const INTERLISTED: &str = "interlisted";

/// A listing's own filer settled again where it is a day old: the one its own
/// regulator lists it under, or why none settles.
fn settle_own(c: &Connection, key: &ListingKey, regulators: &dyn disclosures::Regulators) -> Result<(), String> {
    let Some(source) = own_regulator(key) else { return Ok(()) };
    let checked = sf::listing_filers_checked_at(c, key, source).map_err(|e| e.to_string())?;
    if age_hours(&checked).is_some_and(|h| h < FILERS_RECHECK_HOURS) {
        return Ok(());
    }
    let Some((_, found)) = regulators.filer(key, &today()) else { return Ok(()) };
    let now = now_iso();
    // a regulator that could not be asked leaves what was settled before standing
    let found = match found {
        Ok(found) => found,
        Err(e) => return Err(format!("{} could not be asked who files {}: {e}", source.as_str(), key.named())),
    };
    let by = if source == Regulator::Sec { sf::SettledBy::SecTicker } else { sf::SettledBy::SedarName };
    match found {
        Ok(id) => sf::settle_listing_filers(c, key, source, Ok(&[(id, by)]), &now),
        Err(why) => sf::settle_listing_filers(c, key, source, Err(&why), &now),
    }
    .map_err(|e| e.to_string())
}

/// The filers a listing's filings come from: its own regulator's, and where the
/// TSX's interlisted list pairs it with a listing in the other country, that
/// listing's own regulator's. Each settled again where it is a day old; a
/// regulator that cannot be asked leaves what was settled before.
pub fn settled_filers(app: &Arc<App>, c: &Connection, key: &ListingKey, regulators: &dyn disclosures::Regulators) -> Result<Vec<sf::Filer>, String> {
    let mut failed: Vec<String> = Vec::new();
    if let Err(e) = settle_own(c, key, regulators) {
        failed.push(e);
    }
    interlisted_current(app, c, regulators)?;
    if let Some(partner) = sf::interlisted_partner(c, key).map_err(|e| e.to_string())? {
        if let Err(e) = settle_own(c, &partner, regulators) {
            failed.push(e);
        }
        if let Some(source) = own_regulator(&partner) {
            // the partner's own filer is this listing's under the partner's regulator
            let theirs: Vec<(String, sf::SettledBy)> = sf::listing_filers(c, &partner)
                .map_err(|e| e.to_string())?
                .into_iter()
                .filter(|(f, _)| f.source == source)
                .map(|(f, _)| (f.id, sf::SettledBy::Interlisted))
                .collect();
            let now = now_iso();
            if theirs.is_empty() {
                let why = sf::listing_filer_miss(c, &partner, source).map_err(|e| e.to_string())?.map(|(w, _)| w).unwrap_or_default();
                if !why.is_empty() {
                    sf::settle_listing_filers(c, key, source, Err(&why), &now).map_err(|e| e.to_string())?;
                }
            } else {
                sf::settle_listing_filers(c, key, source, Ok(&theirs), &now).map_err(|e| e.to_string())?;
            }
        }
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
    went(app, DISCLOSURES, sweep_filings_in(app, &c, &book).map_err(|e| format!("The disclosures could not be swept: {e}"))).unwrap_or(0)
}

/// The sweep, the disclosures read and kept in the market cache (`c`), what is
/// told of them and the marks of what was met kept in the book. Each filer's
/// documents are one stream (`filings:<source>:<filer>`), so a filing two listings
/// of one company share is told once.
fn sweep_filings_in(app: &Arc<App>, c: &Connection, book: &bagholder_book::Book) -> Result<usize, String> {
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
        let wrote = refresh_filings(app, &key)?;
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
    let mut detail = if is_distribution_release(&head) { distribution_detail(app, key) } else { String::new() };
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

/// What a distribution release means for this listing, from the issuer's own
/// declared record: the amount just announced, when it goes ex and when it is
/// paid, and the one it replaces.
///
/// A release headline says only that distributions were announced; the figure
/// is what the holder wants, and reading it from the record rather than the
/// release's prose keeps it the same figure the Cashflow tab pays from.
pub fn distribution_detail(app: &Arc<App>, key: &ListingKey) -> String {
    let Some(f) = app.figures.get() else { return String::new() };
    // the release is the announcement; the record it comes from carries the figures, and it
    // is read now rather than when it is next due, so the notice is not a day behind it.
    // A notice that goes out without its figures says why in the header, until the next
    // one has them.
    let detail = read_record_for_notice(app, f, key)
        .map_err(|e| format!("{}'s declared distributions could not be read again: {e}", key.named()))
        .and_then(|read| f.book().map_err(|e| format!("The book could not be read for {}'s distributions: {e}", key.named())).and_then(|book| distribution_detail_in(&book, &read)));
    went(app, "distributions", detail).unwrap_or_default()
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

/// The payer's own declared record, as the book keeps it: the newest declaration
/// of the instruments given (the one listing a symbol names), its frequency where
/// one is stated, and the amount it replaces when that differs.
pub fn distribution_detail_in(book: &bagholder_book::Book, instruments: &[bagholder_core::InstrumentId]) -> Result<String, String> {
    let [id] = instruments else { return Ok(String::new()) };
    let (declared, frequencies) = match (book.declared(), book.frequencies()) {
        (Ok(d), Ok(f)) => (d, f),
        (Err(e), _) | (_, Err(e)) => return Err(format!("The declared distributions could not be read: {e}")),
    };
    let Some(read) = declared.get(id) else { return Ok(String::new()) };
    let mut items: Vec<&bagholder_book::facts::DeclaredRow> = read.items.iter().collect();
    items.sort_by(|a, b| b.ex_date.cmp(&a.ex_date));
    let Some(latest) = items.first() else { return Ok(String::new()) };
    let mut out = format!("{} a share", per_unit(&latest.amount));
    if let Some(f) = frequencies.get(id) {
        out += &format!(", {}", frequency_word(f.per_year));
    }
    out += &format!(" · ex {}", stamp_day(&latest.ex_date.to_string()));
    if let Some(p) = latest.pay_date {
        out += &format!(", paid {}", stamp_day(&p.to_string()));
    }
    if let Some(was) = items.get(1) {
        if was.amount != latest.amount {
            out += &format!(" · was {}", per_unit(&was.amount));
        }
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
fn notice_extra<T: Notable>(key: &ListingKey, exchange: &str, rows: &[T]) -> bagholder_store::feeds::NotificationExtra {
    let (at, link) = (notice_moment(rows), notice_link(rows));
    bagholder_store::feeds::NotificationExtra {
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
fn read_one_unread(app: &Arc<App>, passed: &mut HashSet<(Regulator, String)>) -> Result<Option<Result<(), String>>, String> {
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
fn read_one_of(app: &Arc<App>, only: Option<&ListingKey>, passed: &mut HashSet<(Regulator, String)>) -> Result<Option<Result<(), String>>, String> {
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
fn read_one_in(c: &Connection, filers: &[sf::Filer], passed: &mut HashSet<(Regulator, String)>, readers: &dyn Readers) -> Result<Option<Result<(), String>>, String> {
    let best = sf::filings_of(c, filers)
        .map_err(|e| e.to_string())?
        .into_iter()
        .filter(|r| wants_reading(r) && !passed.contains(&(r.doc.source, r.doc.id.clone())))
        .max_by(|a, b| a.doc.date.cmp(&b.doc.date));
    let Some(before) = best else { return Ok(None) };
    let (source, id) = (before.doc.source, before.doc.id.clone());
    if let Err(e) = filings_enrich_in(c, source, &id, readers) {
        passed.insert((source, id.clone()));
        return Ok(Some(Err(format!("The {} document {id} could not be read: {e}", source.as_str()))));
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
    for filer in &filers {
        let held = sf::filings_of(c, std::slice::from_ref(filer)).map_err(|e| e.to_string())?.len();
        let outcome = match regulators.documents(filer, 200) {
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
                disclosures::SourceError::Other(m) => SourceOutcome { available: true, matched: false, filer: true, count: 0, error: m },
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
    {
        let mut r = app.feeds.reading_now.lock().unwrap_or_else(|e| e.into_inner());
        if ids.is_empty() { r.remove(&key.to_string()); } else { r.insert(key.to_string(), ids); }
    }
    app.events.signal(); // the rows still to be read wear the shimmer
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
            let mut tried: HashSet<(Regulator, String)> = HashSet::new();
            while app.events.watched(&doc) && !app.stopping() {
                let left = conn(&app).and_then(|c| filers_kept(&c, &key).and_then(|f| sf::filings_of(&c, &f).map_err(|e| e.to_string())));
                let Some(left) = went(&app, DISCLOSURES_STORED, left.map_err(|e| format!("The disclosures held could not be read: {e}"))) else { break };
                let mut left: Vec<Filing> = left.into_iter()
                    .filter(|r| wants_reading(r) && !tried.contains(&(r.doc.source, r.doc.id.clone())))
                    .collect();
                left.sort_by(|a, b| b.doc.date.cmp(&a.doc.date));
                let Some(next) = left.first().map(|r| (r.doc.source, r.doc.id.clone())) else { break };
                set_reading(&app, &key, left.iter().map(|r| r.doc.id.clone()).collect());
                // waits for the local model itself when one is coming up; a document that
                // could not be read is said in the header until one is
                went(&app, DISCLOSURES_READING, filings_enrich_result(&app, next.0, &next.1).map_err(|e| format!("{}'s document {} could not be read: {e}", key.named(), next.1)));
                tried.insert(next);
                if app.wait(Duration::from_millis(150)) { // a person's pace at the source
                    break;
                }
            }
            set_reading(&app, &key, vec![]);
        });
    });
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
fn filings_enrich_result(app: &Arc<App>, source: Regulator, doc_id: &str) -> Result<Enriched, String> {
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
        Err(e) => EnrichAnswer::Refused(OkOr::err(e)),
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
pub fn filings_enrich_in(c: &Connection, source: Regulator, doc_id: &str, r: &dyn Readers) -> Result<Enriched, String> {
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
        Err(e) => return Err(e.to_string()),
    };
    if data.is_empty() {
        return Err("the document could not be read".to_string());
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
    fn v<T: serde::Serialize>(r: Result<T, String>) -> Value {
        match r {
            Ok(x) => serde_json::to_value(x).unwrap(),
            Err(e) => json!({"ok": false, "error": e}),
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
    }

    impl Regs {
        fn new() -> Regs {
            Regs { filers: vec![], documents: vec![], pairs: vec![], asked: Cell::new(0) }
        }
    }

    impl disclosures::Regulators for Regs {
        fn filer(&self, listing: &ListingKey, _: &str) -> Option<(Regulator, disclosures::Fetched<Result<String, String>>)> {
            self.asked.set(self.asked.get() + 1);
            let source = own_regulator(listing)?;
            let found = self.filers.iter().find(|(k, _)| k == listing).map(|(_, f)| f.clone()).unwrap_or_else(|| Err(format!("no filer for {listing}")));
            Some((source, Ok(found)))
        }
        fn documents(&self, filer: &sf::Filer, _: usize) -> disclosures::Fetched<Vec<FiledDocument>> {
            self.documents.iter().find(|(f, _)| f == filer).map(|(_, d)| d.clone()).unwrap_or(Ok(vec![]))
        }
        fn interlisted(&self) -> disclosures::Fetched<(String, Vec<sf::Interlisted>)> {
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
            Ok((b"%PDF-1.4 body".to_vec(), "application/pdf".into()))
        }
        fn enrich_document_of(&self, _: &str, _: &str, _: &[u8], _: &str) -> disclosures::Enrichment {
            disclosures::Enrichment { subject: self.read.0.to_string(), summary: self.read.1.to_string(), final_: self.final_ }
        }
    }

    fn fake(model: bool, read: (&'static str, &'static str), final_: bool) -> Fake {
        Fake { model, status: if model { "ready" } else { "off" }, wait: false, read, final_, reads: Cell::new(0), waits: Cell::new(0) }
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
        let r = Fake { model: false, status: "ready", wait: true, read: ("A title", "A sentence."), final_: false, reads: Cell::new(0), waits: Cell::new(0) };
        let out = v(filings_enrich_in(&c, src_of(DOC), DOC, &r));
        assert_eq!(r.waits.get(), 1);
        assert_eq!(out["summary"], "A sentence.");
        assert_eq!(stored(&c), ("A title".to_string(), "A sentence.".to_string(), ENRICH_VERSION));
    }

    #[test]
    fn test_a_model_that_never_comes_up_leaves_the_row_to_be_read_again() {
        let c = enrich_store();
        let r = Fake { model: false, status: "off", wait: false, read: ("A title", ""), final_: false, reads: Cell::new(0), waits: Cell::new(0) };
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
