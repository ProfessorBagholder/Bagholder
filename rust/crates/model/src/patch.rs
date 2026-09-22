//! What moved between two states of the page's data: the entities and the fields,
//! and nothing else (docs/architecture.md, rule 0).
//!
//! The page holds each entity -- a position, a trade, a tile, a headline -- as one
//! object and is sent, for a change, only the fields of only the entities that
//! differ, so it can write them into the objects it already shows and the one
//! element bound to each is all that updates. A price moving on one holding is
//! that holding's four figures and the totals that include it; it is never "the
//! positions".
//!
//! A list of rows is matched by the rows' own identity, not by position, so a row
//! inserted at the top is one insertion and not a rewrite of every row beneath
//! it. The identity is whichever of the usual fields every row has and no two
//! share; a list with none (strings, or rows that repeat) is small and is sent
//! whole when it differs.
//!
//! The operations, each a JSON array:
//!
//! - `["set", path, value]` -- the value at `path` is now `value`.
//! - `["del", path]` -- the object at the parent of `path` no longer has that key.
//! - `["rows", path, key, order, added]` -- the list at `path`, whose rows are
//!   told apart by their `key` field, now holds the rows named in `order`, in that
//!   order; `added` carries, by key, the rows the page has not seen. A row it has
//!   and `order` does not name is gone.
//!
//! A path is a list of steps from the root: a string names an object's field;
//! `{"k": key, "v": value}` names the row of a list whose `key` field is `value`.

use serde_json::{json, Map, Value};

/// The fields a row may be told apart by, in the order they are tried.
const KEYS: [&str; 9] = ["id", "key", "d", "year", "symbol", "grade", "label", "date", "name"];

fn key_text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// The field that tells this list's rows apart: every row has it and no two
/// share a value. `None` for a list that is not rows, or whose rows repeat.
pub fn row_key(rows: &[Value]) -> Option<&'static str> {
    if rows.is_empty() || !rows.iter().all(|r| r.is_object()) {
        return None;
    }
    KEYS.iter().copied().find(|k| {
        let mut seen = std::collections::HashSet::new();
        rows.iter().all(|r| r.get(*k).and_then(key_text).map_or(false, |t| seen.insert(t)))
    })
}

fn step(key: &str, value: &str) -> Value {
    json!({"k": key, "v": value})
}

fn with(path: &[Value], s: Value) -> Vec<Value> {
    let mut p = path.to_vec();
    p.push(s);
    p
}

fn diff_into(old: &Value, new: &Value, path: &[Value], ops: &mut Vec<Value>) {
    if old == new {
        return;
    }
    match (old, new) {
        (Value::Object(a), Value::Object(b)) => {
            for (k, v) in b {
                match a.get(k) {
                    Some(was) => diff_into(was, v, &with(path, json!(k)), ops),
                    None => ops.push(json!(["set", with(path, json!(k)), v])),
                }
            }
            for k in a.keys().filter(|k| !b.contains_key(*k)) {
                ops.push(json!(["del", with(path, json!(k))]));
            }
        }
        (Value::Array(a), Value::Array(b)) => {
            // the same identity must hold on both sides, or a row could be matched to a stranger
            let key = match (row_key(a), row_key(b)) {
                (Some(x), Some(y)) if x == y => x,
                (None, Some(y)) if a.is_empty() => y,
                _ => {
                    ops.push(json!(["set", path, new]));
                    return;
                }
            };
            let id = |r: &Value| key_text(&r[key]).unwrap_or_default();
            let was: std::collections::HashMap<String, &Value> = a.iter().map(|r| (id(r), r)).collect();
            let order: Vec<String> = b.iter().map(id).collect();
            if a.iter().map(id).collect::<Vec<_>>() != order {
                let added: Map<String, Value> = b.iter().filter(|r| !was.contains_key(&id(r))).map(|r| (id(r), r.clone())).collect();
                ops.push(json!(["rows", path, key, order, added]));
            }
            for r in b {
                if let Some(before) = was.get(&id(r)) {
                    diff_into(before, r, &with(path, step(key, &id(r))), ops);
                }
            }
        }
        _ => ops.push(json!(["set", path, new])),
    }
}

