//! What the order code shares: the session, the one way it reads Wealthsimple, the
//! words for a failed call, notices, and whether the checks run.

use std::sync::Arc;

use bagholder_ws::session::CallError;
use serde_json::Value;

use crate::app::App;
use crate::notify;
use crate::session::{ensure_fresh_token, load_session};

pub(crate) use crate::app::log;

/// What an order is refused with when no one is signed in.
pub(crate) const NOT_CONNECTED: &str = "Not connected.";

/// The session orders are sent and read with, its token refreshed when due:
/// `NOT_CONNECTED` where no one is signed in, and a saved login that cannot be
/// read is that error, never "not signed in".
pub(crate) fn ticket_session(app: &Arc<App>) -> Result<bagholder_ws::session::Session, String> {
    #[cfg(test)]
    {
        return app.orders.seam.session.lock().unwrap_or_else(|e| e.into_inner()).clone().ok_or_else(|| NOT_CONNECTED.to_string());
    }
    #[allow(unreachable_code)]
    let sess = load_session(app)?.ok_or_else(|| NOT_CONNECTED.to_string())?;
    if sess.access_token.is_empty() {
        return Err(NOT_CONNECTED.into());
    }
    ensure_fresh_token(app, Some(sess.clone()));
    match load_session(app)? {
        Some(v) if !v.access_token.is_empty() || !v.refresh_token.is_empty() => Ok(v),
        _ => Ok(sess),
    }
}

/// The order mutations, which only the gate sends.
pub const MUTATIONS: [&str; 3] = ["SoOrdersOrderCreate", "SoOrdersOrderCancel", "SoOrdersOrderModify"];

/// One read of Wealthsimple, its answer read as `T`: the order code's only way there
/// that is not the gate. No answer is a failure, as the client itself has it, and so is
/// one that is not the shape asked for. An order mutation is refused here: it leaves
/// through the gate or not at all.
pub(crate) fn gql_as<T: serde::de::DeserializeOwned>(#[cfg_attr(test, allow(unused_variables))] app: &Arc<App>, sess: &bagholder_ws::session::Session, op: &str, vars: Value) -> Result<T, CallError> {
    if MUTATIONS.contains(&op) {
        return Err(CallError::Failed(format!("{op} leaves through the gate")));
    }
    #[cfg(test)]
    {
        let _ = sess;
        let g = app.orders.seam.gql.lock().unwrap_or_else(|e| e.into_inner()).clone();
        return match g {
            Some(g) => read_answer(op, g(op, &vars)?),
            None => Err(CallError::Failed(format!("{op}: no network in tests"))),
        };
    }
    #[cfg(not(test))]
    {
        let home = app.ws_home();
        bagholder_ws::session::Client { home: &home }.graphql::<T>(sess, op, &vars, None)
    }
}

/// An answer's `data` read as `T`, by the client's own rule (`Client::graphql`).
#[cfg(test)]
pub(crate) fn read_answer<T: serde::de::DeserializeOwned>(op: &str, data: Value) -> Result<T, CallError> {
    if data.is_null() {
        return Err(CallError::Failed(format!("graphql failed: {op}")));
    }
    T::deserialize(data).map_err(|e| CallError::Failed(format!("{op}: unreadable answer: {e}")))
}

pub(crate) fn err_text(e: &CallError) -> String {
    let t = e.to_string();
    if t.is_empty() {
        "the call failed".into()
    } else {
        t
    }
}

/// Tell the person: a notice of `kind`, said once under `key`.
pub(crate) fn emit(app: &Arc<App>, kind: &str, key: &str, title: &str, body: &str) {
    // a notice that could not be recorded is said in the header until one is
    notify::tell(app, kind, key, title, body, None);
}

/// Whether the order and bracket checks run now: while the app holds an access token
/// Wealthsimple issued that has not expired. A refresh that failed while the token
/// still has time does not stop them, nor does a sync in progress: a stop is watched
/// every few seconds whatever else reads Wealthsimple (`SPEC.md` §4, Brackets after
/// the fill; brief 15 §2).
pub(crate) fn orders_can_run(app: &Arc<App>) -> bool {
    watch_blocked(app).is_none()
}

/// Why the checks cannot run now, in the person's words; none while they can.
pub(crate) fn watch_blocked(app: &Arc<App>) -> Option<String> {
    #[cfg(test)]
    let held: Result<Option<bagholder_ws::session::Session>, String> = Ok(app.orders.seam.session.lock().unwrap_or_else(|e| e.into_inner()).clone());
    #[cfg(not(test))]
    let held = crate::session::load_session(app);
    let sess = match held {
        Ok(Some(s)) if !s.access_token.is_empty() => s,
        Ok(_) => return Some("Wealthsimple is not signed in".into()),
        Err(e) => return Some(e),
    };
    match bagholder_ws::session::expires_at_unix(&sess) {
        Some(at) if at <= crate::app::now_unix() => Some("the Wealthsimple sign-in has expired; sign in again".into()),
        _ => None,
    }
}

/// The failures the order code has standing, for the header (`SPEC.md` §1: every
/// failure is said until its own next success): the brackets' quote, and each
/// bracket stopped by its guard.
pub fn order_failures(app: &Arc<App>) -> Vec<String> {
    let mut out: Vec<String> = app.orders.quote_problem.lock().unwrap_or_else(|e| e.into_inner()).said().into_iter().collect();
    out.extend(app.orders.watch_problem.lock().unwrap_or_else(|e| e.into_inner()).said());
    out.extend(app.orders.bracket_trouble.lock().unwrap_or_else(|e| e.into_inner()).values().filter_map(|c| c.said()));
    if let Some(f) = app.figures.get() {
        match f.book().and_then(|b| b.live_brackets().map_err(|e| e.to_string())) {
            Ok(live) => {
                for sb in live.iter().filter(|b| b.bracket.phase == bagholder_core::bracket::Phase::Halted) {
                    out.push(format!("The bracket on {} is stopped: {}; nothing more is sent until you change or cancel it", sb.place.symbol, sb.bracket.why.clone().unwrap_or_default()));
                }
            }
            Err(e) => out.push(format!("The brackets could not be read: {e}")),
        }
    }
    out
}
