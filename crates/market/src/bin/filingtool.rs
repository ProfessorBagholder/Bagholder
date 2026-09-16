//! The filing sources' parsers on captured pages and answers, and the whole
//! pipeline live.
use serde_json::{json, Value};
use std::io::Read;

fn st(v: &Value) -> String {
    v.as_str().unwrap_or("").to_string()
}

fn pairs(v: Vec<(String, String)>) -> Value {
    json!(v.into_iter().map(|(a, b)| json!([a, b])).collect::<Vec<_>>())
}

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_default();
    let mut buf = String::new();
    std::io::stdin().read_to_string(&mut buf).unwrap();
    let doc: Value = serde_json::from_str(&buf).unwrap();
    use bagholder_market::{disclosures as d, edgar as e, sedar as s};
    let arr = |k: &str| doc.get(k).and_then(|v| v.as_array()).cloned().unwrap_or_default();

    if mode == "live" {
        let out: Vec<Value> = arr("listings").iter().map(|l| {
            d::fetch(&st(&l[0]), &st(&l[1]), &st(&l[2]), &st(&l[3]), 200, &st(&l[4]))
        }).collect();
        let docs: Vec<Value> = arr("documents").iter().map(|r| match d::document(r) {
            Ok((bytes, ct)) => json!({"len": bytes.len(), "sha1": openssl::sha::sha1(&bytes).iter().map(|b| format!("{:02x}", b)).collect::<String>(), "ct": ct}),
            Err(err) => json!({"error": err.to_string()}),
        }).collect();
        println!("{}", serde_json::to_string(&json!({"fetch": out, "documents": docs})).unwrap());
        return;
    }

    let pages: Vec<Value> = arr("pages").iter().map(|p| {
        let html = st(&p["text"]);
        let names: Vec<Value> = arr("menu_names").iter().map(|n| {
            json!(s::issuer_menu_node(&html, n.as_str()))
        }).collect();
        json!({
            "form_fields": pairs(s::form_fields(&html)),
            "vi_params": pairs(s::vi_params(&html)),
            "search_action": s::search_action(&html).map(|(a, b, c)| json!([a, b, c])),
            "issuer_menu_node": names,
            "docs_menu_node": s::docs_menu_node(&html),
            "filings": s::parse_filings(&html),
            "issuers": s::parse_reporting_issuers(&html),
            "text": s::text(&html.chars().take(5000).collect::<String>()),
        })
    }).collect();
    let isos: Vec<Value> = arr("isos").iter().map(|v| json!(s::iso(&st(v)))).collect();
    let splits: Vec<Value> = arr("files").iter().map(|v| {
        let (a, b) = s::split_type_title(&st(v));
        json!({"split": [a, b], "category": s::category(&st(v))})
    }).collect();
    let items: Vec<Value> = arr("raw_filings").iter().map(|r| s::to_item(&r[0], &st(&r[1]))).collect();
    let ranks: Vec<Value> = arr("ranks").iter().map(|r| {
        let mut rows = r[0].as_array().cloned().unwrap_or_default();
        json!(s::rank(&mut rows, &st(&r[1])))
    }).collect();
    let covers: Vec<Value> = arr("covers").iter().map(|r| json!([s::covers(&st(&r[0]), &st(&r[1]), &st(&r[2]))])).collect();
    let encoded: Vec<Value> = arr("encode").iter().map(|r| {
        let v: Vec<(String, String)> = r.as_array().cloned().unwrap_or_default().iter().map(|p| (st(&p[0]), st(&p[1]))).collect();
        json!(s::urlencode(&v))
    }).collect();

    let subs: Vec<Value> = arr("subs").iter().map(|x| match e::parse_submissions(&x["sub"], x["cik"].as_i64().unwrap_or(0), 200) {
        Ok(v) => json!(v),
        Err(err) => json!({"error": err.to_string()}),
    }).collect();
    let forms: Vec<Value> = arr("forms").iter().map(|f| json!([e::category(&st(&f[0])), e::title(&st(&f[0]), &st(&f[1]))])).collect();
    let bares: Vec<Value> = arr("bares").iter().map(|v| json!(e::bare(&st(v)))).collect();
    let picks: Vec<Value> = arr("indexes").iter().map(|x| {
        let items = x["listing"]["directory"]["item"].as_array().cloned().unwrap_or_default();
        json!(e::pick_content(&items, &st(&x["primary"])))
    }).collect();
    let thirteen: Vec<Value> = arr("thirteen").iter().map(|x| json!(e::enrichment_from_xml(&st(&x["type"]).to_uppercase(), &st(&x["xml"])))).collect();
    let names: Vec<Value> = arr("names").iter().map(|r| json!(d::names_match(&st(&r[0]), &st(&r[1])))).collect();
    let cleans: Vec<Value> = arr("cleans").iter().map(|v| json!(d::clean(&st(v)))).collect();
    let ids: Vec<Value> = arr("ids").iter().map(|r| json!(s::filing_id(&st(&r[0]), &st(&r[1]), &st(&r[2]), &st(&r[3])))).collect();

    println!("{}", serde_json::to_string(&json!({
        "pages": pages, "isos": isos, "files": splits, "items": items, "ranks": ranks, "covers": covers, "encode": encoded,
        "subs": subs, "forms": forms, "bares": bares, "picks": picks, "thirteen": thirteen, "names": names, "cleans": cleans, "ids": ids,
    })).unwrap());
}
