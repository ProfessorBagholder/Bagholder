//! One filter object applied to the whole base: `portfolio_view`,
//! `cashflow_view` and `build_view`.
//!
//! Per-instrument figures stay in the instrument's own currency; everything
//! that adds instruments together is CAD.

use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::base::Base;
use crate::dates::shift_date;
use crate::exposure::exposure_slices;
use crate::filters::{
    clean_filters, date_bounds, in_date_scope, position_matches, trade_matches, Filters, BENCHMARK_LABELS,
};
use crate::fx::to_cad;
use crate::nav::{annualized, drawdown, series_json, yearly_returns};
use crate::normalize::KINDS;
use crate::stats::{by_symbol, grade_buckets, metrics, month_label, monthly, payments_per_year, review_queue, GRADES};
use crate::trades::quote_fits;
use crate::value::{FSum, field_s, get, num};

fn opt_num(v: Option<&Value>) -> Option<f64> {
    match v {
        None | Some(Value::Null) => None,
        Some(x) => {
            let n = num(Some(x), f64::NAN);
            if n.is_nan() { None } else { Some(n) }
        }
    }
}

fn b(v: &Value, k: &str) -> f64 { num(get(v, k), 0.0) }
fn flag(v: &Value, k: &str) -> bool { v.get(k).and_then(|x| x.as_bool()).unwrap_or(false) }

