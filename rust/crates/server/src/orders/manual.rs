//! A fill the person enters by hand.

use super::*;

// ---------------------------------------------------------------------------
// manual activity
// ---------------------------------------------------------------------------

pub(super) fn manual_from_fields(_app: &Arc<App>, body: &Value) -> Value {
    let mut side = upper(or_v(body.get("side"), Some(&json!("BUY"))));
    if side != "BUY" && side != "SELL" {
        side = "BUY".into();
    }
    let pick = |a: &str, b: &str| if body.get(a).map_or(false, |v| !v.is_null()) { body.get(a) } else { body.get(b) };
    let qty = num(pick("qty", "quantity"), Some(0.0)).unwrap_or(0.0).abs();
    let px = num(pick("price", "unitPrice"), Some(0.0)).unwrap_or(0.0).abs();
    let mut date = date_only(or_v(or_v(body.get("date"), body.get("transactionDate")), body.get("occurredAt")));
    if date.is_empty() {
        date = crate::app::today_utc();
    }
    let symbol = upper(body.get("symbol"));
    let mut currency = upper(or_v(body.get("currency"), Some(&json!("CAD"))));
    if currency != "CAD" && currency != "USD" {
        currency = "CAD".into();
    }
    let mut account_id = s(or_v(or_v(body.get("accountId"), body.get("account")), Some(&json!("manual"))));
    if account_id.is_empty() {
        account_id = "manual".into();
    }
    let buy = side == "BUY";
    let signed_qty = if buy { qty } else { -qty };
    let cash = if buy { -(qty * px) } else { qty * px };
    let account_type = s(or_v(body.get("accountType"), Some(&json!(if account_id == "manual" { "Manual" } else { "" }))));
    let desc = format!(
        "{}{}",
        if buy { "Buy" } else { "Sell" },
        if symbol.is_empty() { String::new() } else { format!(" {} {} @ {}", fmt_g(qty), symbol, fmt_g(px)) }
    );
    json!({
        "id": uuid4(),
        "occurredAt": date,
        "transactionDate": date,
        "settlementDate": date,
        "accountId": account_id,
        "bookId": account_id,
        "accountType": account_type,
        "activityType": "Trade",
        "activitySubType": side,
        "description": desc,
        "direction": if buy { "DEBIT" } else { "CREDIT" },
        "symbol": symbol,
        "name": symbol,
        "currency": currency,
        "quantity": signed_qty,
        "unitPrice": px,
        "commission": num(body.get("commission"), Some(0.0)).unwrap_or(0.0).abs(),
        "netCashAmount": cash,
        "category": "trade",
        "balance": null,
        "source": "manual",
    })
}

pub(super) fn normalize_local_row(act: &Value) -> Value {
    let mut act: Map<String, Value> = act.as_object().cloned().unwrap_or_default();
    let mut source = s(act.get("source"));
    if source.is_empty() {
        source = "manual".into();
    }
    if source == "wealthsimple" {
        if let Some(cid) = bagholder_store::activities::canonical_from_row(&Value::Object(act.clone()), "wealthsimple") {
            act.insert("canonicalId".into(), json!(cid));
            act.insert("source".into(), json!("wealthsimple"));
            return Value::Object(act);
        }
        source = "manual".into();
    }
    act.insert("source".into(), json!(source));
    act.shift_remove("canonicalId");
    act.shift_remove("canonical_id");
    if !truthy(act.get("accountId")) {
        act.insert("accountId".into(), json!("manual"));
    }
    if !truthy(act.get("bookId")) {
        let a = act.get("accountId").cloned().unwrap_or(Value::Null);
        act.insert("bookId".into(), a);
    }
    if !truthy(act.get("id")) || bagholder_store::activities::looks_like_homemade_id(&s(act.get("id"))) {
        act.insert("id".into(), json!(uuid4()));
    }
    if !truthy(act.get("occurredAt")) {
        let t = act.get("transactionDate").filter(|v| truthy(Some(v))).cloned().unwrap_or(json!(""));
        act.insert("occurredAt".into(), t);
    }
    Value::Object(act)
}

pub fn append_manual(app: &Arc<App>, body: &Value) -> Value {
    let rows: Vec<Value> = if let Some(Value::Array(a)) = body.get("activities") {
        a.iter().filter(|r| r.is_object()).cloned().collect()
    } else if truthy(body.get("activity")) && body.get("activity").map_or(false, |a| a.is_object()) {
        vec![body["activity"].clone()]
    } else {
        vec![manual_from_fields(app, body)]
    };
    let rows: Vec<Value> = rows.iter().map(normalize_local_row).collect();
    let conn = db(app);
    let result = must(bagholder_store::merge::merge_local_rows(&conn, &rows, &uuid4));
    let mut snap = snapshot(app);
    if !tr(&snap, "syncedAt") {
        let stamp = now_iso();
        must(bagholder_store::tables::set_meta(&conn, "synced_at", &stamp));
        snap = snapshot(app);
    }
    {
        let mut st = app.state.lock().unwrap();
        let synced = f(&snap, "syncedAt");
        if !synced.is_empty() {
            st.last_sync = synced;
        }
    }
    let saved = result.activities;
    let mut out = json!({"ok": true, "added": result.added, "duplicates": result.duplicates});
    if saved.len() == 1 {
        set(&mut out, "activity", saved[0].clone());
    } else if !saved.is_empty() {
        set(&mut out, "activities", Value::Array(saved));
    } else if rows.len() == 1 {
        set(&mut out, "activity", rows[0].clone());
    }
    out
}