/// The operations that turn `old` into `new`. Empty when they are the same.
pub fn diff(old: &Value, new: &Value) -> Vec<Value> {
    let mut ops = Vec::new();
    diff_into(old, new, &[], &mut ops);
    ops
}

/// As `diff`, for a part of the state that lives under `prefix`.
pub fn diff_under(prefix: &[&str], old: &Value, new: &Value) -> Vec<Value> {
    let path: Vec<Value> = prefix.iter().map(|s| json!(s)).collect();
    let mut ops = Vec::new();
    diff_into(old, new, &path, &mut ops);
    ops
}

// --- the typed differ ----------------------------------------------------------
//
// The same operations, found by comparing the model's own values rather than their
// JSON: a struct field by field under the names serde gives its fields, a list of
// rows by the id its row type declares (`#[diff(key = …)]`) rather than one guessed
// from the data, and a value shared between the two states (an `Arc` both hold) not
// at all. `#[derive(Diff)]` (the `bagholder-diff-derive` crate) writes the struct
// and enum impls; the leaves, lists, maps and pointers are here.

/// A value the page holds, compared with its next state.
pub trait Diff: serde::Serialize {
    /// For a row type: the field (by its JSON name) that tells its rows apart.
    const KEY: Option<&'static str> = None;
    /// This row's value of that field, as the page's path step names it.
    fn row_key(&self) -> Option<String> {
        None
    }
    /// Push onto `ops` what turns `self` into `new`, under `path`.
    fn diff(&self, new: &Self, path: &mut Vec<Value>, ops: &mut Vec<Value>);
}

/// The operations that turn `old` into `new`.
pub fn typed<T: Diff + ?Sized>(old: &T, new: &T) -> Vec<Value> {
    typed_under(&[], old, new)
}

/// As `typed`, for a part of the state that lives under `prefix`.
pub fn typed_under<T: Diff + ?Sized>(prefix: &[&str], old: &T, new: &T) -> Vec<Value> {
    let mut path: Vec<Value> = prefix.iter().map(|s| json!(s)).collect();
    let mut ops = Vec::new();
    old.diff(new, &mut path, &mut ops);
    ops
}

fn to_json<T: serde::Serialize + ?Sized>(v: &T) -> Value {
    serde_json::to_value(v).expect("a wire value is plain data")
}

fn set<T: serde::Serialize + ?Sized>(path: &[Value], v: &T, ops: &mut Vec<Value>) {
    ops.push(json!(["set", path, to_json(v)]));
}

/// A value compared as its JSON: the same, or set.
pub fn leaf<T: serde::Serialize + ?Sized>(old: &T, new: &T, path: &mut Vec<Value>, ops: &mut Vec<Value>) {
    let n = to_json(new);
    if to_json(old) != n {
        ops.push(json!(["set", path, n]));
    }
}

/// A value compared as the JSON it is written as, the way `diff` compares JSON.
pub fn as_json<T: serde::Serialize + ?Sized>(old: &T, new: &T, path: &mut Vec<Value>, ops: &mut Vec<Value>) {
    diff_into(&to_json(old), &to_json(new), path, ops)
}

/// The id a row written as JSON carries in `field`.
pub fn json_key_of<T: serde::Serialize + ?Sized>(v: &T, field: &str) -> Option<String> {
    to_json(v).get(field).and_then(key_text)
}

/// One field of a struct, always present.
pub fn field<T: Diff + ?Sized>(old: &T, new: &T, name: &str, path: &mut Vec<Value>, ops: &mut Vec<Value>) {
    path.push(json!(name));
    old.diff(new, path, ops);
    path.pop();
}

