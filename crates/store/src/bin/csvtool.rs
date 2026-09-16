//! The CSV importer, on fixtures, imports and folder scans.
use serde_json::{json, Value};
use std::io::Read;

fn strip_ids(v: &mut Value) {
    if let Some(a) = v.get_mut("activities").and_then(|x| x.as_array_mut()) {
        for r in a {
            if let Some(m) = r.as_object_mut() {
                m.insert("id".into(), json!(""));
            }
        }
    }
}

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_default();
    let mut buf = String::new();
    std::io::stdin().read_to_string(&mut buf).unwrap();
    let doc: Value = serde_json::from_str(&buf).unwrap();
    use bagholder_store::csvimport as c;
    let arr = |k: &str| doc.get(k).and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let st = |v: &Value| v.as_str().unwrap_or("").to_string();

    if mode == "import" || mode == "scan" {
        let conn = rusqlite::Connection::open(st(&doc["db"])).unwrap();
        bagholder_store::schema::init_schema(&conn).unwrap();
        let out: Vec<Value> = if mode == "import" {
            arr("files").iter().map(|f| match c::import_text(&conn, &st(&f[0]), &st(&f[1])) {
                Ok(r) => r,
                Err(_) => json!("raised"),
            }).collect()
        } else {
            arr("steps").iter().map(|s| {
                match st(&s["op"]).as_str() {
                    "set" => c::set_watch_folder(&conn, &st(&s["path"])).unwrap(),
                    "scan" => c::scan_folder(&conn, s.get("folder").and_then(|v| v.as_str()), s["force"].as_bool().unwrap_or(false)).unwrap_or(json!("raised")),
                    "status" => c::status(&conn).unwrap(),
                    "clear" => { c::clear_watch_folder(&conn).unwrap(); c::status(&conn).unwrap() }
                    "write" => { std::fs::write(st(&s["path"]), st(&s["text"])).unwrap(); json!(null) }
                    "remove" => { let _ = std::fs::remove_file(st(&s["path"])); json!(null) }
                    _ => json!(null),
                }
            }).collect()
        };
        println!("{}", serde_json::to_string(&out).unwrap());
        return;
    }

    let numbers: Vec<Value> = arr("numbers").iter().map(|v| json!(c::parse_number(&st(v)))).collect();
    let dates: Vec<Value> = arr("dates").iter().map(|v| json!(c::parse_date(&st(v)))).collect();
    let headers: Vec<Value> = arr("headers").iter().map(|v| json!(c::normalize_header(&st(v)))).collect();
    let formats: Vec<Value> = arr("formats").iter().map(|v| {
        let h: Vec<String> = v.as_array().cloned().unwrap_or_default().iter().map(st).collect();
        json!(c::detect_format(&h))
    }).collect();
    let cats: Vec<Value> = arr("categories").iter().map(|v| json!(c::categorize(&st(&v[0]), &st(&v[1])))).collect();
    let descs: Vec<Value> = arr("descriptions").iter().map(|v| {
        let (s, n) = c::extract_instrument(&st(v));
        json!({"instrument": [s, n], "parsed": c::parse_statement_description(&st(v)).to_json()})
    }).collect();
    let types: Vec<Value> = arr("types").iter().map(|v| {
        let (a, b, k) = c::map_statement_type(&st(&v[0]), &st(&v[1]));
        json!([a, b, k])
    }).collect();
    let books: Vec<Value> = arr("books").iter().map(|v| json!(c::book_id_from_file_name(&st(v)))).collect();
    let footers: Vec<Value> = arr("footers").iter().map(|v| json!(c::is_footer_line(&st(v)))).collect();
    let csvs: Vec<Value> = arr("csvs").iter().map(|v| match c::parse_csv(&st(&v[1]), &st(&v[0])) {
        Ok(mut r) => { strip_ids(&mut r); r }
        Err(_) => json!("raised"),
    }).collect();
    println!("{}", serde_json::to_string(&json!({
        "numbers": numbers, "dates": dates, "headers": headers, "formats": formats, "categories": cats,
        "descriptions": descs, "types": types, "books": books, "footers": footers, "csvs": csvs,
    })).unwrap());
}
