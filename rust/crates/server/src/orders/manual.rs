//! A fill the person enters by hand.

use super::*;
use bagholder_store::activities::ActivityRow;
use serde::Serialize;

// ---------------------------------------------------------------------------
// manual activity
// ---------------------------------------------------------------------------

/// `POST /api/book/append`'s body: either whole rows (`activities`, or the
/// singular `activity`), or the ticket-shaped fields a hand-entered trade
/// sends -- each with the alternative spellings the page and the phones have
/// used, read in the precedence `manual_from_fields` resolves.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct BookAppend {
    #[serde(deserialize_with = "bagholder_model::lenient::list")]
    pub activities: Vec<ActivityRow>,
    pub activity: Option<ActivityRow>,
    #[serde(deserialize_with = "bagholder_model::lenient::text")]
    pub side: String,
    #[serde(deserialize_with = "bagholder_model::lenient::maybe_number")]
    pub qty: Option<f64>,
    #[serde(deserialize_with = "bagholder_model::lenient::maybe_number")]
    pub quantity: Option<f64>,
    #[serde(deserialize_with = "bagholder_model::lenient::maybe_number")]
    pub price: Option<f64>,
    #[serde(deserialize_with = "bagholder_model::lenient::maybe_number")]
    pub unit_price: Option<f64>,
    #[serde(deserialize_with = "bagholder_model::lenient::text")]
    pub date: String,
    #[serde(deserialize_with = "bagholder_model::lenient::text")]
    pub transaction_date: String,
    #[serde(deserialize_with = "bagholder_model::lenient::text")]
    pub occurred_at: String,
    #[serde(deserialize_with = "bagholder_model::lenient::text")]
    pub symbol: String,
    #[serde(deserialize_with = "bagholder_model::lenient::text")]
    pub currency: String,
    #[serde(deserialize_with = "bagholder_model::lenient::text")]
    pub account_id: String,
    #[serde(deserialize_with = "bagholder_model::lenient::text")]
    pub account: String,
    #[serde(deserialize_with = "bagholder_model::lenient::text")]
    pub account_type: String,
    #[serde(deserialize_with = "bagholder_model::lenient::maybe_number")]
    pub commission: Option<f64>,
}

fn first_nonempty(vs: &[&str]) -> String {
    vs.iter().find(|v| !v.is_empty()).map(|v| v.to_string()).unwrap_or_default()
}

fn date_only_str(t: &str) -> String {
    let t = t.trim();
    if t.is_empty() {
        return String::new();
    }
    t.split('T').next().unwrap_or("").chars().take(10).collect()
}

pub(super) fn manual_from_fields(body: &BookAppend) -> ActivityRow {
    let mut side = body.side.trim().to_uppercase();
    if side != "BUY" && side != "SELL" {
        side = "BUY".into();
    }
    let qty = body.qty.or(body.quantity).unwrap_or(0.0).abs();
    let px = body.price.or(body.unit_price).unwrap_or(0.0).abs();
    let mut date = date_only_str(&first_nonempty(&[&body.date, &body.transaction_date, &body.occurred_at]));
    if date.is_empty() {
        date = crate::app::today_utc();
    }
    let symbol = body.symbol.trim().to_uppercase();
    let mut currency = body.currency.trim().to_uppercase();
    if currency.is_empty() {
        currency = "CAD".into();
    }
    if currency != "CAD" && currency != "USD" {
        currency = "CAD".into();
    }
    let mut account_id = first_nonempty(&[&body.account_id, &body.account]);
    if account_id.is_empty() {
        account_id = "manual".into();
    }
    let buy = side == "BUY";
    let signed_qty = if buy { qty } else { -qty };
    let cash = if buy { -(qty * px) } else { qty * px };
    let account_type = if !body.account_type.is_empty() {
        body.account_type.clone()
    } else if account_id == "manual" {
        "Manual".into()
    } else {
        String::new()
    };
    let desc = format!(
        "{}{}",
        if buy { "Buy" } else { "Sell" },
        if symbol.is_empty() { String::new() } else { format!(" {} {} @ {}", fmt_g(qty), symbol, fmt_g(px)) }
    );
    ActivityRow {
        id: uuid4(),
        occurred_at: date.clone(),
        transaction_date: date.clone(),
        settlement_date: date,
        account_id: account_id.clone(),
        book_id: account_id,
        account_type,
        activity_type: "Trade".into(),
        activity_sub_type: side,
        description: desc,
        direction: if buy { "DEBIT" } else { "CREDIT" }.into(),
        symbol: symbol.clone(),
        name: symbol,
        currency,
        quantity: signed_qty,
        unit_price: px,
        commission: body.commission.unwrap_or(0.0).abs(),
        net_cash_amount: cash,
        category: "trade".into(),
        source: "manual".into(),
        ..ActivityRow::default()
    }
}

pub(super) fn normalize_local_row(act: ActivityRow) -> ActivityRow {
    let mut act = act;
    let mut source = act.source.clone();
    if source.is_empty() {
        source = "manual".into();
    }
    if source == "wealthsimple" {
        if let Some(cid) = bagholder_store::activities::canonical_from_row(&act, "wealthsimple") {
            act.canonical_id = Some(cid);
            act.source = "wealthsimple".into();
            return act;
        }
        source = "manual".into();
    }
    act.source = source;
    act.canonical_id = None;
    if act.account_id.is_empty() {
        act.account_id = "manual".into();
    }
    if act.book_id.is_empty() {
        act.book_id = act.account_id.clone();
    }
    if act.id.is_empty() || bagholder_store::activities::looks_like_homemade_id(&act.id) {
        act.id = uuid4();
    }
    if act.occurred_at.is_empty() {
        act.occurred_at = act.transaction_date.clone();
    }
    act
}

/// `POST /api/book/append`'s answer: what was added and, when it is one row
/// or the row the person's own ticket described, that row back.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
pub struct Appended {
    pub ok: bool,
    pub added: usize,
    pub duplicates: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activity: Option<ActivityRow>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activities: Option<Vec<ActivityRow>>,
}

pub fn append_manual(app: &Arc<App>, body: &BookAppend) -> Value {
    let rows: Vec<ActivityRow> = if !body.activities.is_empty() {
        body.activities.clone()
    } else if let Some(a) = &body.activity {
        vec![a.clone()]
    } else {
        vec![manual_from_fields(body)]
    };
    let rows: Vec<ActivityRow> = rows.into_iter().map(normalize_local_row).collect();
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
    let mut out = Appended { ok: true, added: result.added, duplicates: result.duplicates, activity: None, activities: None };
    if saved.len() == 1 {
        out.activity = Some(saved[0].clone());
    } else if !saved.is_empty() {
        out.activities = Some(saved);
    } else if rows.len() == 1 {
        out.activity = Some(rows[0].clone());
    }
    serde_json::to_value(&out).unwrap_or(Value::Null)
}