/// One field serde leaves out when it is empty: set when it appears, deleted when it goes.
pub fn field_present<T: Diff + ?Sized>(old: &T, new: &T, was: bool, is: bool, name: &str, path: &mut Vec<Value>, ops: &mut Vec<Value>) {
    path.push(json!(name));
    match (was, is) {
        (true, true) => old.diff(new, path, ops),
        (false, true) => set(path, new, ops),
        (true, false) => ops.push(json!(["del", path])),
        (false, false) => {}
    }
    path.pop();
}

/// A row's id as a path step names it: its text, or its number written out.
pub fn key_of<T: serde::Serialize + ?Sized>(v: &T) -> Option<String> {
    key_text(&to_json(v))
}

macro_rules! leaves {
    ($($t:ty),*) => {$(
        impl Diff for $t {
            fn diff(&self, new: &Self, path: &mut Vec<Value>, ops: &mut Vec<Value>) {
                if self != new {
                    set(path, new, ops);
                }
            }
        }
    )*};
}
leaves!(String, str, bool, i32, i64, u32, u64, usize);

impl Diff for f64 {
    fn diff(&self, new: &Self, path: &mut Vec<Value>, ops: &mut Vec<Value>) {
        // NaN is written as null, and null is null
        if self != new && !(self.is_nan() && new.is_nan()) {
            set(path, new, ops);
        }
    }
}

impl<T: Diff + ?Sized> Diff for &T {
    fn diff(&self, new: &Self, path: &mut Vec<Value>, ops: &mut Vec<Value>) {
        (**self).diff(*new, path, ops)
    }
}

impl<T: Diff + ?Sized> Diff for std::sync::Arc<T> {
    fn diff(&self, new: &Self, path: &mut Vec<Value>, ops: &mut Vec<Value>) {
        // the one value both states hold has not moved
        if !std::sync::Arc::ptr_eq(self, new) {
            (**self).diff(new, path, ops)
        }
    }
}

impl<T: Diff> Diff for Option<T> {
    fn diff(&self, new: &Self, path: &mut Vec<Value>, ops: &mut Vec<Value>) {
        match (self, new) {
            (Some(a), Some(b)) => a.diff(b, path, ops),
            (None, None) => {}
            _ => set(path, new, ops),
        }
    }
}

impl<A: serde::Serialize, B: serde::Serialize> Diff for (A, B) {
    fn diff(&self, new: &Self, path: &mut Vec<Value>, ops: &mut Vec<Value>) {
        leaf(self, new, path, ops)
    }
}

/// Whether `old` and `new` are the same, as far as the page could tell.
fn same<T: Diff + ?Sized>(old: &T, new: &T) -> bool {
    let mut ops = Vec::new();
    old.diff(new, &mut Vec::new(), &mut ops);
    ops.is_empty()
}

/// Each row's id, when every row has one and no two share it.
fn ids<T: Diff>(rows: &[T]) -> Option<Vec<String>> {
    let mut seen = std::collections::HashSet::new();
    let ids: Vec<String> = rows.iter().map(|r| r.row_key()).collect::<Option<_>>()?;
    ids.iter().all(|k| seen.insert(k.as_str())).then_some(ids)
}

impl<T: Diff> Diff for Vec<T> {
    fn diff(&self, new: &Self, path: &mut Vec<Value>, ops: &mut Vec<Value>) {
        self.as_slice().diff(new.as_slice(), path, ops)
    }
}

// serde writes arrays of a fixed length only up to 32, each length its own impl
macro_rules! arrays {
    ($($n:literal),*) => {$(
        impl<T: Diff> Diff for [T; $n] {
            fn diff(&self, new: &Self, path: &mut Vec<Value>, ops: &mut Vec<Value>) {
                self.as_slice().diff(new.as_slice(), path, ops)
            }
        }
    )*};
}
arrays!(1, 2, 3, 4, 5, 6, 7, 8);