/// `portfolio_view`: the Portfolio tiles, CAD aggregates over the
/// accounts in scope.
///
/// Market value, cost basis and unrealized P&L come from the open positions in
/// scope at today's rate. Net asset value is the sum of Wealthsimple's net
/// liquidation value per account, margin used the negative cash balances per
/// currency, available margin the buying power of the margin accounts only --
/// every self-directed account answers that query with its cash to buy with,
/// which is not margin.
pub fn portfolio_view(base: &Base, f: &Filters, positions: &[Value]) -> Value {
    let today = base.today.clone();
    let cad = |amount: f64, currency: &str| to_cad(&base.fx, amount, currency, &today);

    let names = f.list("account");
    // closed accounts hold nothing and count for nothing here
    let accounts: Vec<&Value> = base
        .accounts
        .iter()
        .filter(|a| field_s(a, "status").to_lowercase() != "closed")
        .filter(|a| names.is_empty() || names.contains(&field_s(a, "name")))
        .collect();
    let ids: BTreeSet<String> = accounts.iter().map(|a| field_s(a, "id")).collect();
    let name_of: HashMap<String, String> =
        accounts.iter().map(|a| (field_s(a, "id"), field_s(a, "name"))).collect();

    let mv: f64 = positions
        .iter()
        .map(|p| cad(if flag(p, "short") { -b(p, "mv") } else { b(p, "mv") }, &field_s(p, "currency")))
        .fsum();
    let cost: f64 = positions.iter().map(|p| cad(b(p, "cost").abs(), &field_s(p, "currency"))).fsum();
    let unreal: f64 = positions.iter().map(|p| cad(b(p, "unreal"), &field_s(p, "currency"))).fsum();

    let navs: Vec<f64> = accounts
        .iter()
        .filter_map(|a| opt_num(get(a, "nav")).map(|n| cad(n, &field_s(a, "currency"))))
        .collect();

    // the negative cash balances are the margin drawn; the positive ones the cash
    let mut used: BTreeMap<String, f64> = BTreeMap::new();
    let mut cash_by: BTreeMap<String, f64> = BTreeMap::new();
    for bal in base.balances.iter() {
        let aid = field_s(bal, "accountId");
        let ccy = base.cash_currencies.get(&field_s(bal, "securityId")).cloned();
        let q = num(get(bal, "quantity"), 0.0);
        if let Some(ccy) = ccy {
            if ids.contains(&aid) && !ccy.is_empty() {
                if q < 0.0 {
                    *used.entry(ccy).or_insert(0.0) += -q;
                } else if q > 0.0 {
                    *cash_by.entry(ccy).or_insert(0.0) += q;
                }
            }
        }
    }
    let margin_used: f64 = used.iter().map(|(c, v)| cad(*v, c)).fsum();
    let cash: f64 = cash_by.iter().map(|(c, v)| cad(*v, c)).fsum();

    // the day's change: each quoted position's, over what those were worth at the previous close
    let quoted: Vec<&Value> = positions.iter().filter(|p| opt_num(get(p, "dayChange")).is_some()).collect();
    let day_change: Option<f64> = if quoted.is_empty() {
        None
    } else {
        Some(quoted.iter().map(|p| cad(b(p, "dayChange"), &field_s(p, "currency"))).fsum())
    };
    let prev_value = match day_change {
        Some(dc) => {
            quoted
                .iter()
                .map(|p| cad(if flag(p, "short") { -b(p, "mv") } else { b(p, "mv") }, &field_s(p, "currency")))
                .fsum()
                - dc
        }
        None => 0.0,
    };

    // only a margin account's buying power is margin available
    let margin_ids: BTreeSet<String> = accounts
        .iter()
        .filter(|a| field_s(a, "type").to_uppercase().contains("MARGIN"))
        .map(|a| field_s(a, "id"))
        .collect();
    let mut avail: Vec<f64> = Vec::new();
    let mut unavailable: Vec<String> = Vec::new();
    for m in base.margin.iter() {
        let aid = field_s(m, "accountId");
        if !margin_ids.contains(&aid) {
            continue;
        }
        match opt_num(get(m, "buyingPower")) {
            None => unavailable.push(name_of.get(&aid).cloned().unwrap_or(aid)),
            Some(bp) => {
                let ccy = { let c = field_s(m, "currency"); if c.is_empty() { "CAD".into() } else { c } };
                avail.push(cad(bp, &ccy));
            }
        }
    }
    unavailable.sort();

    let mut alloc: Vec<Value> = positions
        .iter()
        .filter_map(|p| {
            let v = cad(b(p, "mv"), &field_s(p, "currency"));
            if v > 0.0 {
                Some(json!({"id": field_s(p, "id"), "symbol": field_s(p, "symbol"), "account": field_s(p, "account"), "value": v}))
            } else {
                None
            }
        })
        .collect();
    alloc.sort_by(|x, y| b(y, "value").partial_cmp(&b(x, "value")).unwrap_or(std::cmp::Ordering::Equal));
    let alloc_total: f64 = alloc.iter().map(|x| b(x, "value")).fsum();
    for x in alloc.iter_mut() {
        let share = if alloc_total != 0.0 { b(x, "value") / alloc_total } else { 0.0 };
        if let Value::Object(m) = x {
            m.insert("share".into(), json!(share));
        }
    }

    let (sectors, regions) = exposure_slices(positions, &base.exposures, &cad);
    let nav_sum: f64 = navs.iter().fsum();
    let account_count: BTreeSet<String> = positions.iter().map(|p| field_s(p, "account")).collect();

    json!({
        "allocation": alloc,
        "sectors": sectors,
        "regions": regions,
        "marketValue": crate::value::sum_of(positions.is_empty(), mv),
        "costBasis": crate::value::sum_of(positions.is_empty(), cost),
        "unrealized": crate::value::sum_of(positions.is_empty(), unreal),
        "unrealizedPct": if cost != 0.0 { json!(unreal / cost) } else { Value::Null },
        "positionCount": positions.len(),
        "accountCount": account_count.len(),
        "nav": if navs.is_empty() { Value::Null } else { json!(nav_sum) },
        "navAccounts": navs.len(),
        "marginUsed": crate::value::sum_of(used.is_empty(), margin_used),
        "marginUsedBy": used.iter().map(|(c, v)| (c.clone(), json!(round2(*v)))).collect::<Map<String, Value>>(),
        "marginUsedPct": if mv != 0.0 { json!(margin_used / mv) } else { Value::Null },
        "availableMargin": if avail.is_empty() { Value::Null } else { json!(avail.iter().fsum()) },
        "availableMarginUnavailable": unavailable,
        // the tiles a book without a margin account shows in the margin tiles' places
        "hasMargin": !margin_ids.is_empty(),
        "cash": crate::value::sum_of(cash_by.is_empty(), cash),
        "cashPct": if !navs.is_empty() && nav_sum != 0.0 { json!(cash / nav_sum) } else { Value::Null },
        "dayChange": day_change,
        "dayChangePct": match day_change {
            Some(dc) if !quoted.is_empty() && prev_value != 0.0 => json!(dc / prev_value),
            _ => Value::Null,
        },
    })
}

