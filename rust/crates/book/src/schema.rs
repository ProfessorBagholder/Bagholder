//! The book's schema, as numbered migrations. A released migration is never
//! edited: each version's schema is committed in `schema/v<N>.sql` and the tests
//! compare (`tests/schema.rs`).

use bagholder_sqlite::migrate::{Migration, Schema};

pub static MIGRATIONS: [Migration; 13] = [
    Migration { number: 1, name: "the book", sql: include_str!("../migrations/001-the-book.sql") },
    Migration { number: 2, name: "the facts", sql: include_str!("../migrations/002-the-facts.sql") },
    Migration { number: 3, name: "the facts as stated", sql: include_str!("../migrations/003-the-facts-as-stated.sql") },
    Migration { number: 4, name: "series that ended", sql: include_str!("../migrations/004-series-that-ended.sql") },
    Migration { number: 5, name: "no recorded closes", sql: include_str!("../migrations/005-no-recorded-closes.sql") },
    Migration { number: 6, name: "the broker's statements", sql: include_str!("../migrations/006-the-broker-s-statements.sql") },
    Migration { number: 7, name: "the person's zone", sql: include_str!("../migrations/007-the-person-s-zone.sql") },
    Migration { number: 8, name: "a distribution's form", sql: include_str!("../migrations/008-a-distribution-s-form.sql") },
    Migration { number: 9, name: "buying power", sql: include_str!("../migrations/009-buying-power.sql") },
    Migration { number: 10, name: "margin boost", sql: include_str!("../migrations/010-margin-boost.sql") },
    Migration { number: 11, name: "units paid on", sql: include_str!("../migrations/011-units-paid-on.sql") },
    Migration { number: 12, name: "a value stated on arrival", sql: include_str!("../migrations/012-a-value-stated-on-arrival.sql") },
    Migration { number: 13, name: "orders and brackets", sql: include_str!("../migrations/013-orders-and-brackets.sql") },
];

pub static SCHEMA: Schema = Schema {
    name: "book",
    // "BHBK" in the file's header: a book, not another store's file
    application_id: 0x4248_424B,
    migrations: &MIGRATIONS,
};
