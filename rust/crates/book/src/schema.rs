//! The book's schema, as numbered migrations. A released migration is never
//! edited: each version's schema is committed in `schema/v<N>.sql` and the tests
//! compare (`tests/schema.rs`).

use bagholder_sqlite::migrate::{Migration, Schema};

pub static MIGRATIONS: [Migration; 1] = [Migration { number: 1, name: "the book", sql: include_str!("../migrations/001-the-book.sql") }];

pub static SCHEMA: Schema = Schema {
    name: "book",
    // "BHBK" in the file's header: a book, not another store's file
    application_id: 0x4248_424B,
    migrations: &MIGRATIONS,
};