/// Rounds to two places, half to even.
fn round2(v: f64) -> f64 {
    let scaled = v * 100.0;
    let r = scaled.round();
    let r = if (scaled - scaled.trunc()).abs() == 0.5 && r % 2.0 != 0.0 { r - scaled.signum() } else { r };
    r / 100.0
}

struct Rate {
    per: f64,
    freq: i64,
    annual: f64,
    verified: bool,
    source: &'static str,
}

/// `cashflow_view`.
pub fn cashflow_view(base: &Base, f: &Filters, positions_all: &[Value], margin_used: f64, has_margin: bool) -> Value {
    let today = base.today.clone();
    let accts = f.list("account");
    let search = f.search.to_uppercase();
    let sym_list = f.list("symbol");

    let in_scope = |r: &Value| -> bool {
        if !accts.is_empty() && !accts.contains(&field_s(r, "account")) {
            return false;
        }
        if !search.is_empty() && !field_s(r, "symbol").to_uppercase().contains(&search) {
            return false;
        }
        if !sym_list.is_empty() && !sym_list.contains(&field_s(r, "symbol")) {
            return false;
        }
        in_date_scope(f, &today, &field_s(r, "date"))
    };

    let everything: Vec<&Value> = base.cashflow.iter().filter(|r| in_scope(r)).collect();
    let recs: Vec<&Value> = everything.iter().copied().filter(|r| field_s(r, "kind") == "Dividend").collect();

    let mut skipped: Vec<String> = ["grade", "tag", "kind", "exchange", "side", "result"]
        .iter()
        .filter(|k| !f.list(k).is_empty())
        .map(|k| k.to_string())
        .collect();
    // the ranges, in their declared order
    for k in crate::filters::RANGE_KEYS {
        if f.ranges[k].v.is_some() {
            skipped.push(k.to_string());
        }
    }

    // The chart runs to the current month (or the end of the date filter), with
    // an empty bar for a month that has not paid yet.
    let mut keys: Vec<String> = Vec::new();
    let mut bucket: HashMap<String, (f64, usize)> = HashMap::new();
    if !recs.is_empty() {
        let mut months_seen: Vec<String> = recs.iter().map(|r| field_s(r, "date").chars().take(7).collect()).collect();
        months_seen.sort();
        let first = months_seen[0].clone();
        let mut last = months_seen[months_seen.len() - 1].clone();
        let mut end_day = today.clone();
        if let Some((_, hi)) = date_bounds(f, &today) {
            end_day = if hi < today { hi } else { today.clone() };
        } else if !f.years.is_empty() {
            let y_end = format!("{}-12-31", f.years.iter().max().unwrap());
            end_day = if y_end < today { y_end } else { today.clone() };
        }
        let end_month: String = end_day.chars().take(7).collect();
        if end_month > last {
            last = end_month;
        }
        let mut y: i64 = first[..4].parse().unwrap_or(0);
        let mut m: u32 = first[5..7].parse().unwrap_or(1);
        loop {
            let k = format!("{:04}-{:02}", y, m);
            if k > last {
                break;
            }
            keys.push(k.clone());
            bucket.insert(k, (0.0, 0));
            m += 1;
            if m > 12 {
                m = 1;
                y += 1;
            }
        }
    }
    for r in &recs {
        let k: String = field_s(r, "date").chars().take(7).collect();
        if let Some(e) = bucket.get_mut(&k) {
            e.0 += b(r, "amountCad");
            e.1 += 1;
        }
    }
    let months: Vec<Value> = keys
        .iter()
        .map(|k| {
            let (sum, n) = bucket[k];
            json!({"key": k, "label": month_label(k), "value": sum, "count": n})
        })
        .collect();

    let payers: BTreeSet<String> = base
        .cashflow
        .iter()
        .filter(|r| field_s(r, "kind") == "Dividend")
        .map(|r| field_s(r, "symbol"))
        .collect();
    let held: Vec<&Value> = positions_all
        .iter()
        .filter(|p| payers.contains(&field_s(p, "symbol")) && !flag(p, "short"))
        .filter(|p| accts.is_empty() || accts.contains(&field_s(p, "account")))
        .filter(|p| search.is_empty() || field_s(p, "symbol").to_uppercase().contains(&search))
        .collect();
    let for_yoc: Vec<&Value> = base
        .cashflow
        .iter()
        .filter(|r| field_s(r, "kind") == "Dividend")
        .filter(|r| accts.is_empty() || accts.contains(&field_s(r, "account")))
        .filter(|r| search.is_empty() || field_s(r, "symbol").to_uppercase().contains(&search))
        .collect();

    let last_rec = recs.first().map(|r| field_s(r, "date")).unwrap_or_else(|| today.clone());
    let cut = trailing_year_month(&last_rec);
    let this_year: String = today.chars().take(4).collect();

    let sum_for = |sym: &str, pred: &dyn Fn(&Value) -> bool| -> Value {
        let v: Vec<f64> = for_yoc.iter().filter(|r| field_s(r, "symbol") == sym && pred(r)).map(|r| b(r, "amountCad")).collect();
        crate::value::sum_of(v.is_empty(), v.iter().fsum())
    };

    // The fund's own declared record first: the latest distribution that has
    // gone ex, and payments per year from the gaps between its ex-dates, so a
    // schedule change shows at once. Never assumed from the instrument.
    let rate_for = |sym: &str| -> Option<Rate> {
        let public: Vec<&Value> = base
            .distributions
            .get(sym)
            .and_then(|v| v.as_array())
            .map(|a| a.iter().collect())
            .unwrap_or_default();
        let mut declared: Vec<&&Value> = public.iter().filter(|d| field_s(d, "exDate") <= today).collect();
        if !declared.is_empty() {
            declared.sort_by(|a, b2| field_s(b2, "exDate").cmp(&field_s(a, "exDate")));
            let per = b(declared[0], "amount");
            let freq = payments_per_year(&public.iter().map(|d| field_s(d, "exDate")).collect::<Vec<_>>());
            if per != 0.0 {
                if let Some(freq) = freq {
                    return Some(Rate { per, freq, annual: per * freq as f64, verified: true, source: "declared" });
                }
            }
        }
        let mut rs: Vec<&&Value> = for_yoc
            .iter()
            .filter(|r| field_s(r, "symbol") == sym && opt_num(get(r, "per")).map_or(false, |p| p != 0.0))
            .collect();
        if rs.is_empty() {
            return None;
        }
        rs.sort_by(|a, b2| field_s(b2, "date").cmp(&field_s(a, "date")));
        let per = b(rs[0], "per");
        if per == 0.0 {
            return None;
        }
        let dates: Vec<String> = for_yoc.iter().filter(|r| field_s(r, "symbol") == sym).map(|r| field_s(r, "date")).collect();
        let freq = payments_per_year(&dates);
        let verified = freq.is_some();
        let freq = freq.unwrap_or(12);
        Some(Rate { per, freq, annual: per * freq as f64, verified, source: "payments" })
    };

    // The next distribution still to be paid, whether or not it has gone ex,
    // else the last known one.
    let distribution_dates = |sym: &str| -> (String, String, bool, bool) {
        let mut recs_: Vec<&Value> = base
            .distributions
            .get(sym)
            .and_then(|v| v.as_array())
            .map(|a| a.iter().collect())
            .unwrap_or_default();
        let key = |d: &Value| -> (String, String) {
            let pay: String = field_s(d, "payDate").chars().take(10).collect();
            let ex = field_s(d, "exDate");
            (if pay.is_empty() { ex.clone() } else { pay }, ex)
        };
        recs_.sort_by(|a, b2| key(a).cmp(&key(b2)));
        let unpaid: Vec<&&Value> = recs_.iter().filter(|d| key(d).0 >= today).collect();
        let pick: Option<&Value> = unpaid.first().map(|d| **d).or_else(|| recs_.last().copied());
        let (ex, pay) = match pick {
            Some(p) => (field_s(p, "exDate"), field_s(p, "payDate").chars().take(10).collect::<String>()),
            None => {
                let q = base.quotes.get(sym).cloned().unwrap_or(Value::Null);
                let ex: String = field_s(&q, "exDividendDate").chars().take(10).collect();
                let mut paid: Vec<String> =
                    for_yoc.iter().filter(|r| field_s(r, "symbol") == sym).map(|r| field_s(r, "date")).collect();
                paid.sort();
                (ex, paid.last().cloned().unwrap_or_default())
            }
        };
        let ex_past = !ex.is_empty() && ex < today;
        let pay_past = !pay.is_empty() && pay < today;
        (ex, pay, ex_past, pay_past)
    };

    let last_price = |p: &Value| -> (f64, &'static str) {
        let sym = field_s(p, "symbol");
        let q = base.quotes.get(&sym);
        let q = match q { Some(q) if quote_fits(Some(q), &field_s(p, "kind")) => Some(q), _ => None };
        if let Some(q) = q {
            if let Some(px) = opt_num(get(q, "price")) {
                if px > 0.0 {
                    return (px, "close");
                }
            }
        }
        (b(p, "last"), "fill")
    };

    let mut holdings: Vec<Value> = Vec::new();
    for p in &held {
        let sym = field_s(p, "symbol");
        let r = rate_for(&sym);
        let basis = b(p, "cost");
        let avg = b(p, "avg");
        let (last_px, price_source) = last_price(p);
        let (ex, pay, ex_past, pay_past) = distribution_dates(&sym);
        let qty = b(p, "qty");
        holdings.push(json!({
            "id": field_s(p, "id"),
            "symbol": sym,
            "account": field_s(p, "account"),
            "qty": qty,
            "per": r.as_ref().map(|x| x.per),
            "freq": r.as_ref().map(|x| x.freq),
            "freqVerified": r.as_ref().map_or(false, |x| x.verified),
            "rateSource": r.as_ref().map_or("", |x| x.source),
            "cost": basis,
            "avg": avg,
            "last": last_px,
            "priceSource": price_source,
            "ytd": sum_for(&sym, &|x| field_s(x, "date").starts_with(&this_year)),
            "ttm": sum_for(&sym, &|x| field_s(x, "date").chars().take(7).collect::<String>() >= cut),
            "all": sum_for(&sym, &|_| true),
            "nextExDate": ex,
            "nextPayDate": pay,
            "exPast": ex_past,
            "payPast": pay_past,
            "yob": r.as_ref().map(|x| x.per * qty),
            "annual": r.as_ref().map(|x| x.annual * qty),
            "yoc": r.as_ref().and_then(|x| if avg != 0.0 { Some(x.annual / avg) } else { None }),
            "currentYield": r.as_ref().and_then(|x| if last_px != 0.0 { Some(x.annual / last_px) } else { None }),
        }));
    }

    let verified: Vec<&Value> = holdings.iter().filter(|h| !h["annual"].is_null()).collect();
    let basis_all: f64 = verified.iter().map(|h| b(h, "cost")).fsum();
    let earned_all: f64 = verified.iter().map(|h| b(h, "ttm")).fsum();
    let annual_all: f64 = verified.iter().map(|h| b(h, "annual")).fsum();
    let total: f64 = recs.iter().map(|r| b(r, "amountCad")).fsum();

    let this_yr: i64 = this_year.parse().unwrap_or(0);
    let mut tiles: Vec<Value> = Vec::new();
    for y in [this_yr - 2, this_yr - 1, this_yr] {
        let ys = y.to_string();
        let rs: Vec<&&Value> = recs.iter().filter(|r| field_s(r, "date").starts_with(&ys)).collect();
        let sm: f64 = rs.iter().map(|r| b(r, "amountCad")).fsum();
        let paid = keys.iter().filter(|k| k.starts_with(&ys) && bucket[*k].1 > 0).count().max(1);
        tiles.push(json!({
            "label": if y == this_yr { format!("{} YTD", y) } else { ys.clone() },
            "total": crate::value::sum_of(rs.is_empty(), sm),
            "perMonth": sm / paid as f64,
            "count": rs.len(),
        }));
    }
    let months_in_scope = keys.iter().filter(|k| bucket[*k].1 > 0).count().max(1);
    tiles.push(json!({"label": "All time", "total": crate::value::sum_of(recs.is_empty(), total), "perMonth": total / months_in_scope as f64, "count": recs.len()}));

    if has_margin {
        // margin used is the Portfolio tab's figure; under it the average
        // margin interest per charged month
        let charges: Vec<&&Value> = everything.iter().filter(|r| field_s(r, "kind") == "Interest charge").collect();
        let charge_months: BTreeSet<String> =
            charges.iter().map(|r| field_s(r, "date").chars().take(7).collect()).collect();
        let charged: f64 = charges.iter().map(|r| -b(r, "amountCad")).fsum();
        tiles.push(json!({
            "label": "Margin used",
            "marginUsed": margin_used,
            "interestPerMonth": if charge_months.is_empty() { 0.0 } else { charged / charge_months.len() as f64 },
            "interestMonths": charge_months.len(),
        }));
    } else {
        // without a margin account: the trailing twelve months, averaged over
        // the months that paid
        let since = shift_date(&today, -365);
        let window: Vec<&&Value> = recs
            .iter()
            .filter(|r| { let d = field_s(r, "date"); d > since && d <= today })
            .collect();
        let sm: f64 = window.iter().map(|r| b(r, "amountCad")).fsum();
        let paid: BTreeSet<String> = window.iter().map(|r| field_s(r, "date").chars().take(7).collect()).collect();
        let paid = paid.len().max(1);
        tiles.push(json!({"label": "Last 12 months", "total": crate::value::sum_of(window.is_empty(), sm), "perMonth": sm / paid as f64, "count": window.len()}));
    }
    tiles.push(json!({
        "label": "Yield on cost",
        "yield": if basis_all != 0.0 { json!(annual_all / basis_all) } else { Value::Null },
        "projected": annual_all / 12.0,
        "earned": crate::value::sum_of(verified.is_empty(), earned_all),
        "book": crate::value::sum_of(verified.is_empty(), basis_all),
    }));

    let other: Vec<&&Value> = everything.iter().filter(|r| field_s(r, "kind") != "Dividend").collect();
    json!({
        "tiles": tiles,
        "months": months,
        "holdings": holdings,
        "rows": recs,
        "other": other,
        "total": crate::value::sum_of(recs.is_empty(), total),
        "count": recs.len(),
        "skippedFilters": skipped,
        "interest": other.iter().filter(|r| field_s(r, "kind") == "Interest").map(|r| b(r, "amountCad")).fsum(),
        "withholding": other.iter().filter(|r| field_s(r, "kind") == "Withholding tax").map(|r| b(r, "amountCad")).fsum(),
    })
}

