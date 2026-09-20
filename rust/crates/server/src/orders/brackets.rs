//! The bracket engine: the stop and the target held for an entry, placed when it
//! fills, kept to the shares it filled, and ended when one of them exits.

use super::*;

// ---------------------------------------------------------------------------
// brackets
// ---------------------------------------------------------------------------

pub const BRACKET_POLL_SEC: u64 = 5;
pub(super) const BRACKET_RETRY_SEC: [i64; 4] = [60, 300, 900, 3600];
pub(super) const TRAIL_MIN_MOVE: f64 = 0.005;
pub(super) const TARGET_BACK_OFF: f64 = 0.01;
pub(super) const BRACKET_ROLL_SEC: f64 = 7.0 * 86400.0;
pub(super) const BRACKET_ROLL_LAST_SEC: f64 = 2.0 * 86400.0;
pub(super) const GTC_DAYS: i64 = 90;
pub(super) const BRACKET_TIF: &str = "UNTIL_CANCEL";
pub(super) const BRACKET_ENDED_QUIETLY: [&str; 5] = ["stopped", "target", "cancelled by the user", "both legs removed", "sold from the ticket"];

pub(super) static BRACKET_LOCK: AtomicBool = AtomicBool::new(false);

pub fn bracket_said() -> &'static Mutex<HashSet<String>> {
    static SAID: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    SAID.get_or_init(|| Mutex::new(HashSet::new()))
}

pub fn stop_allowed_cache() -> &'static Mutex<HashMap<String, bool>> {
    static C: OnceLock<Mutex<HashMap<String, bool>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(HashMap::new()))
}

/// A bracket that still has work to do stands at one of these.
pub(super) const BRACKET_LIVE_ST: [BracketStatus; 6] =
    [BracketStatus::Waiting, BracketStatus::Armed, BracketStatus::Firing, BracketStatus::TargetPlaced, BracketStatus::Stopping, BracketStatus::Closing];

/// Resting at Wealthsimple: it can still fill, and it can be cancelled.
pub(super) fn resting(o: &Order) -> bool {
    matches!(o.status, OrderStatus::Sent | OrderStatus::Pending)
}

/// Resting, or on its way out: not yet known to be gone.
pub(super) fn in_flight(o: &Order) -> bool {
    o.status.is_live()
}

/// A bracket whose exits may be working: past waiting, and not yet winding down.
pub(super) fn in_play(b: &Bracket) -> bool {
    matches!(b.status, BracketStatus::Armed | BracketStatus::Firing | BracketStatus::TargetPlaced | BracketStatus::Stopping)
}

pub(super) fn release_shares(b: &Bracket, sold: f64) {
    let remaining = round_half_even(b.quantity.unwrap_or(0.0) - sold, 6);
    for oid in [&b.sl_order_id, &b.tp_order_id] {
        let err = cancel_exit(oid);
        if !err.is_empty() {
            log(&format!("bagholder bracket: {} for {}: cancel of {} refused: {}", b.id, b.symbol, if oid.is_empty() { "None" } else { oid.as_str() }, err));
        }
    }
    patch_bracket(&b.id, BracketPatch {
        quantity: Some(Some(remaining)), sl_order_id: Some(String::new()), tp_order_id: Some(String::new()),
        status: Some(BracketStatus::Armed), error: Some(String::new()), attempts: Some(0), ..BracketPatch::default()
    });
    log(&format!(
        "bagholder bracket: {} for {}: {} of its shares sold from the ticket; the stop is placed again on the {} left",
        b.id, b.symbol, qty_text(sold), qty_text(remaining)
    ));
}

pub(super) fn await_cancels(b: &Bracket, seconds: u32) {
    if !orders_live() {
        return;
    }
    for _ in 0..seconds {
        let open: Vec<Order> = own_exit_rows(b).into_iter().filter(in_flight).collect();
        if open.is_empty() {
            return;
        }
        for o in &open {
            refresh_orders(&o.id);
        }
        #[cfg(not(test))]
        std::thread::sleep(Duration::from_secs(1));
    }
}

/// The bracket an entry asked for, waiting for the entry to fill.
pub fn create_bracket(entry: &Order) -> Bracket {
    let sl = entry.stop_loss.clone().unwrap_or_default();
    let tp = entry.take_profit.clone().unwrap_or_default();
    let b = Bracket {
        id: format!("bracket-{}", uuid4()),
        order_id: entry.id.clone(),
        account_id: entry.account_id.clone(),
        security_id: entry.security_id.clone(),
        symbol: entry.symbol.clone(),
        currency: entry.currency.clone(),
        quantity: entry.quantity,
        tif: BRACKET_TIF.into(),
        sl_kind: sl.kind,
        sl_price: sl.price,
        sl_trail: sl.trail,
        sl_trail_unit: if sl.trail_unit.is_set() { sl.trail_unit } else { TrailUnit::Pct },
        tp_price: tp.price,
        status: BracketStatus::Waiting,
        ..Bracket::default()
    };
    must(so::typed::insert_bracket(&db(), &b, &now_iso()));
    bracket(&b.id).unwrap_or(b)
}

