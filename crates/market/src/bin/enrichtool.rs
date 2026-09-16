//! The document readers: forms, subjects, text and the reading of a model's
//! answers, on fixtures.
use serde_json::{json, Value};
use std::io::Read;

fn st(v: &Value) -> String {
    v.as_str().unwrap_or("").to_string()
}

fn main() {
    let mut buf = String::new();
    std::io::stdin().read_to_string(&mut buf).unwrap();
    let doc: Value = serde_json::from_str(&buf).unwrap();
    use bagholder_market::{enrich as e, forms as f};
    let arr = |k: &str| doc.get(k).and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let hex = |s: &str| -> Vec<u8> { (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect() };

    let texts: Vec<Value> = arr("texts").iter().map(|t| json!({"is_form": f::is_form(&st(t)), "read": f::read(&st(t))})).collect();
    let subjects: Vec<Value> = arr("pdf_hex").iter().map(|h| json!(e::extract_pdf_subject(&hex(&st(h))))).collect();
    let files: Vec<Value> = arr("files").iter().map(|x| {
        let data = std::fs::read(st(&x["path"])).unwrap_or_default();
        let is_pdf = data.starts_with(b"%PDF-");
        json!({
            "subject": e::extract_pdf_subject(&data),
            "html_text": if is_pdf { Value::Null } else { json!(e::html_text(&data)) },
        })
    }).collect();
    let readable: Vec<Value> = arr("readable").iter().map(|t| json!(e::readable(&st(t)))).collect();
    let answers: Vec<Value> = arr("answers").iter().map(|t| json!({
        "first_sentence": e::first_sentence(&st(t)),
        "strip_preamble": e::strip_preamble(&st(t)),
        "summary": e::summary_from(&st(t)),
        "title": e::title_from(&st(t)),
        "hedged": e::hedged(&st(t)),
        "junk": e::is_junk_title(&st(t)),
    })).collect();
    let htmls: Vec<Value> = arr("htmls").iter().map(|t| json!(e::html_text(st(t).as_bytes()))).collect();
    println!("{}", serde_json::to_string(&json!({
        "texts": texts, "subjects": subjects, "files": files, "readable": readable, "answers": answers, "htmls": htmls,
    })).unwrap());
}
