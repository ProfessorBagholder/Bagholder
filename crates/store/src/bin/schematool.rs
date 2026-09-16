//! Runs the Rust store's `ensure` against the database at the given path, so
//! the result can be compared with what `store.ensure()` leaves behind.
fn main() {
    let path = std::env::args().nth(1).expect("path");
    let conn = rusqlite::Connection::open(&path).unwrap();
    bagholder_store::relabel::ensure(&conn).unwrap();
}