/// Eleven months back from a date, as `YYYY-MM`.
fn trailing_year_month(day: &str) -> String {
    let y: i64 = day[..4].parse().unwrap_or(0);
    let m: i64 = day[5..7].parse().unwrap_or(1);
    let mut cm = m - 11;
    let mut cy = y;
    while cm <= 0 {
        cm += 12;
        cy -= 1;
    }
    format!("{:04}-{:02}", cy, cm)
}

/// `build_view`.
pub fn build_view(base: &Base, filters: Option<&Value>) -> Value {
    let f = clean_filters(filters);
    let today = base.today.clone();

    let trades: Vec<Value> = base.trades.iter().filter(|t| trade_matches(t, &f, &today)).cloned().collect();
    // performance stats score only trades with a known entry basis: a deposited
    // (transferred-in) coin has no buy made here and cannot be scored
    let scored: Vec<Value> = trades
        .iter()
        .filter(|t| !t.get("flags").and_then(|v| v.as_array()).map(|a| a.iter().any(|f| f == "basis-unknown")).unwrap_or(false))
        .cloned()
        .collect();
    let positions: Vec<Value> = base.positions.iter().filter(|p| position_matches(p, &f)).cloned().collect();

    let accts = f.list("account");
    let (series, series_label) = if accts.len() == 1 && base.equity_by_account.contains_key(&accts[0]) {
        (base.equity_by_account[&accts[0]].clone(), accts[0].clone())
    } else {
        ((*base.equity).clone(), "All accounts".to_string())
    };

    let bench_key = f.benchmark.clone();
    let empty = BTreeMap::new();
    let bench = base.benchmarks.get(&bench_key).unwrap_or(&empty);
    let years = yearly_returns(&series, bench, &today);
    let ann = annualized(&years);
    let dd = drawdown(&series);
    let bounds = date_bounds(&f, &today);
    let portfolio = portfolio_view(base, &f, &positions);

    let mut shown: Vec<&crate::nav::Point> = match &bounds {
        Some((lo, hi)) => series.iter().filter(|p| *lo <= p.d && p.d <= *hi).collect(),
        None if !f.years.is_empty() => series
            .iter()
            .filter(|p| f.years.contains(&p.d.chars().take(4).collect::<String>()))
            .collect(),
        None => series.iter().collect(),
    };
    if !shown.is_empty() {
        // the pre-history a chart should not start from
        let peak = shown.iter().map(|p| p.v).fold(f64::NEG_INFINITY, f64::max);
        let first_idx = shown.iter().position(|p| p.v > peak * 0.01).unwrap_or(0);
        shown = shown.split_off(first_idx);
    }
    let shown_owned: Vec<crate::nav::Point> = shown.into_iter().cloned().collect();

    let mut tags: Vec<String> = base
        .trades
        .iter()
        .flat_map(|t| t.get("tags").and_then(|v| v.as_array()).cloned().unwrap_or_default())
        .map(|x| crate::value::s(Some(&x)))
        .collect();
    tags.sort();
    tags.dedup();

    let mut symbols: Vec<String> = base
        .trades
        .iter()
        .chain(base.positions.iter())
        .map(|r| field_s(r, "symbol"))
        .collect();
    symbols.sort();
    symbols.dedup();

    // what the ⌘K list shows beside each symbol: its name, exchange and kind,
    // from the rows that carry it (a name that is only the symbol counts as none)
    let mut listings: Map<String, Value> = Map::new();
    let mut listing_order: Vec<String> = Vec::new();
    for r in base.trades.iter().chain(base.positions.iter()) {
        let sym = field_s(r, "symbol");
        if !listings.contains_key(&sym) {
            listing_order.push(sym.clone());
            listings.insert(
                sym.clone(),
                json!({"name": "", "exchange": "", "kind": field_s(r, "kind"), "currency": field_s(r, "currency")}),
            );
        }
        let cur = listings.get_mut(&sym).unwrap();
        let name = field_s(r, "name");
        if field_s(cur, "name").is_empty() && !name.is_empty() && name != sym {
            cur["name"] = json!(name);
        }
        let exch = field_s(r, "exchange");
        if field_s(cur, "exchange").is_empty() && !exch.is_empty() {
            cur["exchange"] = json!(exch);
        }
    }

    let mut accounts: Vec<String> = base
        .trades
        .iter()
        .chain(base.positions.iter())
        .chain(base.cashflow.iter())
        .map(|r| field_s(r, "account"))
        .collect();
    accounts.sort();
    accounts.dedup();

    let mut exchanges: Vec<String> = base
        .trades
        .iter()
        .chain(base.positions.iter())
        .map(|r| field_s(r, "exchange"))
        .filter(|e| !e.is_empty())
        .collect();
    exchanges.sort();
    exchanges.dedup();

    let kinds: Vec<&str> = KINDS
        .iter()
        .copied()
        .filter(|k| {
            base.trades.iter().any(|t| field_s(t, "kind") == *k) || base.positions.iter().any(|p| field_s(p, "kind") == *k)
        })
        .collect();

    let mut year_options: Vec<String> = base
        .trades
        .iter()
        .map(|t| field_s(t, "exitDate"))
        .filter(|d| !d.is_empty())
        .map(|d| d.chars().take(4).collect())
        .collect();
    year_options.sort();
    year_options.dedup();
    year_options.reverse();

    let book: f64 = positions.iter().map(|p| b(p, "cost").abs()).fsum();
    let mv: f64 = positions.iter().map(|p| if flag(p, "short") { -b(p, "mv") } else { b(p, "mv") }).fsum();
    let unreal: f64 = positions.iter().map(|p| b(p, "unreal")).fsum();

    let margin_used = b(&portfolio, "marginUsed");
    let has_margin = flag(&portfolio, "hasMargin");
    let mut cashflow = cashflow_view(base, &f, &base.positions, margin_used, has_margin);
    if let Some(tiles) = cashflow.get_mut("tiles").and_then(|t| t.as_array_mut()) {
        for t in tiles.iter_mut().filter(|t| t.get("marginUsed").is_some()) {
            t["marginUsed"] = portfolio["marginUsed"].clone();
        }
    }

    let mut grades: Vec<String> = GRADES.iter().map(|g| g.to_string()).collect();
    grades.push("Ungraded".into());

    json!({
        "ok": true,
        "generated": crate::clock::now_utc_stamp(),
        "today": today,
        "syncedAt": base.synced_at,
        "currency": "CAD",
        "market": {"fxLast": base.fx_last, "benchmarkLast": base.benchmark_last},
        "filters": f.to_json(),
        "options": {
            "accounts": accounts,
            "symbols": symbols,
            "listings": listings,
            "tags": tags,
            "exchanges": exchanges,
            "kinds": kinds,
            "grades": grades,
            "sides": ["SELL", "COVER"],
            "results": ["Winners", "Losers", "Breakeven"],
            "years": year_options,
        },
        "kpi": metrics(&scored),
        "equity": {
            "label": series_label,
            "series": series_json(&shown_owned),
            "drawdown": dd,
            "annualized": ann,
        },
        "years": years,
        "benchmark": {
            "key": bench_key.clone(),
            "label": BENCHMARK_LABELS.iter().find(|(k, _)| *k == bench_key).map(|(_, v)| *v).unwrap_or(""),
        },
        "monthly": monthly(&scored),
        "bySymbol": by_symbol(&scored),
        "grades": grade_buckets(&trades),
        "queue": review_queue(&trades),
        "trades": trades,
        "tradeCount": trades.len(),
        "tradeTotal": base.trades.len(),
        "positions": positions,
        "positionsSummary": {"count": positions.len(), "book": crate::value::sum_of(positions.is_empty(), book), "mv": crate::value::sum_of(positions.is_empty(), mv), "unreal": crate::value::sum_of(positions.is_empty(), unreal)},
        "portfolio": portfolio,
        "markets": crate::markets::markets_view(base, &positions),
        "cashflow": cashflow,
        "unmatched": base.book.fifo.unmatched,
        "accounts": *base.accounts,
        "activityCount": base.activity_count,
    })
}

