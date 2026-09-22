//! The documents a page can be sent while it shows them (`events`): how each is
//! read from what the app holds, and what is worth doing once when one opens.
//!
//! Reading a document never waits on the network: it says what is stored. Getting
//! fresher data is a job started in the background when the document opens (and by
//! whatever keeps that data current while someone is looking); when the job commits
//! what it found, the stream sends the difference.

use std::sync::Arc;

use serde_json::Value;

use crate::app::App;

/// The open ticket's quote, by document key. Wealthsimple pushes nothing, so while
/// a ticket is open its quote is asked for again every few seconds -- here, by the
/// server, for as long as some page shows that ticket and not a moment longer. The
/// page is sent only what moved in it (usually the bid, the ask and the last).
fn quotes() -> &'static std::sync::Mutex<std::collections::HashMap<String, Value>> {
    static Q: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, Value>>> = std::sync::OnceLock::new();
    Q.get_or_init(Default::default)
}

const QUOTE_EVERY: std::time::Duration = std::time::Duration::from_secs(5);

fn quote_shown(app: Arc<App>, key: String) {
    crate::app::spawn("bagholder-ticket-quote", move || {
        app.clone().single_flight(&key.clone(), (), || {
            let q = &key["quote:".len()..];
            while crate::events::watched(&key) && !app.stopping() {
                let v = crate::orders::ticket_quote(&app, &one(q, "symbol"), &one(q, "security"), &one(q, "account"), &one(q, "exchange"));
                quotes().lock().unwrap_or_else(|e| e.into_inner()).insert(key.clone(), v);
                crate::events::signal();
                if app.wait(QUOTE_EVERY) {
                    break;
                }
            }
            quotes().lock().unwrap_or_else(|e| e.into_inner()).remove(&key);
        });
    });
}

/// The document `key` as it stands, or `None` for a key nothing answers to.
pub fn read(app: &Arc<App>, key: &str, _params: &Value) -> Option<Value> {
    match key {
        "orders" => Some(crate::orders::orders_payload(app, false)),
        "shorts" => Some(crate::feeds::shorts_feed(app)),
        // the bell: the newest notifications and how many are unread. A new one reaches
        // the page as a row inserted; marking them read, as `readAt` set on those rows
        "notifications" => {
            let conn = app.open().ok()?;
            Some(serde_json::json!({
                "rows": bagholder_store::feeds::list_notifications(&conn, 0, "", false, 50, true).ok()?,
                "unread": bagholder_store::feeds::unread_notifications(&conn).ok()?,
            }))
        }
        // `history:<the chart's own query>`: only whether its intraday bars are still
        // being read -- the bars themselves are fetched once, when this says they are in
        // `filings:<symbol=…&name=…&exchange=…&currency=…>`: one listing's disclosures
        k if k.starts_with("filings:") => Some(crate::feeds::filings_stored(app, &one(&k["filings:".len()..], "symbol"))),
        // `filings-feed:<scope>`: the newest disclosures across the listings in scope;
        // `reading` says whether anything is reading their documents (a local model is up)
        k if k.starts_with("filings-feed:") => {
            let mut feed = crate::feeds::filings_feed(app, &k["filings-feed:".len()..], 200);
            feed["reading"] = serde_json::json!(bagholder_market::enrich::summary_available());
            Some(feed)
        }
        // `quote:<symbol=…&security=…&account=…&exchange=…>`: nothing until the first answer
        // `fear:<index>`: the fear and greed meter
        k if k.starts_with("fear:") => Some(crate::feeds::fear_stored(app, &k["fear:".len()..])),
        k if k.starts_with("quote:") => quotes().lock().unwrap_or_else(|e| e.into_inner()).get(k).cloned(),
        k if k.starts_with("history:") => Some(serde_json::json!({"pending": crate::feeds::history_pending(app, &k["history:".len()..])})),
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
