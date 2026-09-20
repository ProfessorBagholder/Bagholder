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
pub const BRACKET_LIVE: [&str; 6] = ["waiting", "armed", "firing", "target_placed", "stopping", "closing"];
pub(super) const BRACKET_RESTING: [&str; 2] = ["sent", "pending"];
pub(super) const BRACKET_INFLIGHT: [&str; 3] = ["sent", "pending", "cancelling"];
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

pub(super) fn st_in(o: &Value, set: &[&str]) -> bool {
    set.contains(&f(o, "status").as_str())
}

pub(super) fn release_shares(b: &Value, sold: f64) {
    let remaining = round_half_even(or0(b, "quantity") - sold, 6);
    for k in ["slOrderId", "tpOrderId"] {
        let oid = f(b, k);
        let err = cancel_exit(&oid);
        if !err.is_empty() {
            log(&format!("bagholder bracket: {} for {}: cancel of {} refused: {}", f(b, "id"), f(b, "symbol"), if oid.is_empty() { "None".into() } else { oid }, err));
        }
    }
    update_bracket(&f(b, "id"), json!({"quantity": remaining, "slOrderId": "", "tpOrderId": "", "status": "armed", "error": "", "attempts": 0}));
    log(&format!(
        "bagholder bracket: {} for {}: {} of its shares sold from the ticket; the stop is placed again on the {} left",
        f(b, "id"), f(b, "symbol"), qty_text(sold), qty_text(remaining)
    ));
}

pub(super) fn await_cancels(b: &Value, seconds: u32) {
    if !orders_live() {
        return;
    }
    for _ in 0..seconds {
        let open: Vec<Value> = own_exit_rows(b).into_iter().filter(|o| st_in(o, &BRACKET_INFLIGHT)).collect();
        if open.is_empty() {
            return;
        }
        for o in &open {
            refresh_orders(&f(o, "id"));
        }
        #[cfg(not(test))]
        std::thread::sleep(Duration::from_secs(1));
    }
}