/// `DETAIL_KEYS`: what a row carries only when the page has opened it.
pub const DETAIL_KEYS: [&str; 2] = ["legs", "fills"];

/// `slim`: the view as the page receives it -- the legs and fills of one
/// trade or holding only, because sending every leg of every trade on every
/// poll is most of the payload.
pub fn slim(view: &Value, detail: Option<&str>) -> Value {
    let mut out = view.clone();
    for key in ["trades", "positions"] {
        let rows: Vec<Value> = view
            .get(key)
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .map(|r| {
                        let keep = detail.map(|d| field_s(r, "id") == d).unwrap_or(false);
                        if keep {
                            r.clone()
                        } else {
                            // rebuilt rather than removed from: serde_json's
                            // `Map::remove` under `preserve_order` swaps the
                            // last entry into the hole, and the row would
                            // reach the page with its keys shuffled
                            let mut kept = Map::new();
                            if let Some(m) = r.as_object() {
                                for (k, v) in m {
                                    if !DETAIL_KEYS.contains(&k.as_str()) {
                                        kept.insert(k.clone(), v.clone());
                                    }
                                }
                            }
                            Value::Object(kept)
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        if let Value::Object(m) = &mut out {
            m.insert(key.into(), Value::Array(rows));
        }
    }
    out
}

/// `trade_detail`: the legs and fills of one trade or holding, by id.
pub fn trade_detail(base: &Base, trade_id: &str) -> Option<Value> {
    for rows in [&base.trades, &base.positions] {
        for r in rows.iter() {
            if field_s(r, "id") == trade_id {
                return Some(json!({
                    "id": trade_id,
                    "legs": r.get("legs").cloned().unwrap_or_else(|| json!([])),
                    "fills": r.get("fills").cloned().unwrap_or_else(|| json!([])),
                }));
            }
        }
    }
    None
}