/// Bracket test seams: `stop_allowed`, `cancel_order`, the said lines and the caches.
#[cfg(test)]
pub mod bracket_seam {
    use super::*;
    pub static STOP_ALLOWED: Mutex<Option<bool>> = Mutex::new(None);
    pub static CANCEL_ORDER: Mutex<Option<Value>> = Mutex::new(None);
    pub static SAID: Mutex<Vec<String>> = Mutex::new(Vec::new());
    pub fn reset() {
        *STOP_ALLOWED.lock().unwrap_or_else(|e| e.into_inner()) = None;
        *CANCEL_ORDER.lock().unwrap_or_else(|e| e.into_inner()) = None;
        SAID.lock().unwrap_or_else(|e| e.into_inner()).clear();
        bracket_said().lock().unwrap_or_else(|e| e.into_inner()).clear();
        stop_allowed_cache().lock().unwrap_or_else(|e| e.into_inner()).clear();
    }
}

pub(super) fn say_once(key: String, line: &str) {
    if !bracket_said().lock().unwrap().insert(key) {
        return;
    }
    #[cfg(test)]
    bracket_seam::SAID.lock().unwrap_or_else(|e| e.into_inner()).push(line.to_string());
    log(line);
}

/// How far under the high a trailing stop sits, at `price`.
pub(super) fn trail_distance(b: &Bracket, price: f64) -> Option<f64> {
    if b.sl_kind != SlKind::Trail || !some(b.sl_trail) {
        return None;
    }
    let t = b.sl_trail.unwrap_or(0.0);
    Some(if b.sl_trail_unit == TrailUnit::Pct { price * t / 100.0 } else { t })
}

/// The sell a bracket places for a leg: the order, and what Wealthsimple is sent.
pub(super) fn exit_body(b: &Bracket, kind: OrderType, price: Option<f64>, role: Role) -> Result<(Order, Value), String> {
    let ticket = Ticket {
        symbol: b.symbol.clone(),
        security_id: b.security_id.clone(),
        account_id: b.account_id.clone(),
        side: "SELL".into(),
        kind: kind.as_str().into(),
        tif: Some(BRACKET_TIF.into()),
        quantity: b.quantity,
        limit_price: if kind == OrderType::Limit { price } else { None },
        stop_price: if kind == OrderType::Stop { price } else { None },
        currency: Some(b.currency.clone()),
        ..Ticket::default()
    };
    let (mut row, req) = ticket_order(&ticket)?;
    row.role = role;
    row.parent_id = b.order_id.clone();
    Ok((row, req))
}

/// (order id, error).
pub(super) fn place_exit(b: &Bracket, kind: OrderType, price: Option<f64>, role: Role) -> (String, String) {
    let (mut row, req) = match exit_body(b, kind, price, role) {
        Ok(x) => x,
        Err(e) => return (String::new(), e),
    };
    if !orders_live() {
        let key = format!("{}|{}|{}", b.id, role, rp(price.map(|p| round_half_even(p, 4))));
        say_once(key, &format!("bagholder bracket (orders are off, not placed): {} {} for {}: {}", role, kind, b.symbol, bagholder_store::tables::json_text_sorted(&req)));
        return (String::new(), String::new());
    }
    let r = submit_order(&mut row, &req);
    if !tr(&r, "ok") {
        let e = f(&r, "error");
        return (String::new(), if e.is_empty() { "not sent".into() } else { e });
    }
    (f(&r, "id"), String::new())
}

/// Cancel an exit that rests; nothing to do, and no error, for one that does not.
pub(super) fn cancel_exit(order_id: &str) -> String {
    if order_id.is_empty() {
        return String::new();
    }
    match order(order_id) {
        Some(row) if resting(&row) => {}
        _ => return String::new(),
    }
    let r = cancel_order(order_id);
    let e = f(&r, "error");
    if tr(&r, "ok") || e.contains("not open") {
        return String::new();
    }
    if e.is_empty() {
        "cancel failed".into()
    } else {
        e
    }
}

/// The order a bracket holds for a leg; failing that, the newest it ever placed for it.
pub(super) fn exit_row(b: &Bracket, role: Role) -> Option<Order> {
    let held = if role == Role::Stop { &b.sl_order_id } else { &b.tp_order_id };
    if !held.is_empty() {
        if let Some(row) = order(held) {
            return Some(row);
        }
    }
    orders_all().into_iter().find(|o| o.parent_id == b.order_id && o.role == role)
}

pub(super) fn retry_wait(attempts: i64) -> i64 {
    BRACKET_RETRY_SEC[(attempts.min(BRACKET_RETRY_SEC.len() as i64) - 1).max(0) as usize]
}

pub(super) fn may_retry(b: &Bracket) -> bool {
    if b.attempts == 0 {
        return true;
    }
    let wait = retry_wait(b.attempts);
    match parse_z(&b.updated_at) {
        Some(t) => now_unix() - t as f64 >= wait as f64,
        None => true,
    }
}

