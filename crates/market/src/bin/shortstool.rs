//! The short-selling readers, on fixtures or live.
use serde_json::{json, Map, Value};
use std::io::Read;

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(|x| x.as_str()).unwrap_or("")
}

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_default();
    let mut buf = String::new();
    std::io::stdin().read_to_string(&mut buf).unwrap();
    let doc: Value = serde_json::from_str(&buf).unwrap();
    use bagholder_market::shorts as sh;
    let arr = |k: &str| doc.get(k).and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let conn = rusqlite::Connection::open(s(&doc, "db")).unwrap();

    if mode == "live" {
        let out: Vec<Value> = arr("listings").iter().map(|l| {
            let f = |i: usize| l.get(i).and_then(|v| v.as_str()).unwrap_or("").to_string();
            sh::for_listing(&conn, &f(0), &f(1), &f(2), s(&doc, "today"), l.get(3).and_then(|v| v.as_bool()).unwrap_or(false), &f(4))
        }).collect();
        println!("{}", serde_json::to_string(&out).unwrap());
        return;
    }

    let dates: Vec<Value> = arr("dates").iter().map(|d| {
        let d = d.as_str().unwrap_or("");
        json!({
            "positions": sh::position_dates(d, 6),
            "series": sh::position_dates(d, 8),
            "periods": sh::volume_periods(d, 6).into_iter().map(|(a, b)| json!([a, b])).collect::<Vec<_>>(),
            "trading": sh::trading_days(d, 6),
        })
    }).collect();
    let floats: Vec<Value> = arr("floats").iter().map(|v| json!(sh::num(Some(v)).map(|x| if x.is_nan() { "nan".to_string() } else { format!("{:?}", x) }))).collect();
    let us_volume: Vec<Value> = arr("us_volume").iter().map(|t| Value::Object(sh::parse_us_volume(t.as_str().unwrap_or("")))).collect();
    let ca_volume: Vec<Value> = arr("ca_volume").iter().map(|t| match sh::parse_ca_volume(t.as_str().unwrap_or("")) {
        Ok(m) => Value::Object(m),
        Err(_) => json!("raised"),
    }).collect();
    let ca_positions: Vec<Value> = arr("ca_positions").iter().map(|p| {
        let raw = std::fs::read(p.as_str().unwrap_or("")).unwrap_or_default();
        match bagholder_market::xls::table(&raw) {
            Ok(g) => Value::Object(sh::parse_ca_positions(&g)),
            Err(_) => Value::Null,
        }
    }).collect();
    let us_position: Vec<Value> = arr("us_position").iter().map(sh::parse_us_position).collect();
    let fits: Vec<Value> = arr("fits").iter().map(|r| json!(sh::venue_fits(r[0].as_str().unwrap_or(""), r[1].as_str().unwrap_or("")))).collect();
    let markets: Vec<Value> = arr("markets").iter().map(|r| {
        json!(sh::market_of(r[0].as_str().unwrap_or(""), r[1].as_str().unwrap_or(""), r[2].as_str().unwrap_or("")))
    }).collect();
    let funds: Vec<Value> = arr("funds").iter().map(|n| json!(bagholder_model::exposure::is_fund(n.as_str().unwrap_or("")))).collect();

    // one listing from fixed answers: the regulators' parts, the series and
    // the float are given, the rest is this crate's
    let listings: Vec<Value> = arr("finish").iter().map(|c| {
        let today = s(c, "today");
        let sym = s(c, "symbol").trim().to_uppercase();
        let ex = s(c, "exchange");
        let where_ = sh::market_of(&sym, ex, s(c, "currency"));
        if where_.is_empty() {
            return json!({});
        }
        let files = c.get("files").cloned().unwrap_or(json!({}));
        let rows = |k: &str| files.get(k).and_then(|f| f.get("rows")).and_then(|r| r.as_object()).cloned().unwrap_or_default();
        let key = |k: &str| files.get(k).and_then(|f| f.get("key")).and_then(|r| r.as_str()).unwrap_or("").to_string();
        let mut rec: Map<String, Value> = Map::new();
        let parts = if where_ == "us" {
            vec![sh::parse_us_position(c.get("finra").unwrap_or(&Value::Null)), sh::us_volume_from(&key("us_volume"), &rows("us_volume"), &sym)]
        } else {
            let vol_rows = rows("ca_volume");
            let vol = if key("ca_volume").is_empty() {
                json!({})
            } else {
                sh::ca_volume_from(&key("ca_volume"), vol_rows.get(&sym), ex, || c.get("traded").and_then(|t| t.as_f64()))
            };
            vec![sh::ca_position_from(&key("ca_position"), &rows("ca_position"), &sym, ex, today), vol]
        };
        for p in parts {
            if let Value::Object(m) = p {
                for (k, v) in m {
                    rec.insert(k, v);
                }
            }
        }
        let series = c.get("series").and_then(|v| v.as_array()).cloned().unwrap_or_default();
        let float = c.get("float").and_then(|v| v.as_f64());
        sh::finish(&conn, rec, &sym, ex, s(c, "currency"), today, c.get("trend").and_then(|v| v.as_bool()).unwrap_or(false),
                   s(c, "name"), where_, |_| series, |_| float)
    }).collect();

    println!("{}", serde_json::to_string(&json!({
        "dates": dates, "floats": floats, "us_volume": us_volume, "ca_volume": ca_volume,
        "ca_positions": ca_positions, "us_position": us_position, "fits": fits, "markets": markets,
        "funds": funds, "finish": listings,
    })).unwrap());
}