impl<T: Diff> Diff for [T] {
    fn diff(&self, new: &Self, path: &mut Vec<Value>, ops: &mut Vec<Value>) {
        if self.is_empty() && new.is_empty() {
            return;
        }
        // a list gone empty, or of rows no id tells apart, is sent whole when it differs
        let keyed = match (T::KEY, new.is_empty()) {
            (Some(key), false) => match (ids(self), ids(new)) {
                (Some(was), Some(now)) => Some((key, was, now)),
                _ => None,
            },
            _ => None,
        };
        let Some((key, was, now)) = keyed else {
            if self.len() != new.len() || self.iter().zip(new).any(|(a, b)| !same(a, b)) {
                set(path, new, ops);
            }
            return;
        };
        let at: std::collections::HashMap<&str, usize> = was.iter().enumerate().map(|(i, k)| (k.as_str(), i)).collect();
        if was != now {
            let added: Map<String, Value> = new.iter().zip(&now).filter(|(_, k)| !at.contains_key(k.as_str())).map(|(r, k)| (k.clone(), to_json(r))).collect();
            ops.push(json!(["rows", path, key, now, added]));
        }
        for (row, k) in new.iter().zip(&now) {
            if let Some(&i) = at.get(k.as_str()) {
                path.push(step(key, k));
                self[i].diff(row, path, ops);
                path.pop();
            }
        }
    }
}

/// An object keyed by name: each value compared under its key, a key gained set,
/// a key lost deleted.
fn keyed<'a, V: Diff + 'a>(
    old: impl Fn(&str) -> Option<&'a V>,
    new: impl Iterator<Item = (&'a String, &'a V)>,
    gone: impl Iterator<Item = &'a String>,
    path: &mut Vec<Value>,
    ops: &mut Vec<Value>,
) {
    for (k, v) in new {
        path.push(json!(k));
        match old(k) {
            Some(was) => was.diff(v, path, ops),
            None => set(path, v, ops),
        }
        path.pop();
    }
    for k in gone {
        path.push(json!(k));
        ops.push(json!(["del", path]));
        path.pop();
    }
}

impl<V: Diff> Diff for std::collections::BTreeMap<String, V> {
    fn diff(&self, new: &Self, path: &mut Vec<Value>, ops: &mut Vec<Value>) {
        keyed(|k| self.get(k), new.iter(), self.keys().filter(|k| !new.contains_key(*k)), path, ops)
    }
}

impl<V: Diff> Diff for std::collections::HashMap<String, V> {
    fn diff(&self, new: &Self, path: &mut Vec<Value>, ops: &mut Vec<Value>) {
        keyed(|k| self.get(k), new.iter(), self.keys().filter(|k| !new.contains_key(*k)), path, ops)
    }
}

impl<V: Diff> Diff for crate::wire::Ordered<V> {
    fn diff(&self, new: &Self, path: &mut Vec<Value>, ops: &mut Vec<Value>) {
        let find = |m: &'_ crate::wire::Ordered<V>, k: &str| m.0.iter().position(|(key, _)| key == k);
        keyed(|k| find(self, k).map(|i| &self.0[i].1), new.0.iter().map(|(k, v)| (k, v)), self.0.iter().map(|(k, _)| k).filter(|k| find(new, k).is_none()), path, ops)
    }
}

impl Diff for Value {
    fn diff(&self, new: &Self, path: &mut Vec<Value>, ops: &mut Vec<Value>) {
        diff_into(self, new, path, ops)
    }
}

fn walk<'a>(root: &'a mut Value, path: &[Value]) -> Option<&'a mut Value> {
    let mut at = root;
    for s in path {
        at = match s {
            Value::String(k) => at.get_mut(k.as_str())?,
            Value::Object(o) => {
                let (k, v) = (o.get("k")?.as_str()?, o.get("v")?.as_str()?);
                at.as_array_mut()?.iter_mut().find(|r| r.get(k).and_then(key_text).as_deref() == Some(v))?
            }
            _ => return None,
        };
    }
    Some(at)
}