pub(super) fn md5_8(msg: &str) -> String {
    openssl::hash::hash(openssl::hash::MessageDigest::md5(), msg.as_bytes())
        .map(|d| d.iter().map(|b| format!("{:02x}", b)).collect::<String>()[..8].to_string())
        .unwrap_or_default()
}

pub(super) fn capitalized(t: &str) -> String {
    let mut c = t.chars();
    match c.next() {
        Some(first) => first.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// " · <account>" for the entry's account, when it is known.
fn account_tail(b: &Bracket) -> String {
    let acct = order(&b.order_id).map(|e| e.account).unwrap_or_default();
    if acct.is_empty() { String::new() } else { format!(" · {}", acct) }
}

pub(super) fn fail(b: &Bracket, msg: &str) {
    let attempts = b.attempts + 1;
    patch_bracket(&b.id, BracketPatch { error: Some(msg.into()), attempts: Some(attempts), ..BracketPatch::default() });
    log(&format!("bagholder bracket: {} for {}: {} (attempt {}; next in {} s)", b.id, b.symbol, msg, attempts, retry_wait(attempts)));
    if attempts == 1 {
        emit(
            "problems",
            &format!("bracket:{}:fail:{}", b.id, md5_8(msg)),
            &format!("Bracket · {}", b.symbol),
            &format!("{} · trying again in a minute{}", capitalized(msg), account_tail(b)),
        );
    }
}

pub(super) fn arm_step(b: &Bracket, entry: Option<&Order>) {
    let mut b = b.clone();
    if b.status == BracketStatus::Waiting {
        let entry = match entry {
            None => {
                patch_bracket(&b.id, BracketPatch { status: Some(BracketStatus::Cancelled), outcome: Some("entry not found".into()), ..BracketPatch::default() });
                return;
            }
            Some(e) => e,
        };
        let est = entry.status;
        if matches!(est, OrderStatus::Pending | OrderStatus::Sent | OrderStatus::Cancelling | OrderStatus::Dry) {
            return;
        }
        let mut filled = entry.filled_qty.unwrap_or(0.0);
        if est == OrderStatus::Filled && filled == 0.0 {
            filled = entry.quantity.unwrap_or(0.0);
        }
        if filled == 0.0 || filled <= 0.0 {
            patch_bracket(&b.id, BracketPatch { status: Some(BracketStatus::Cancelled), outcome: Some(format!("entry {}", est)), ..BracketPatch::default() });
            log(&format!("bagholder bracket: {} for {} off: entry {} without a fill", b.id, b.symbol, est));
            return;
        }
        let mut patch = BracketPatch { quantity: Some(Some(filled)), status: Some(BracketStatus::Armed), armed_at: Some(now_iso()), ..BracketPatch::default() };
        patch.apply(&mut b);
        if b.sl_kind == SlKind::Trail {
            let high = or_f(or_f(entry.avg_fill, entry.limit_price), b.sl_price);
            if let Some(high) = high.filter(|h| *h != 0.0) {
                let sl_price = round_half_even(high - trail_distance(&b, high).unwrap_or(0.0), 2);
                patch.high_water = Some(Some(high));
                patch.sl_price = Some(Some(sl_price));
                patch.apply(&mut b);
            }
        }
        patch_bracket(&b.id, patch);
        log(&format!("bagholder bracket: {} armed for {} x {}", b.id, qty_text(filled), b.symbol));
    }
    if b.status != BracketStatus::Armed || !b.sl_kind.is_set() || !b.sl_order_id.is_empty() {
        return;
    }
    if !nothing_resting(&b) {
        return;
    }
    let native = stop_allowed(&b.security_id);
    if !native {
        if b.sl_mode != SlMode::Watched {
            patch_bracket(&b.id, BracketPatch { sl_mode: Some(SlMode::Watched), sl_native: Some(false), ..BracketPatch::default() });
            log(&format!("bagholder bracket: {} for {}: Wealthsimple takes no stop order for it; the stop is watched here", b.id, b.symbol));
        }
        return;
    }
    if !may_retry(&b) {
        return;
    }
    let (oid, err) = place_exit(&b, OrderType::Stop, b.sl_price, Role::Stop);
    if !err.is_empty() {
        fail(&b, &format!("stop not placed: {}", err));
    } else if !oid.is_empty() {
        patch_bracket(&b.id, BracketPatch {
            sl_order_id: Some(oid), sl_native: Some(true), sl_mode: Some(SlMode::Native), error: Some(String::new()), attempts: Some(0), moved_at: Some(now_iso()),
            ..BracketPatch::default()
        });
        log(&format!("bagholder bracket: {} stop placed at {} for {}", b.id, rp(b.sl_price), b.symbol));
    }
}

pub(super) fn stop_allowed(security_id: &str) -> bool {
    #[cfg(test)]
    if let Some(v) = *bracket_seam::STOP_ALLOWED.lock().unwrap_or_else(|e| e.into_inner()) {
        return v;
    }
    if let Some(v) = stop_allowed_cache().lock().unwrap().get(security_id) {
        return *v;
    }
    let sess = match ticket_session() {
        Some(s) => s,
        None => return false,
    };
    let ok = match gql(&sess, "FetchSecurityMarketData", json!({"id": security_id})) {
        Ok(d) => parse_market_data(&d)["orderTypes"].as_array().map_or(false, |a| a.iter().any(|t| t == "STOP")),
        Err(e) => {
            log(&format!("bagholder bracket: order types for {} unknown: {}", security_id, e));
            return false;
        }
    };
    stop_allowed_cache().lock().unwrap().insert(security_id.to_string(), ok);
    ok
}

/// Every exit order this bracket has placed, whatever became of it.
pub(super) fn own_exit_rows(b: &Bracket) -> Vec<Order> {
    orders_all().into_iter().filter(|o| o.parent_id == b.order_id && matches!(o.role, Role::Stop | Role::Target)).collect()
}

/// End a bracket: cancel what it has resting, and say why it ended. It is `closing`
/// until Wealthsimple confirms nothing rests, then `done`.
pub(super) fn end_bracket(b: &Bracket, outcome: &str, note: &str) -> BracketStatus {
    let mut pending = false;
    for o in own_exit_rows(b) {
        if resting(&o) {
            let err = cancel_exit(&o.id);
            if !err.is_empty() {
                log(&format!("bagholder bracket: {} for {}: cancel of {} refused: {}; tried again on the next check", b.id, b.symbol, o.id, err));
            }
            pending = true;
        } else if o.status == OrderStatus::Cancelling {
            pending = true;
        }
    }
    let status = if pending { BracketStatus::Closing } else { BracketStatus::Done };
    patch_bracket(&b.id, BracketPatch {
        status: Some(status), outcome: Some(outcome.into()), error: Some(note.into()), sl_order_id: Some(String::new()), tp_order_id: Some(String::new()),
        ..BracketPatch::default()
    });
    log(&format!("bagholder bracket: {} for {}: {}{}", b.id, b.symbol, outcome, if pending { "; its resting exit is being cancelled" } else { "" }));
    if !BRACKET_ENDED_QUIETLY.contains(&outcome) {
        emit("problems", &format!("bracket:{}:off", b.id), &format!("Bracket off · {}", b.symbol), &format!("{}{}", capitalized(outcome), account_tail(b)));
    }
    status
}

pub(super) fn closing_step(b: &Bracket) {
    let open: Vec<Order> = own_exit_rows(b).into_iter().filter(in_flight).collect();
    for o in open.iter().filter(|o| resting(o)) {
        let err = cancel_exit(&o.id);
        if !err.is_empty() {
            log(&format!("bagholder bracket: {} for {}: cancel of {} refused again: {}", b.id, b.symbol, o.id, err));
        }
    }
    if open.is_empty() {
        patch_bracket(&b.id, BracketPatch { status: Some(BracketStatus::Done), ..BracketPatch::default() });
        log(&format!("bagholder bracket: {} for {}: nothing rests at Wealthsimple; done", b.id, b.symbol));
    }
}

/// An exit resting at Wealthsimple that no live bracket holds is cancelled: it would
/// sell shares nothing is watching.
pub(super) fn sweep_exits() {
    for o in orders_all() {
        if !matches!(o.role, Role::Stop | Role::Target) || !resting(&o) {
            continue;
        }
        let b = must(so::typed::bracket_for_order(&db(), &o.parent_id));
        let held_by = b.as_ref().map_or(false, |b| b.status.is_live() && (b.status == BracketStatus::Closing || o.id == b.sl_order_id || o.id == b.tp_order_id));
        if held_by {
            continue;
        }
        let err = cancel_exit(&o.id);
        say_once(
            format!("{}|orphan", o.id),
            &format!(
                "bagholder bracket: {} for {} rests at Wealthsimple with no bracket holding it; cancelled{}\n",
                o.id,
                o.symbol,
                if err.is_empty() { String::new() } else { format!(" (refused: {})", err) }
            ),
        );
    }
}

pub(super) fn nothing_resting(b: &Bracket) -> bool {
    !own_exit_rows(b).iter().any(in_flight)
}

/// Why the position this bracket guards is gone, when it was closed somewhere other
/// than through the bracket; empty while it stands. The sale must be in the activity
/// feed, or the position missing from two balance reads after one that showed it: a
/// single read that lacks it decides nothing.
pub(super) fn closed_elsewhere(b: &Bracket) -> String {
    if !in_play(b) || b.armed_at.is_empty() || !nothing_resting(b) {
        return String::new();
    }
    let conn = db();
    let sold = must(bagholder_store::feeds::sold_since(&conn, &b.account_id, &b.security_id, &b.armed_at, &b.symbol));
    if sold != 0.0 && sold >= b.quantity.unwrap_or(0.0) {
        return format!("sold: {} shares in the activity feed", qty_text(sold));
    }
    let read_at = must(bagholder_store::tables::get_meta(&conn, "balances_read_at", ""));
    if read_at.is_empty() || read_at <= b.armed_at {
        return String::new();
    }
    let held = must(bagholder_store::feeds::position_quantity(&conn, &b.account_id, &b.security_id));
    if held.map_or(false, |h| h > 0.0) {
        if !b.seen_held || !b.missed_at.is_empty() {
            patch_bracket(&b.id, BracketPatch { seen_held: Some(true), missed_at: Some(String::new()), ..BracketPatch::default() });
        }
        return String::new();
    }
    if !b.seen_held {
        return String::new();
    }
    if b.missed_at.is_empty() {
        patch_bracket(&b.id, BracketPatch { missed_at: Some(read_at.clone()), ..BracketPatch::default() });
        log(&format!("bagholder bracket: {} for {}: the balances read at {} does not list the position; a second read decides", b.id, b.symbol, read_at));
        return String::new();
    }
    if read_at > b.missed_at {
        return format!("position gone: two balance reads without it ({}, {})", b.missed_at, read_at);
    }
    String::new()
}

/// Seconds until Wealthsimple lets a resting order lapse: its own expiry, or ninety
/// days from when a good-till-cancelled one was sent.
pub(super) fn expires_in(row: &Order, now: f64) -> Option<f64> {
    let mut exp = parse_utc_text(&row.expires_at);
    if exp.is_none() && row.tif.to_uppercase() == "UNTIL_CANCEL" {
        let sent = if row.submitted_at.is_empty() { &row.created_at } else { &row.submitted_at };
        exp = parse_utc_text(sent).map(|t| t + GTC_DAYS * 86400);
    }
    exp.map(|e| e as f64 - now)
}

/// What the engine reads of a quote.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct Tape {
    pub last: Option<f64>,
    pub bid: Option<f64>,
    pub open: bool,
}

impl Tape {
    pub(super) fn of(q: &Value) -> Tape {
        Tape { last: on(q, "last"), bid: on(q, "bid"), open: f(q, "marketStatus").to_uppercase() == "OPEN" }
    }
}

/// A resting exit is placed again before it lapses: within a week of it while the
/// market is closed, within two days whatever the market is doing.
pub(super) fn roll_due(row: Option<&Order>, tape: Option<Tape>, now: f64) -> bool {
    let row = match row {
        Some(r) if resting(r) => r,
        _ => return false,
    };
    let left = match expires_in(row, now) {
        Some(l) if l <= BRACKET_ROLL_SEC => l,
        _ => return false,
    };
    if left <= BRACKET_ROLL_LAST_SEC {
        return true;
    }
    !tape.map_or(false, |t| t.open)
}

pub(super) fn roll_step(b: &Bracket, tape: Option<Tape>) {
    let now = now_unix().floor();
    if b.status == BracketStatus::Armed && b.sl_mode == SlMode::Native && !b.sl_order_id.is_empty() {
        if roll_due(order(&b.sl_order_id).as_ref(), tape, now) {
            let err = cancel_exit(&b.sl_order_id);
            if !err.is_empty() {
                fail(b, &format!("stop not rolled: {}", err));
                return;
            }
            patch_bracket(&b.id, BracketPatch { sl_order_id: Some(String::new()), moved_at: Some(now_iso()), error: Some(String::new()), ..BracketPatch::default() });
            log(&format!("bagholder bracket: {} for {}: stop at {} nears Wealthsimple's ninety days; cancelled, placed again at the same level", b.id, b.symbol, rp(b.sl_price)));
        }
    } else if b.status == BracketStatus::TargetPlaced {
        if !b.tp_order_id.is_empty() {
            if roll_due(order(&b.tp_order_id).as_ref(), tape, now) {
                let err = cancel_exit(&b.tp_order_id);
                if !err.is_empty() {
                    fail(b, &format!("target not rolled: {}", err));
                    return;
                }
                patch_bracket(&b.id, BracketPatch { tp_order_id: Some(String::new()), moved_at: Some(now_iso()), error: Some(String::new()), ..BracketPatch::default() });
                log(&format!("bagholder bracket: {} for {}: target at {} nears Wealthsimple's ninety days; cancelled, placed again", b.id, b.symbol, rp(b.tp_price)));
            }
        } else if exit_row(b, Role::Target).map_or(false, |r| matches!(r.status, OrderStatus::Cancelled | OrderStatus::Expired)) {
            fire_target(b);
        }
    }
}

/// Why a leg that Wealthsimple no longer holds ends the bracket.
fn leg_gone(leg: &str, row: &Order) -> String {
    if row.status == OrderStatus::Cancelled {
        format!("{} cancelled at Wealthsimple by hand", leg)
    } else {
        format!("{} {} at Wealthsimple{}", leg, row.status, if row.error.is_empty() { String::new() } else { format!(": {}", row.error) })
    }
}

/// Bring a bracket in line with what became of its orders. True when it ended.
pub(super) fn reconcile_step(b: &Bracket) -> bool {
    let mut b = b.clone();
    if b.status == BracketStatus::Closing {
        closing_step(&b);
        return true;
    }
    let (stop_row, tp_row) = (exit_row(&b, Role::Stop), exit_row(&b, Role::Target));
    if stop_row.as_ref().map_or(false, |r| r.status == OrderStatus::Filled) {
        end_bracket(&b, "stopped", "");
        return true;
    }
    if tp_row.as_ref().map_or(false, |r| r.status == OrderStatus::Filled) {
        end_bracket(&b, "target", "");
        return true;
    }
    let moved = |theirs: Option<f64>, ours: Option<f64>| some(theirs) && some(ours) && (theirs.unwrap_or(0.0) - ours.unwrap_or(0.0)).abs() > 0.005;
    if let Some(sr) = &stop_row {
        if !b.sl_order_id.is_empty() && resting(sr) && moved(sr.stop_price, b.sl_price) {
            patch_bracket(&b.id, BracketPatch { sl_price: Some(sr.stop_price), ..BracketPatch::default() });
            log(&format!("bagholder bracket: {} for {}: stop moved by hand to {}; the bracket follows", b.id, b.symbol, rp(sr.stop_price)));
            b.sl_price = sr.stop_price;
        }
    }
    if let Some(tp) = &tp_row {
        if !b.tp_order_id.is_empty() && resting(tp) && moved(tp.limit_price, b.tp_price) {
            patch_bracket(&b.id, BracketPatch { tp_price: Some(tp.limit_price), ..BracketPatch::default() });
            log(&format!("bagholder bracket: {} for {}: target moved by hand to {}; the bracket follows", b.id, b.symbol, rp(tp.limit_price)));
        }
    }
    let gone = |r: &&Order| matches!(r.status, OrderStatus::Cancelled | OrderStatus::Rejected | OrderStatus::Failed);
    let native_stop = b.status == BracketStatus::Armed && b.sl_mode == SlMode::Native && !b.sl_order_id.is_empty();
    if native_stop && stop_row.as_ref().map_or(false, |r| r.status == OrderStatus::Expired) {
        patch_bracket(&b.id, BracketPatch { sl_order_id: Some(String::new()), error: Some(String::new()), ..BracketPatch::default() });
        log(&format!("bagholder bracket: {} for {}: stop expired at Wealthsimple; placed again", b.id, b.symbol));
    } else if let Some(sr) = stop_row.as_ref().filter(|r| native_stop && gone(r)) {
        end_bracket(&b, &leg_gone("stop", sr), "");
        return true;
    }
    let tp_placed = b.status == BracketStatus::TargetPlaced && !b.tp_order_id.is_empty();
    if tp_placed && tp_row.as_ref().map_or(false, |r| r.status == OrderStatus::Expired) {
        patch_bracket(&b.id, BracketPatch { tp_order_id: Some(String::new()), error: Some(String::new()), ..BracketPatch::default() });
        log(&format!("bagholder bracket: {} for {}: target expired at Wealthsimple; placed again", b.id, b.symbol));
    } else if let Some(tp) = tp_row.as_ref().filter(|r| tp_placed && gone(r)) {
        end_bracket(&b, &leg_gone("target", tp), "");
        return true;
    }
    let why = closed_elsewhere(&b);
    if !why.is_empty() {
        end_bracket(&b, &why, "");
        return true;
    }
    false
}

/// Act on the price: trail the stop, fire a watched stop, place the target when it is
/// reached, and swap the two when the price turns while one of them rests. Only while
/// the market is open, and only on a quote that has a last price.
pub(super) fn watch_step(b: &Bracket, tape: Option<Tape>) {
    let (tape, last) = match tape {
        Some(t) if t.open => match t.last {
            Some(l) => (t, l),
            None => return,
        },
        _ => return,
    };
    let mut b = b.clone();
    let now_s = now_iso();
    let trigger = tape.bid.unwrap_or(last);
    let (has_stop, has_target) = (b.sl_kind.is_set(), some(b.tp_price));
    let (sl_price, tp_price) = (b.sl_price.unwrap_or(0.0), b.tp_price.unwrap_or(0.0));
    let at_target = has_target && trigger >= tp_price;
    let status = b.status;
    if b.sl_kind == SlKind::Trail && (status == BracketStatus::TargetPlaced || (status == BracketStatus::Armed && !at_target)) {
        let hw0 = or_f(b.high_water, Some(0.0)).unwrap_or(0.0);
        let high = if last > hw0 { last } else { hw0 };
        if Some(high) != b.high_water {
            patch_bracket(&b.id, BracketPatch { high_water: Some(Some(high)), ..BracketPatch::default() });
        }
        let new_stop = round_half_even(high - trail_distance(&b, high).unwrap_or(0.0), 2);
        if new_stop > sl_price + f64::max(0.01, sl_price * TRAIL_MIN_MOVE) {
            if !b.sl_order_id.is_empty() {
                let err = cancel_exit(&b.sl_order_id);
                if !err.is_empty() {
                    fail(&b, &format!("stop not moved: {}", err));
                    return;
                }
            }
            patch_bracket(&b.id, BracketPatch { sl_price: Some(Some(new_stop)), sl_order_id: Some(String::new()), moved_at: Some(now_s), ..BracketPatch::default() });
            log(&format!("bagholder bracket: {} for {}: stop moves to {} (high {})", b.id, b.symbol, rp(Some(new_stop)), rp(Some(high))));
            b.sl_price = Some(new_stop);
            b.sl_order_id.clear();
        }
    }
    // the stop's level may just have moved
    let sl_price = b.sl_price.unwrap_or(0.0);
    if status == BracketStatus::Armed && has_stop && b.sl_mode == SlMode::Watched && b.sl_order_id.is_empty() && some(b.sl_price) && trigger <= sl_price {
        if !may_retry(&b) {
            return;
        }
        let (oid, err) = place_exit(&b, OrderType::Market, b.sl_price, Role::Stop);
        if !err.is_empty() {
            fail(&b, &format!("stop not placed: {}", err));
        } else if !oid.is_empty() {
            patch_bracket(&b.id, BracketPatch { sl_order_id: Some(oid), status: Some(BracketStatus::Firing), error: Some(String::new()), ..BracketPatch::default() });
            log(&format!("bagholder bracket: {} for {}: stop hit at {}, market sell placed", b.id, b.symbol, rp(Some(trigger))));
        }
        return;
    }
    if status == BracketStatus::Armed && at_target {
        if !b.sl_order_id.is_empty() {
            let err = cancel_exit(&b.sl_order_id);
            if !err.is_empty() {
                fail(&b, &format!("stop not cancelled for the target: {}", err));
                return;
            }
            patch_bracket(&b.id, BracketPatch { status: Some(BracketStatus::Firing), error: Some(String::new()), ..BracketPatch::default() });
            log(&format!("bagholder bracket: {} for {}: target reached at {}, stop cancel sent", b.id, b.symbol, rp(Some(trigger))));
            return;
        }
        fire_target(&b);
    }
    if status == BracketStatus::Firing && has_target && b.tp_order_id.is_empty() && exit_row(&b, Role::Stop).map_or(false, |r| r.status == OrderStatus::Cancelled) {
        fire_target(&b);
    }
    if status == BracketStatus::TargetPlaced && has_stop && some(b.sl_price) && !b.tp_order_id.is_empty() {
        if trigger <= sl_price {
            let err = cancel_exit(&b.tp_order_id);
            if !err.is_empty() {
                fail(&b, &format!("target not cancelled for the stop: {}", err));
                return;
            }
            patch_bracket(&b.id, BracketPatch { status: Some(BracketStatus::Stopping), tp_order_id: Some(String::new()), error: Some(String::new()), attempts: Some(0), ..BracketPatch::default() });
            log(&format!(
                "bagholder bracket: {} for {}: stop level {} reached at {} while the limit sell rested; its cancel sent, market sell follows",
                b.id, b.symbol, rp(b.sl_price), rp(Some(trigger))
            ));
            return;
        }
        if has_target && trigger < tp_price * (1.0 - TARGET_BACK_OFF) {
            let err = cancel_exit(&b.tp_order_id);
            if !err.is_empty() {
                fail(&b, &format!("target not cancelled for the stop: {}", err));
                return;
            }
            patch_bracket(&b.id, BracketPatch {
                status: Some(BracketStatus::Armed), tp_order_id: Some(String::new()), sl_order_id: Some(String::new()), error: Some(String::new()), attempts: Some(0),
                ..BracketPatch::default()
            });
            log(&format!("bagholder bracket: {} for {}: target out of reach at {}; the limit sell's cancel sent, the stop order goes back", b.id, b.symbol, rp(Some(trigger))));
            return;
        }
    }
    if status == BracketStatus::Stopping && exit_row(&b, Role::Target).map_or(false, |r| matches!(r.status, OrderStatus::Cancelled | OrderStatus::Expired)) {
        if !may_retry(&b) || !nothing_resting(&b) {
            return;
        }
        let (oid, err) = place_exit(&b, OrderType::Market, b.sl_price, Role::Stop);
        if !err.is_empty() {
            fail(&b, &format!("stop not placed: {}", err));
        } else if !oid.is_empty() {
            patch_bracket(&b.id, BracketPatch { sl_order_id: Some(oid), status: Some(BracketStatus::Firing), error: Some(String::new()), attempts: Some(0), ..BracketPatch::default() });
            log(&format!("bagholder bracket: {} for {}: market sell placed at the stop", b.id, b.symbol));
        }
    }
}

pub(super) fn fire_target(b: &Bracket) {
    if !may_retry(b) || !nothing_resting(b) {
        return;
    }
    let (oid, err) = place_exit(b, OrderType::Limit, b.tp_price, Role::Target);
    if !err.is_empty() {
        fail(b, &format!("target not placed: {}", err));
    } else if !oid.is_empty() {
        patch_bracket(&b.id, BracketPatch { tp_order_id: Some(oid), status: Some(BracketStatus::TargetPlaced), error: Some(String::new()), attempts: Some(0), ..BracketPatch::default() });
        log(&format!("bagholder bracket: {} for {}: limit sell at {} placed", b.id, b.symbol, rp(b.tp_price)));
    }
}

pub(super) fn panic_text(e: &(dyn std::any::Any + Send)) -> String {
    e.downcast_ref::<String>().cloned().or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_else(|| "panic".into())
}

/// Quotes by security id, fetched here when None.
pub fn bracket_tick(quotes: Option<HashMap<String, Value>>) -> Value {
    if BRACKET_LOCK.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_err() {
        return json!({"ok": false, "skipped": "running"});
    }
    struct Release;
    impl Drop for Release {
        fn drop(&mut self) {
            BRACKET_LOCK.store(false, Ordering::SeqCst);
        }
    }
    let _release = Release;
    let sweep = || {
        if let Err(e) = catch_unwind(sweep_exits) {
            log(&format!("bagholder bracket: sweep failed: {}", panic_text(&*e)));
        }
    };
    let live = live_brackets();
    if live.is_empty() {
        sweep();
        return json!({"ok": true, "brackets": 0});
    }
    if orders_live() {
        // what the engine is about to decide on is read from Wealthsimple first
        for b in &live {
            let prev_in_flight = |role: Role| {
                if let Some(prev) = exit_row(b, role).filter(in_flight) {
                    refresh_orders(&prev.id);
                }
            };
            match b.status {
                BracketStatus::Waiting => {
                    refresh_orders(&b.order_id);
                }
                BracketStatus::Firing if !b.sl_order_id.is_empty() && b.tp_order_id.is_empty() => {
                    refresh_orders(&b.sl_order_id);
                }
                BracketStatus::Armed if b.sl_kind.is_set() && b.sl_order_id.is_empty() => prev_in_flight(Role::Stop),
                BracketStatus::TargetPlaced if b.tp_order_id.is_empty() => prev_in_flight(Role::Target),
                BracketStatus::Stopping => prev_in_flight(Role::Target),
                BracketStatus::Closing => {
                    for o in own_exit_rows(b).iter().filter(|o| o.status == OrderStatus::Cancelling) {
                        refresh_orders(&o.id);
                    }
                }
                _ => {}
            }
        }
    }
    let orders: HashMap<String, Order> = orders_all().into_iter().map(|o| (o.id.clone(), o)).collect();
    let quotes = match quotes {
        Some(q) => q,
        None => {
            let mut ids: Vec<String> = live.iter().filter(|b| in_play(b)).map(|b| b.security_id.clone()).collect();
            ids.sort();
            ids.dedup();
            let mut q = HashMap::new();
            if !ids.is_empty() {
                if let Some(sess) = ticket_session() {
                    match fetch_quotes(&sess, &ids) {
                        Ok(x) => q = x,
                        Err(e) => log(&format!("bagholder bracket: quotes failed: {}", err_text(&e))),
                    }
                }
            }
            q
        }
    };
    let tapes: HashMap<&String, Tape> = quotes.iter().map(|(id, q)| (id, Tape::of(q))).collect();
    for b in &live {
        // each step reads the bracket afresh: the one before it may have changed it
        let r = catch_unwind(AssertUnwindSafe(|| {
            let entry = orders.get(&b.order_id);
            if reconcile_step(b) {
                return;
            }
            let Some(b) = bracket(&b.id) else { return };
            roll_step(&b, tapes.get(&b.security_id).copied());
            let Some(b) = bracket(&b.id) else { return };
            arm_step(&b, entry);
            let Some(b) = bracket(&b.id) else { return };
            if in_play(&b) {
                watch_step(&b, tapes.get(&b.security_id).copied());
            }
        }));
        if let Err(e) = r {
            log(&format!("bagholder bracket: {} tick failed: {}", b.id, panic_text(&*e)));
        }
    }
    sweep();
    json!({"ok": true, "brackets": live.len()})
}

/// Whether the bracket engine has anything to do: a bracket in play, or an exit
/// order resting at Wealthsimple that a tick may have to cancel. With neither, a
/// tick reads two tables and returns; so the engine sleeps until a commit gives it
/// one, and then keeps its cadence -- a stop is watched every few seconds for as
/// long as it exists, exactly as before.
pub(super) fn bracket_work() -> bool {
    catch_unwind(|| !live_brackets().is_empty() || orders_all().iter().any(|o| matches!(o.role, Role::Stop | Role::Target) && resting(o)))
        .unwrap_or(true) // could not tell: tick, rather than miss a stop
}

pub fn bracket_loop() {
    while crate::events::park_until(bracket_work) {
        if app().wait(Duration::from_secs(BRACKET_POLL_SEC)) {
            return;
        }
        if !connected_not_syncing() {
            continue;
        }
        if let Err(e) = catch_unwind(|| bracket_tick(None)) {
            log(&format!("bagholder bracket: tick failed: {}", panic_text(&*e)));
        }
    }
}

pub fn cancel_bracket(bracket_id: &str) -> Value {
    let b = match bracket(bracket_id) {
        Some(b) => b,
        None => return json!({"ok": false, "error": "No such bracket."}),
    };
    if !b.status.is_live() {
        return json!({"ok": false, "error": "That bracket is not live."});
    }
    end_bracket(&b, "cancelled by the user", "");
    json!({"ok": true, "id": b.id})
}
