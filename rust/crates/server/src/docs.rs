//! The documents a page can be sent while it shows them (`events`): how each is
//! read from what the app holds, and what is worth doing once when one opens.
//!
//! Reading a document never waits on the network: it says what is stored. Getting
//! fresher data is a job started in the background when the document opens (and by
//! whatever keeps that data current while someone is looking); when the job commits
//! what it found, the stream sends the difference.
//!
//! Each document is its own typed struct; `Doc` is the enum of all of them, one
//! value of it in `Feed::sent_docs` per key a page is showing. Its own `Diff` is
//! written by hand rather than derived: the same variant is compared field by
//! field by that variant's own `Diff`, and a key never changes shape once opened,
//! so the mismatched-variant arm below is not something a page ever sees.

use std::sync::Arc;

use serde::Serialize;
use serde_json::Value;

use bagholder_diff_derive::Diff;
use bagholder_model::patch::Diff;
use ts_rs::TS;

use crate::app::App;
use crate::feeds::{FearDoc, FilingsDoc, FilingsFeed, ShortsFeed};
use crate::notify::NotificationsDoc;
use crate::orders::OrdersDoc;

/// The open ticket's quote, by document key. Wealthsimple pushes nothing, so while
/// a ticket is open its quote is asked for again every few seconds -- here, by the
/// server, for as long as some page shows that ticket and not a moment longer. The
/// page is sent only what moved in it (usually the bid, the ask and the last).
#[derive(Default)]
pub struct DocsState {
    quotes: std::sync::Mutex<std::collections::HashMap<String, Value>>,
}

const QUOTE_EVERY: std::time::Duration = std::time::Duration::from_secs(5);

fn quote_shown(app: Arc<App>, key: String) {
    crate::app::spawn("bagholder-ticket-quote", move || {
        app.clone().single_flight(&key.clone(), (), || {
            let q = &key["quote:".len()..];
            while app.events.watched(&key) && !app.stopping() {
                let v = crate::orders::ticket_quote(&app, &one(q, "symbol"), &one(q, "security"), &one(q, "account"), &one(q, "exchange"));
                app.docs.quotes.lock().unwrap_or_else(|e| e.into_inner()).insert(key.clone(), v);
                app.events.signal();
                if app.wait(QUOTE_EVERY) {
                    break;
                }
            }
            app.docs.quotes.lock().unwrap_or_else(|e| e.into_inner()).remove(&key);
        });
    });
}

/// `filings:<symbol>`: a listing's disclosures, or the store's own refusal --
/// answered as `filings_stored` returns it, never a JSON error shape improvised
/// on top.
#[derive(Clone, Debug, Serialize, TS)]
#[serde(untagged)]
pub enum FilingsAnswer {
    Ok(FilingsDoc),
    Err {
        #[ts(type = "false")]
        ok: bool,
        error: String,
    },
}

impl Diff for FilingsAnswer {
    fn diff(&self, new: &Self, path: &mut Vec<Value>, ops: &mut Vec<Value>) {
        match (self, new) {
            (FilingsAnswer::Ok(a), FilingsAnswer::Ok(b)) => a.diff(b, path, ops),
            _ => bagholder_model::patch::as_json(self, new, path, ops),
        }
    }
}

/// `history:<the chart's own query>`: only whether its intraday bars are still
/// being read -- the bars themselves are fetched once, when this says they are in.
#[derive(Clone, Debug, Serialize, TS, Diff)]
pub struct HistoryPending {
    pub pending: bool,
}

/// One document a page can watch, whole once and then by what changes in it.
#[derive(Clone, Debug, Serialize)]
#[serde(untagged)]
pub enum Doc {
    Orders(OrdersDoc),
    Shorts(ShortsFeed),
    /// The bell: the newest notifications and how many are unread. A new one
    /// reaches the page as a row inserted; marking them read, as `readAt` set
    /// on those rows.
    Notifications(NotificationsDoc),
    Filings(FilingsAnswer),
    /// The newest disclosures across a set of listings; `reading` says whether
    /// anything is reading their documents (a local model is up).
    FilingsFeed(FilingsFeed),
    Fear(FearDoc),
    /// `quote:<symbol=…&security=…&account=…&exchange=…>`: nothing until the
    /// first answer.
    Quote(Value),
    History(HistoryPending),
}

