//! Prints every operation's query text as JSON, so it can be compared with
//! `bagholder.QUERIES` character for character.
fn main() {
    let mut m = serde_json::Map::new();
    for (k, q) in bagholder_ws::queries::QUERIES {
        m.insert(k.to_string(), serde_json::Value::String(q.to_string()));
    }
    println!("{}", serde_json::to_string(&serde_json::Value::Object(m)).unwrap());
}