/// Apply operations to `target`, as the page does. For the tests: a patch applied
/// to the old state must give the new one.
pub fn apply(target: &mut Value, ops: &[Value]) {
    for op in ops {
        let path = op[1].as_array().cloned().unwrap_or_default();
        match op[0].as_str() {
            Some("set") if path.is_empty() => *target = op[2].clone(),
            Some("set") | Some("del") => {
                let (last, parent) = path.split_last().unwrap();
                let Some(parent) = walk(target, parent) else { continue };
                match (last, op[0].as_str()) {
                    (Value::String(k), Some("set")) => parent[k.as_str()] = op[2].clone(),
                    (Value::String(k), _) => {
                        parent.as_object_mut().map(|o| o.shift_remove(k.as_str()));
                    }
                    (Value::Object(_), Some("set")) => {
                        if let Some(row) = walk(target, &path) {
                            *row = op[2].clone();
                        }
                    }
                    _ => {}
                }
            }
            Some("rows") => {
                let key = op[2].as_str().unwrap_or("id").to_string();
                let Some(list) = walk(target, &path).and_then(|v| v.as_array_mut()) else { continue };
                let mut have: std::collections::HashMap<String, Value> =
                    list.drain(..).map(|r| (key_text(&r[key.as_str()]).unwrap_or_default(), r)).collect();
                for id in op[3].as_array().into_iter().flatten().filter_map(|v| v.as_str()) {
                    if let Some(row) = have.remove(id).or_else(|| op[4].get(id).cloned()) {
                        list.push(row);
                    }
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn book() -> Value {
        json!({
            "kpi": {"realized": 1200.5, "count": 3},
            "positions": [
                {"id": "p:QNC", "symbol": "QNC", "last": 1.75, "mv": 175.0, "unreal": 25.0, "tags": ["core"]},
                {"id": "p:CH", "symbol": "CH", "last": 0.20, "mv": 400.0, "unreal": -40.0, "tags": []},
            ],
            "positionsSummary": {"count": 2, "mv": 575.0, "unreal": -15.0},
            "trades": [{"id": "t1", "symbol": "AAA", "grade": ""}],
            "markets": {"news": [{"id": "n1", "headline": "one"}, {"id": "n2", "headline": "two"}]},
        })
    }

    fn turns(old: &Value, new: &Value) -> Vec<Value> {
        let ops = diff(old, new);
        let mut got = old.clone();
        apply(&mut got, &ops);
        assert_eq!(&got, new, "the patch applied to the old state gives the new one");
        ops
    }

    #[test]
    fn test_nothing_moved_is_nothing_sent() {
        assert!(diff(&book(), &book()).is_empty());
    }

    #[test]
    fn test_one_price_moving_is_that_holdings_figures_and_the_totals_and_nothing_else() {
        let (old, mut new) = (book(), book());
        new["positions"][0]["last"] = json!(1.80);
        new["positions"][0]["mv"] = json!(180.0);
        new["positions"][0]["unreal"] = json!(30.0);
        new["positionsSummary"]["mv"] = json!(580.0);
        new["positionsSummary"]["unreal"] = json!(-10.0);
        let ops = turns(&old, &new);
        let row = json!({"k": "id", "v": "p:QNC"});
        assert_eq!(ops, vec![
            json!(["set", ["positions", row, "last"], 1.80]),
            json!(["set", ["positions", row, "mv"], 180.0]),
            json!(["set", ["positions", row, "unreal"], 30.0]),
            json!(["set", ["positionsSummary", "mv"], 580.0]),
            json!(["set", ["positionsSummary", "unreal"], -10.0]),
        ]);
        let text = serde_json::to_string(&ops).unwrap();
        assert!(!text.contains("CH") && !text.contains("AAA") && !text.contains("headline"), "the other holding, the trades and the news are not mentioned");
    }

    #[test]
    fn test_a_grade_set_is_one_field_of_one_trade() {
        let (old, mut new) = (book(), book());
        new["trades"][0]["grade"] = json!("B");
        assert_eq!(turns(&old, &new), vec![json!(["set", ["trades", {"k": "id", "v": "t1"}, "grade"], "B"])]);
    }

    #[test]
    fn test_a_headline_arriving_is_one_row_inserted_not_the_list_again() {
        let (old, mut new) = (book(), book());
        new["markets"]["news"].as_array_mut().unwrap().insert(0, json!({"id": "n3", "headline": "three"}));
        let ops = turns(&old, &new);
        assert_eq!(ops, vec![json!(["rows", ["markets", "news"], "id", ["n3", "n1", "n2"], {"n3": {"id": "n3", "headline": "three"}}])]);
    }

    #[test]
    fn test_a_row_leaving_and_the_rest_reordering() {
        let (old, mut new) = (book(), book());
        new["positions"].as_array_mut().unwrap().remove(0);
        assert_eq!(turns(&old, &new), vec![json!(["rows", ["positions"], "id", ["p:CH"], {}])]);
        let mut swapped = book();
        swapped["markets"]["news"].as_array_mut().unwrap().reverse();
        assert_eq!(turns(&old, &swapped), vec![json!(["rows", ["markets", "news"], "id", ["n2", "n1"], {}])]);
    }

    #[test]
    fn test_a_field_gained_and_a_field_lost() {
        let (old, mut new) = (book(), book());
        new["kpi"]["winRate"] = json!(0.5);
        new["kpi"].as_object_mut().unwrap().remove("count");
        assert_eq!(turns(&old, &new), vec![json!(["set", ["kpi", "winRate"], 0.5]), json!(["del", ["kpi", "count"]])]);
    }

    #[test]
    fn test_a_list_that_is_not_rows_is_sent_whole_and_rows_that_repeat_are_too() {
        let (old, mut new) = (book(), book());
        new["positions"][0]["tags"] = json!(["core", "swing"]);
        assert_eq!(turns(&old, &new), vec![json!(["set", ["positions", {"k": "id", "v": "p:QNC"}, "tags"], ["core", "swing"]])]);
        // two rows with the same symbol and no id: no field tells them apart
        let a = json!({"alloc": [{"symbol": "CH", "v": 1}, {"symbol": "CH", "v": 2}]});
        let mut b = a.clone();
        b["alloc"][1]["v"] = json!(3);
        assert_eq!(row_key(a["alloc"].as_array().unwrap()), None);
        assert_eq!(turns(&a, &b), vec![json!(["set", ["alloc"], b["alloc"]])]);
    }

    #[test]
    fn test_the_first_rows_of_an_empty_list() {
        let old = json!({"watchlist": []});
        let new = json!({"watchlist": [{"symbol": "QNC", "exchange": "TSX-V"}]});
        assert_eq!(turns(&old, &new), vec![json!(["rows", ["watchlist"], "symbol", ["QNC"], {"QNC": {"symbol": "QNC", "exchange": "TSX-V"}}])]);
    }

    #[test]
    fn test_every_shared_case_patches_to_every_other() {
        // real views: any one turned into any other, by patch, exactly
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../tests/wire");
        let mut views: Vec<Value> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().path())
            .filter(|p| p.extension().map_or(false, |x| x == "json"))
            .map(|p| serde_json::from_str::<Value>(&std::fs::read_to_string(p).unwrap()).unwrap()["wire"].clone())
            .collect();
        views.truncate(12);
        assert!(views.len() >= 8);
        for a in &views {
            for b in &views {
                turns(a, b);
            }
        }
    }
}