pub fn create_bracket(order_row: &Value) -> Value {
    let empty = json!({});
    let sl = order_row.get("stopLoss").filter(|v| truthy(Some(v))).unwrap_or(&empty);
    let tp = order_row.get("takeProfit").filter(|v| truthy(Some(v))).unwrap_or(&empty);
    let id = format!("bracket-{}", uuid4());
    let unit = if tr(sl, "trailUnit") { f(sl, "trailUnit") } else { "pct".into() };
    let b = json!({
        "id": id, "orderId": f(order_row, "id"), "accountId": f(order_row, "accountId"), "securityId": f(order_row, "securityId"),
        "symbol": f(order_row, "symbol"), "currency": f(order_row, "currency"), "quantity": gv(order_row, "quantity"), "tif": BRACKET_TIF,
        "slKind": f(sl, "kind"), "slPrice": gv(sl, "price"), "slTrail": gv(sl, "trail"), "slTrailUnit": unit,
        "tpPrice": gv(tp, "price"), "status": "waiting",
    });
    must(so::insert_bracket(&db(), &b, &now_iso()));
    get_bracket(&id).unwrap_or(b)
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

pub(super) fn trail_distance(b: &Value, price: f64) -> Option<f64> {
    if f(b, "slKind") != "trail" || !tr(b, "slTrail") {
        return None;
    }
    let t = or0(b, "slTrail");
    Some(if f(b, "slTrailUnit") == "pct" { price * t / 100.0 } else { t })
}

pub(super) fn exit_body(b: &Value, exec_type: &str, price: Option<f64>, role: &str) -> Result<(Value, Value), String> {
    let mut body = json!({"symbol": gv(b, "symbol"), "securityId": gv(b, "securityId"), "accountId": gv(b, "accountId"), "side": "SELL", "type": exec_type, "tif": BRACKET_TIF,
        "quantity": gv(b, "quantity"), "currency": gv(b, "currency")});
    if exec_type == "LIMIT" {
        set(&mut body, "limitPrice", jo(price));
    }
    if exec_type == "STOP" {
        set(&mut body, "stopPrice", jo(price));
    }
    let (mut row, req) = order_request(&body)?;
    set(&mut row, "role", json!(role));
    set(&mut row, "parentId", gv(b, "orderId"));
    Ok((row, req))
}

/// (order id, error).
pub(super) fn place_exit(b: &Value, exec_type: &str, price: Option<f64>, role: &str) -> (String, String) {
    let (mut row, req) = match exit_body(b, exec_type, price, role) {
        Ok(x) => x,
        Err(e) => return (String::new(), e),
    };
    if !orders_live() {
        let key = format!("{}|{}|{}", f(b, "id"), role, rp(price.map(|p| round_half_even(p, 4))));
        say_once(key, &format!("bagholder bracket (orders are off, not placed): {} {} for {}: {}", role, exec_type, f(b, "symbol"), bagholder_store::tables::json_text_sorted(&req)));
        return (String::new(), String::new());
    }
    let r = submit_order(&mut row, &req);
    if !tr(&r, "ok") {
        let e = f(&r, "error");
        return (String::new(), if e.is_empty() { "not sent".into() } else { e });
    }
    (f(&r, "id"), String::new())
}

pub(super) fn cancel_exit(order_id: &str) -> String {
    if order_id.is_empty() {
        return String::new();
    }
    match get_order(order_id) {
        Some(row) if st_in(&row, &BRACKET_RESTING) => {}
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

pub(super) fn exit_row(b: &Value, role: &str) -> Option<Value> {
    let held = f(b, if role == "stop" { "slOrderId" } else { "tpOrderId" });
    if !held.is_empty() {
        if let Some(row) = get_order(&held) {
            return Some(row);
        }
    }
    let parent = f(b, "orderId");
    list_orders().into_iter().find(|o| f(o, "parentId") == parent && f(o, "role") == role)
}

pub(super) fn attempts_of(b: &Value) -> i64 {
    or0(b, "attempts") as i64
}

pub(super) fn retry_wait(attempts: i64) -> i64 {
    BRACKET_RETRY_SEC[(attempts.min(BRACKET_RETRY_SEC.len() as i64) - 1).max(0) as usize]
}

pub(super) fn may_retry(b: &Value) -> bool {
    let attempts = attempts_of(b);
    if attempts == 0 {
        return true;
    }
    let wait = retry_wait(attempts);
    match parse_z(&f(b, "updatedAt")) {
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

pub(super) fn fail(b: &Value, msg: &str) {
    let attempts = attempts_of(b) + 1;
    update_bracket(&f(b, "id"), json!({"error": msg, "attempts": attempts}));
    log(&format!("bagholder bracket: {} for {}: {} (attempt {}; next in {} s)", f(b, "id"), f(b, "symbol"), msg, attempts, retry_wait(attempts)));
    if attempts == 1 {
        let entry = get_order(&f(b, "orderId")).unwrap_or(json!({}));
        let acct = f(&entry, "account");
        emit(
            "problems",
            &format!("bracket:{}:fail:{}", f(b, "id"), md5_8(msg)),
            &format!("Bracket · {}", f(b, "symbol")),
            &format!("{} · trying again in a minute{}", capitalized(msg), if acct.is_empty() { String::new() } else { format!(" · {}", acct) }),
        );
    }
}

pub(super) fn arm_step(b: &Value, entry: Option<&Value>) {
    let mut b = b.clone();
    let bid = f(&b, "id");
    if f(&b, "status") == "waiting" {
        let entry = match entry {
            None => {
                update_bracket(&bid, json!({"status": "cancelled", "outcome": "entry not found"}));
                return;
            }
            Some(e) => e,
        };
        let est = f(entry, "status");
        if ["pending", "sent", "cancelling", "dry"].contains(&est.as_str()) {
            return;
        }
        let mut filled = or0(entry, "filledQty");
        if est == "filled" && filled == 0.0 {
            filled = or0(entry, "quantity");
        }
        if filled == 0.0 || filled <= 0.0 {
            update_bracket(&bid, json!({"status": "cancelled", "outcome": format!("entry {}", est)}));
            log(&format!("bagholder bracket: {} for {} off: entry {} without a fill", bid, f(&b, "symbol"), est));
            return;
        }
        let armed_at = now_iso();
        set(&mut b, "quantity", json!(filled));
        set(&mut b, "status", json!("armed"));
        set(&mut b, "armedAt", json!(armed_at));
        let mut patch = json!({"quantity": filled, "status": "armed", "armedAt": armed_at});
        if f(&b, "slKind") == "trail" {
            let high = or_f(or_f(on(entry, "avgFill"), on(entry, "limitPrice")), on(&b, "slPrice"));
            if let Some(high) = high.filter(|h| *h != 0.0) {
                let sl_price = round_half_even(high - trail_distance(&b, high).unwrap_or(0.0), 2);
                set(&mut patch, "highWater", json!(high));
                set(&mut patch, "slPrice", json!(sl_price));
                set(&mut b, "highWater", json!(high));
                set(&mut b, "slPrice", json!(sl_price));
            }
        }
        update_bracket(&bid, patch);
        log(&format!("bagholder bracket: {} armed for {} x {}", bid, qty_text(filled), f(&b, "symbol")));
    }
    if f(&b, "status") != "armed" || !tr(&b, "slKind") || tr(&b, "slOrderId") {
        return;
    }
    if !nothing_resting(&b) {
        return;
    }
    let native = stop_allowed(&f(&b, "securityId"));
    if !native {
        if f(&b, "slMode") != "watched" {
            update_bracket(&bid, json!({"slMode": "watched", "slNative": false}));
            log(&format!("bagholder bracket: {} for {}: Wealthsimple takes no stop order for it; the stop is watched here", bid, f(&b, "symbol")));
        }
        return;
    }
    if !may_retry(&b) {
        return;
    }
    let (oid, err) = place_exit(&b, "STOP", on(&b, "slPrice"), "stop");
    if !err.is_empty() {
        fail(&b, &format!("stop not placed: {}", err));
    } else if !oid.is_empty() {
        update_bracket(&bid, json!({"slOrderId": oid, "slNative": true, "slMode": "native", "error": "", "attempts": 0, "movedAt": now_iso()}));
        log(&format!("bagholder bracket: {} stop placed at {} for {}", bid, rp(on(&b, "slPrice")), f(&b, "symbol")));
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

pub(super) fn own_exit_rows(b: &Value) -> Vec<Value> {
    let parent = f(b, "orderId");
    list_orders().into_iter().filter(|o| f(o, "parentId") == parent && (f(o, "role") == "stop" || f(o, "role") == "target")).collect()
}

pub(super) fn end_bracket(b: &Value, outcome: &str, note: &str) -> String {
    let mut pending = false;
    for o in own_exit_rows(b) {
        if st_in(&o, &BRACKET_RESTING) {
            let err = cancel_exit(&f(&o, "id"));
            if !err.is_empty() {
                log(&format!("bagholder bracket: {} for {}: cancel of {} refused: {}; tried again on the next check", f(b, "id"), f(b, "symbol"), f(&o, "id"), err));
            }
            pending = true;
        } else if f(&o, "status") == "cancelling" {
            pending = true;
        }
    }
    let status = if pending { "closing" } else { "done" };
    update_bracket(&f(b, "id"), json!({"status": status, "outcome": outcome, "error": note, "slOrderId": "", "tpOrderId": ""}));
    log(&format!("bagholder bracket: {} for {}: {}{}", f(b, "id"), f(b, "symbol"), outcome, if pending { "; its resting exit is being cancelled" } else { "" }));
    if !BRACKET_ENDED_QUIETLY.contains(&outcome) {
        let entry = get_order(&f(b, "orderId")).unwrap_or(json!({}));
        let acct = f(&entry, "account");
        emit(
            "problems",
            &format!("bracket:{}:off", f(b, "id")),
            &format!("Bracket off · {}", f(b, "symbol")),
            &format!("{}{}", capitalized(outcome), if acct.is_empty() { String::new() } else { format!(" · {}", acct) }),
        );
    }
    status.to_string()
}

pub(super) fn closing_step(b: &Value) {
    let open: Vec<Value> = own_exit_rows(b).into_iter().filter(|o| st_in(o, &BRACKET_INFLIGHT)).collect();
    for o in &open {
        if st_in(o, &BRACKET_RESTING) {
            let err = cancel_exit(&f(o, "id"));
            if !err.is_empty() {
                log(&format!("bagholder bracket: {} for {}: cancel of {} refused again: {}", f(b, "id"), f(b, "symbol"), f(o, "id"), err));
            }
        }
    }
    if open.is_empty() {
        update_bracket(&f(b, "id"), json!({"status": "done"}));
        log(&format!("bagholder bracket: {} for {}: nothing rests at Wealthsimple; done", f(b, "id"), f(b, "symbol")));
    }
}

pub(super) fn sweep_exits() {
    for o in list_orders() {
        let role = f(&o, "role");
        if (role != "stop" && role != "target") || !st_in(&o, &BRACKET_RESTING) {
            continue;
        }
        let b = must(so::bracket_for_order(&db(), &f(&o, "parentId")));
        let held_by = b.as_ref().map_or(false, |b| {
            st_in(b, &BRACKET_LIVE) && (f(b, "status") == "closing" || f(&o, "id") == f(b, "slOrderId") || f(&o, "id") == f(b, "tpOrderId"))
        });
        if held_by {
            continue;
        }
        let err = cancel_exit(&f(&o, "id"));
        say_once(
            format!("{}|orphan", f(&o, "id")),
            &format!(
                "bagholder bracket: {} for {} rests at Wealthsimple with no bracket holding it; cancelled{}\n",
                f(&o, "id"),
                if o.get("symbol").map_or(true, |v| v.is_null()) { "None".into() } else { f(&o, "symbol") },
                if err.is_empty() { String::new() } else { format!(" (refused: {})", err) }
            ),
        );
    }
}

pub(super) fn nothing_resting(b: &Value) -> bool {
    !own_exit_rows(b).iter().any(|o| st_in(o, &BRACKET_INFLIGHT))
}

pub(super) fn closed_elsewhere(b: &Value) -> String {
    if !["armed", "firing", "target_placed", "stopping"].contains(&f(b, "status").as_str()) || !tr(b, "armedAt") || !nothing_resting(b) {
        return String::new();
    }
    let conn = db();
    let armed_at = f(b, "armedAt");
    let sold = must(bagholder_store::feeds::sold_since(&conn, &f(b, "accountId"), &f(b, "securityId"), &armed_at, &f(b, "symbol")));
    if sold != 0.0 && sold >= or0(b, "quantity") {
        return format!("sold: {} shares in the activity feed", qty_text(sold));
    }
    let read_at = must(bagholder_store::tables::get_meta(&conn, "balances_read_at", ""));
    if read_at.is_empty() || read_at <= armed_at {
        return String::new();
    }
    let held = must(bagholder_store::feeds::position_quantity(&conn, &f(b, "accountId"), &f(b, "securityId")));
    if let Some(h) = held {
        if h > 0.0 {
            if !tr(b, "seenHeld") || tr(b, "missedAt") {
                update_bracket(&f(b, "id"), json!({"seenHeld": true, "missedAt": ""}));
            }
            return String::new();
        }
    }
    if !tr(b, "seenHeld") {
        return String::new();
    }
    let missed = f(b, "missedAt");
    if missed.is_empty() {
        update_bracket(&f(b, "id"), json!({"missedAt": read_at}));
        log(&format!("bagholder bracket: {} for {}: the balances read at {} does not list the position; a second read decides", f(b, "id"), f(b, "symbol"), read_at));
        return String::new();
    }
    if read_at > missed {
        return format!("position gone: two balance reads without it ({}, {})", missed, read_at);
    }
    String::new()
}

pub(super) fn expires_in(row: &Value, now: f64) -> Option<f64> {
    let mut exp = parse_utc(row.get("expiresAt"));
    if exp.is_none() && f(row, "tif").to_uppercase() == "UNTIL_CANCEL" {
        let sub = parse_utc(or_v(row.get("submittedAt"), row.get("createdAt")));
        exp = sub.map(|t| t + GTC_DAYS * 86400);
    }
    exp.map(|e| e as f64 - now)
}

pub(super) fn roll_due(row: Option<&Value>, quote: Option<&Value>, now: f64) -> bool {
    let row = match row {
        Some(r) if st_in(r, &["sent", "pending"]) => r,
        _ => return false,
    };
    let left = match expires_in(row, now) {
        Some(l) if l <= BRACKET_ROLL_SEC => l,
        _ => return false,
    };
    if left <= BRACKET_ROLL_LAST_SEC {
        return true;
    }
    quote.map(|q| f(q, "marketStatus").to_uppercase()).unwrap_or_default() != "OPEN"
}

pub(super) fn roll_step(b: &Value, quote: Option<&Value>) {
    let now = now_unix().floor();
    let now_s = now_iso();
    let bid = f(b, "id");
    let status = f(b, "status");
    if status == "armed" && f(b, "slMode") == "native" && tr(b, "slOrderId") {
        let row = get_order(&f(b, "slOrderId"));
        if roll_due(row.as_ref(), quote, now) {
            let err = cancel_exit(&f(b, "slOrderId"));
            if !err.is_empty() {
                fail(b, &format!("stop not rolled: {}", err));
                return;
            }
            update_bracket(&bid, json!({"slOrderId": "", "movedAt": now_s, "error": ""}));
            log(&format!("bagholder bracket: {} for {}: stop at {} nears Wealthsimple's ninety days; cancelled, placed again at the same level", bid, f(b, "symbol"), rp(on(b, "slPrice"))));
        }
    } else if status == "target_placed" {
        if tr(b, "tpOrderId") {
            let row = get_order(&f(b, "tpOrderId"));
            if roll_due(row.as_ref(), quote, now) {
                let err = cancel_exit(&f(b, "tpOrderId"));
                if !err.is_empty() {
                    fail(b, &format!("target not rolled: {}", err));
                    return;
                }
                update_bracket(&bid, json!({"tpOrderId": "", "movedAt": now_s, "error": ""}));
                log(&format!("bagholder bracket: {} for {}: target at {} nears Wealthsimple's ninety days; cancelled, placed again", bid, f(b, "symbol"), rp(on(b, "tpPrice"))));
            }
        } else if let Some(tp_row) = exit_row(b, "target") {
            if st_in(&tp_row, &["cancelled", "expired"]) {
                fire_target(b);
            }
        }
    }
}

pub(super) fn reconcile_step(b: &Value, _entry: Option<&Value>) -> &'static str {
    let mut b = b.clone();
    let bid = f(&b, "id");
    if f(&b, "status") == "closing" {
        closing_step(&b);
        return "done";
    }
    let (stop_row, tp_row) = (exit_row(&b, "stop"), exit_row(&b, "target"));
    if stop_row.as_ref().map_or(false, |r| f(r, "status") == "filled") {
        end_bracket(&b, "stopped", "");
        return "done";
    }
    if tp_row.as_ref().map_or(false, |r| f(r, "status") == "filled") {
        end_bracket(&b, "target", "");
        return "done";
    }
    if let Some(sr) = &stop_row {
        if tr(&b, "slOrderId") && st_in(sr, &BRACKET_RESTING) && tr(sr, "stopPrice") && tr(&b, "slPrice") && (or0(sr, "stopPrice") - or0(&b, "slPrice")).abs() > 0.005 {
            update_bracket(&bid, json!({"slPrice": gv(sr, "stopPrice")}));
            log(&format!("bagholder bracket: {} for {}: stop moved by hand to {}; the bracket follows", bid, f(&b, "symbol"), rp(on(sr, "stopPrice"))));
            set(&mut b, "slPrice", gv(sr, "stopPrice"));
        }
    }
    if let Some(tp) = &tp_row {
        if tr(&b, "tpOrderId") && st_in(tp, &BRACKET_RESTING) && tr(tp, "limitPrice") && tr(&b, "tpPrice") && (or0(tp, "limitPrice") - or0(&b, "tpPrice")).abs() > 0.005 {
            update_bracket(&bid, json!({"tpPrice": gv(tp, "limitPrice")}));
            log(&format!("bagholder bracket: {} for {}: target moved by hand to {}; the bracket follows", bid, f(&b, "symbol"), rp(on(tp, "limitPrice"))));
        }
    }
    let status = f(&b, "status");
    let native_stop = status == "armed" && f(&b, "slMode") == "native" && tr(&b, "slOrderId");
    if native_stop && stop_row.as_ref().map_or(false, |r| f(r, "status") == "expired") {
        update_bracket(&bid, json!({"slOrderId": "", "error": ""}));
        log(&format!("bagholder bracket: {} for {}: stop expired at Wealthsimple; placed again", bid, f(&b, "symbol")));
    } else if let Some(sr) = stop_row.as_ref().filter(|r| native_stop && st_in(r, &["cancelled", "rejected", "failed"])) {
        let why = if f(sr, "status") == "cancelled" {
            "stop cancelled at Wealthsimple by hand".to_string()
        } else {
            format!("stop {} at Wealthsimple{}", f(sr, "status"), if tr(sr, "error") { format!(": {}", f(sr, "error")) } else { String::new() })
        };
        end_bracket(&b, &why, "");
        return "done";
    }
    let tp_placed = status == "target_placed" && tr(&b, "tpOrderId");
    if tp_placed && tp_row.as_ref().map_or(false, |r| f(r, "status") == "expired") {
        update_bracket(&bid, json!({"tpOrderId": "", "error": ""}));
        log(&format!("bagholder bracket: {} for {}: target expired at Wealthsimple; placed again", bid, f(&b, "symbol")));
    } else if let Some(tp) = tp_row.as_ref().filter(|r| tp_placed && st_in(r, &["cancelled", "rejected", "failed"])) {
        let why = if f(tp, "status") == "cancelled" {
            "target cancelled at Wealthsimple by hand".to_string()
        } else {
            format!("target {} at Wealthsimple{}", f(tp, "status"), if tr(tp, "error") { format!(": {}", f(tp, "error")) } else { String::new() })
        };
        end_bracket(&b, &why, "");
        return "done";
    }
    let why = closed_elsewhere(&b);
    if !why.is_empty() {
        end_bracket(&b, &why, "");
        return "done";
    }
    ""
}

pub(super) fn watch_step(b: &Value, quote: Option<&Value>) {
    let quote = match quote {
        Some(q) if f(q, "marketStatus").to_uppercase() == "OPEN" => q,
        _ => return,
    };
    let (last, bid_px) = (on(quote, "last"), on(quote, "bid"));
    let last = match last {
        Some(l) => l,
        None => return,
    };
    let mut b = b.clone();
    let id = f(&b, "id");
    let sym = f(&b, "symbol");
    let now_s = now_iso();
    let trigger = bid_px.unwrap_or(last);
    let at_target = tr(&b, "tpPrice") && trigger >= or0(&b, "tpPrice");
    let status = f(&b, "status");
    if f(&b, "slKind") == "trail" && (status == "target_placed" || (status == "armed" && !at_target)) {
        let hw = on(&b, "highWater");
        let hw0 = or_f(hw, Some(0.0)).unwrap_or(0.0);
        let high = if last > hw0 { last } else { hw0 };
        if Some(high) != hw {
            update_bracket(&id, json!({"highWater": high}));
        }
        let new_stop = round_half_even(high - trail_distance(&b, high).unwrap_or(0.0), 2);
        let cur = or0(&b, "slPrice");
        if new_stop > cur + f64::max(0.01, cur * TRAIL_MIN_MOVE) {
            if tr(&b, "slOrderId") {
                let err = cancel_exit(&f(&b, "slOrderId"));
                if !err.is_empty() {
                    fail(&b, &format!("stop not moved: {}", err));
                    return;
                }
            }
            update_bracket(&id, json!({"slPrice": new_stop, "slOrderId": "", "movedAt": now_s}));
            log(&format!("bagholder bracket: {} for {}: stop moves to {} (high {})", id, sym, rp(Some(new_stop)), rp(Some(high))));
            set(&mut b, "slPrice", json!(new_stop));
            set(&mut b, "slOrderId", json!(""));
        }
    }
    if status == "armed" && tr(&b, "slKind") && f(&b, "slMode") == "watched" && !tr(&b, "slOrderId") && tr(&b, "slPrice") {
        if trigger <= or0(&b, "slPrice") {
            if !may_retry(&b) {
                return;
            }
            let (oid, err) = place_exit(&b, "MARKET", on(&b, "slPrice"), "stop");
            if !err.is_empty() {
                fail(&b, &format!("stop not placed: {}", err));
            } else if !oid.is_empty() {
                update_bracket(&id, json!({"slOrderId": oid, "status": "firing", "error": ""}));
                log(&format!("bagholder bracket: {} for {}: stop hit at {}, market sell placed", id, sym, rp(Some(trigger))));
            }
            return;
        }
    }
    if status == "armed" && tr(&b, "tpPrice") && trigger >= or0(&b, "tpPrice") {
        if tr(&b, "slOrderId") {
            let err = cancel_exit(&f(&b, "slOrderId"));
            if !err.is_empty() {
                fail(&b, &format!("stop not cancelled for the target: {}", err));
                return;
            }
            update_bracket(&id, json!({"status": "firing", "error": ""}));
            log(&format!("bagholder bracket: {} for {}: target reached at {}, stop cancel sent", id, sym, rp(Some(trigger))));
            return;
        }
        fire_target(&b);
    }
    if status == "firing" && tr(&b, "tpPrice") && !tr(&b, "tpOrderId") {
        if exit_row(&b, "stop").map_or(false, |r| f(&r, "status") == "cancelled") {
            fire_target(&b);
        }
    }
    if status == "target_placed" && tr(&b, "slKind") && tr(&b, "slPrice") && tr(&b, "tpOrderId") {
        if trigger <= or0(&b, "slPrice") {
            let err = cancel_exit(&f(&b, "tpOrderId"));
            if !err.is_empty() {
                fail(&b, &format!("target not cancelled for the stop: {}", err));
                return;
            }
            update_bracket(&id, json!({"status": "stopping", "tpOrderId": "", "error": "", "attempts": 0}));
            log(&format!(
                "bagholder bracket: {} for {}: stop level {} reached at {} while the limit sell rested; its cancel sent, market sell follows",
                id, sym, rp(on(&b, "slPrice")), rp(Some(trigger))
            ));
            return;
        }
        if tr(&b, "tpPrice") && trigger < or0(&b, "tpPrice") * (1.0 - TARGET_BACK_OFF) {
            let err = cancel_exit(&f(&b, "tpOrderId"));
            if !err.is_empty() {
                fail(&b, &format!("target not cancelled for the stop: {}", err));
                return;
            }
            update_bracket(&id, json!({"status": "armed", "tpOrderId": "", "slOrderId": "", "error": "", "attempts": 0}));
            log(&format!("bagholder bracket: {} for {}: target out of reach at {}; the limit sell's cancel sent, the stop order goes back", id, sym, rp(Some(trigger))));
            return;
        }
    }
    if status == "stopping" {
        if exit_row(&b, "target").map_or(false, |r| st_in(&r, &["cancelled", "expired"])) {
            if !may_retry(&b) || !nothing_resting(&b) {
                return;
            }
            let (oid, err) = place_exit(&b, "MARKET", on(&b, "slPrice"), "stop");
            if !err.is_empty() {
                fail(&b, &format!("stop not placed: {}", err));
            } else if !oid.is_empty() {
                update_bracket(&id, json!({"slOrderId": oid, "status": "firing", "error": "", "attempts": 0}));
                log(&format!("bagholder bracket: {} for {}: market sell placed at the stop", id, sym));
            }
        }
    }
}

pub(super) fn fire_target(b: &Value) {
    if !may_retry(b) || !nothing_resting(b) {
        return;
    }
    let (oid, err) = place_exit(b, "LIMIT", on(b, "tpPrice"), "target");
    if !err.is_empty() {
        fail(b, &format!("target not placed: {}", err));
    } else if !oid.is_empty() {
        update_bracket(&f(b, "id"), json!({"tpOrderId": oid, "status": "target_placed", "error": "", "attempts": 0}));
        log(&format!("bagholder bracket: {} for {}: limit sell at {} placed", f(b, "id"), f(b, "symbol"), rp(on(b, "tpPrice"))));
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
    let live = brackets(&BRACKET_LIVE);
    if live.is_empty() {
        sweep();
        return json!({"ok": true, "brackets": 0});
    }
    if orders_live() {
        for b in &live {
            let st = f(b, "status");
            let prev_in_flight = |role: &str| {
                if let Some(prev) = exit_row(b, role) {
                    if st_in(&prev, &["sent", "pending", "cancelling"]) {
                        refresh_orders(&f(&prev, "id"));
                    }
                }
            };
            if st == "waiting" {
                refresh_orders(&f(b, "orderId"));
            } else if st == "firing" && tr(b, "slOrderId") && !tr(b, "tpOrderId") {
                refresh_orders(&f(b, "slOrderId"));
            } else if st == "armed" && tr(b, "slKind") && !tr(b, "slOrderId") {
                prev_in_flight("stop");
            } else if st == "target_placed" && !tr(b, "tpOrderId") {
                prev_in_flight("target");
            } else if st == "stopping" {
                prev_in_flight("target");
            } else if st == "closing" {
                for o in own_exit_rows(b) {
                    if f(&o, "status") == "cancelling" {
                        refresh_orders(&f(&o, "id"));
                    }
                }
            }
        }
    }
    let orders: HashMap<String, Value> = list_orders().into_iter().map(|o| (f(&o, "id"), o)).collect();
    let quotes = match quotes {
        Some(q) => q,
        None => {
            let mut ids: Vec<String> = live.iter().filter(|b| st_in(b, &["armed", "firing", "target_placed", "stopping"])).map(|b| f(b, "securityId")).collect();
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
    for b in &live {
        let r = catch_unwind(AssertUnwindSafe(|| {
            let bid = f(b, "id");
            let entry = orders.get(&f(b, "orderId"));
            if reconcile_step(b, entry) == "done" {
                return;
            }
            let Some(b) = get_bracket(&bid) else { return };
            roll_step(&b, quotes.get(&f(&b, "securityId")));
            let Some(b) = get_bracket(&bid) else { return };
            arm_step(&b, entry);
            let Some(b) = get_bracket(&bid) else { return };
            if st_in(&b, &["armed", "firing", "target_placed", "stopping"]) {
                watch_step(&b, quotes.get(&f(&b, "securityId")));
            }
        }));
        if let Err(e) = r {
            log(&format!("bagholder bracket: {} tick failed: {}", f(b, "id"), panic_text(&*e)));
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
    catch_unwind(|| {
        !brackets(&BRACKET_LIVE).is_empty()
            || list_orders().iter().any(|o| { let r = f(o, "role"); (r == "stop" || r == "target") && st_in(o, &BRACKET_RESTING) })
    })
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
    let b = match get_bracket(bracket_id) {
        Some(b) => b,
        None => return json!({"ok": false, "error": "No such bracket."}),
    };
    if !st_in(&b, &BRACKET_LIVE) {
        return json!({"ok": false, "error": "That bracket is not live."});
    }
    end_bracket(&b, "cancelled by the user", "");
    json!({"ok": true, "id": gv(&b, "id")})
}
