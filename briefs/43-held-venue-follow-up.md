# Brief 43: #525's follow-up `2616ea3d`: right for the hand-entered trade; the same defect stands for every booked fill. Fix it where names are folded

## What holds

A trade entered by hand, or imported, on a held instrument now restates that instrument's venue. This is brief 42, item 1, for those two paths.
- `entries::tests::a_trade_by_hand_on_another_venue_is_never_the_holding_of_that_ticker` now covers every held share on every venue, in a fixed order.
- The book's test covers every venue.
- Undoing the fix in `person.rs` fails `a_trade_entered_by_hand_is_the_transaction_it_states`. I checked this.

The idle hour is accepted:
- in steady state the branch matches master (55.9 s and 106 MB, against master's 57.3 s and 110 MB);
- the first hour after the migration, 102 s and 169 MB, is a one-time re-read.

## What does not hold

1. **A booked fill erases the holding's venue.**
   - **Where:** the fill record names its instrument with no venue, dated the fill's day (`book/src/fills.rs:87`). Its payload carries none, though the order's listing names one (`OrderListing.exchange`).
   - **Probe** (a test in `book/tests/fills.rs`, run, then removed): SHOP held on Nasdaq gives `SHOP@US`. After its order's fill is booked, the newest name is `SHOP` with no venue, and the key fails with "SHOP names no venue".
   - **The effect:** after every order the app places, the holding's price, chart, news and filings wait until the pull brings the broker's row and the fill gives way. A first purchase through the app makes an instrument with no venue.
2. **The cause sits where names are folded, not in each writer.**
   - The fold starts a new run whenever a sighting's venue differs, and a sighting that states none counts as a run with no venue (`book/src/names.rs:109–119`). So any record that does not state the venue erases it. Patching writers one at a time leaves the next one to be found.
   - **`current_venue` has two problems of its own:**
     - **A back-dated trade gets today's venue.** It writes today's venue onto the trade's own day (`names.rs` `current_venue`, `person.rs:294`). A trade dated before a venue move therefore puts a run on the new venue into the past.
     - **It has its own venue rule.** It reads the code, then the words (`mic_of(mic).or(words)`): a second copy of the split in brief 42, item 6.

## Required

- **In the fold:** a sighting that states no venue says nothing about the venue. Its run keeps the venue that the last sighting stating one named, as of that day. A record that names a held instrument by its reference then needs no venue looked up, and a back-dated one is right on its own day.
- **The fill record carries the venue of its order's listing.**
- **A rule test.** It runs over generated histories of sightings, with and without venues, across days, from every writer: the broker, a fill, a hand entry and an import. It asserts that the current venue is the last one stated, and that a sighting with none never changes it.
- **The venue rule:** `current_venue` (or whatever replaces it) uses the one venue function that brief 42, item 6, requires.

The rest of brief 42 stands.