impl Diff for Doc {
    fn diff(&self, new: &Self, path: &mut Vec<Value>, ops: &mut Vec<Value>) {
        use Doc::*;
        match (self, new) {
            (Orders(a), Orders(b)) => a.diff(b, path, ops),
            (Shorts(a), Shorts(b)) => a.diff(b, path, ops),
            (Notifications(a), Notifications(b)) => a.diff(b, path, ops),
            (Filings(a), Filings(b)) => a.diff(b, path, ops),
            (FilingsFeed(a), FilingsFeed(b)) => a.diff(b, path, ops),
            (Fear(a), Fear(b)) => a.diff(b, path, ops),
            (Quote(a), Quote(b)) => a.diff(b, path, ops),
            (History(a), History(b)) => a.diff(b, path, ops),
            // a key's document never actually changes shape once opened: kept only
            // so a mismatch here is a whole-object diff (as the untyped differ gave
            // two unlike objects), never a panic
            _ => bagholder_model::patch::as_json(self, new, path, ops),
        }
    }
}

/// The document `key` as it stands, or `None` for a key nothing answers to.
pub fn read(app: &Arc<App>, key: &str) -> Option<Doc> {
    match key {
        "orders" => Some(Doc::Orders(crate::orders::orders_doc(app, false))),
        "shorts" => Some(Doc::Shorts(crate::feeds::shorts_feed(app))),
        "notifications" => {
            let conn = app.open().ok()?;
            Some(Doc::Notifications(NotificationsDoc {
                rows: bagholder_store::feeds::list_notifications(&conn, 0, "", false, 50, true).ok()?,
                unread: bagholder_store::feeds::unread_notifications(&conn).ok()?,
            }))
        }
        // `filings:<symbol=…&name=…&exchange=…&currency=…>`: one listing's disclosures
        k if k.starts_with("filings:") => Some(Doc::Filings(match crate::feeds::filings_stored(app, &one(&k["filings:".len()..], "symbol")) {
            Ok(d) => FilingsAnswer::Ok(d),
            Err(e) => FilingsAnswer::Err { ok: false, error: e },
        })),
        // `filings-feed:<scope>`: the newest disclosures across the listings in scope
        k if k.starts_with("filings-feed:") => Some(Doc::FilingsFeed(crate::feeds::filings_feed(app, &k["filings-feed:".len()..], 200))),
        // `fear:<index>`: the fear and greed meter
        k if k.starts_with("fear:") => Some(Doc::Fear(crate::feeds::fear_stored(app, &k["fear:".len()..]))),
        k if k.starts_with("quote:") => app.docs.quotes.lock().unwrap_or_else(|e| e.into_inner()).get(k).cloned().map(Doc::Quote),
        k if k.starts_with("history:") => Some(Doc::History(HistoryPending { pending: crate::feeds::history_pending(app, &k["history:".len()..]) })),
        _ => None,
    }
}

/// A page has just started showing `key`.
pub fn opened(app: &Arc<App>, key: &str) {
    match key {
        "orders" => {
            crate::orders::kick_orders_refresh(app);
        }
        k if k.starts_with("fear:") => crate::feeds::fear_shown(app.clone(), k.to_string(), k["fear:".len()..].to_string()),
        k if k.starts_with("quote:") => quote_shown(app.clone(), k.to_string()),
        k if k.starts_with("filings:") => {
            let q = &k["filings:".len()..];
            crate::feeds::filings_shown(app.clone(), k.to_string(), one(q, "symbol"), one(q, "name"), one(q, "exchange"), one(q, "currency"));
        }
        _ => {}
    }
}

/// One value of a query string, decoded.
fn one(query: &str, name: &str) -> String {
    crate::feeds::qs_one(query, name)
}
