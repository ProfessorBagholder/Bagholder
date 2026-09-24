# The old app's mistakes

One line per known mistake of the old app (the Python app and the Rust port of it), each with what fails if it comes back. "Not guarded yet" names the stage that builds the guard. A scan means `rust/crates/engine/tests/no_guesses.rs`, which reads the new build's crates (`core`, `book`, `engine`, `sources`) for the old names and shapes.

A test in the old crates that records one of these as it behaves today is named `test_known_wrong_…` and points here; it goes with the old code at the switch.

## Identity and the record

- **Things identified by Wealthsimple's ids and bare symbols**, so a share could take a coin's price. Guarded: case "two instruments that share a ticker never share a lot" (`shares.json`); case "a quote from another kind's source prices nothing" (`positions_and_income.json`).
- **Trade ids built from dates, quantities and prices**, so a revised row or a re-inferred split renamed earlier trades and cut their notes loose. Guarded: `identity.json` cases; `book/tests/links.rs` `a_superseding_record_takes_the_trade_and_its_note`.
- **Broker rows rewritten at every start** (option buys relabelled "to open", multi-leg orders by their cash's sign). Guarded: `book/tests/records.rs` `a_changed_payload_is_a_revision_and_is_derived_again`; scan for `relabel`. Pinned in the old crates by `store/tests/relabel.rs`.
- **Bad rows dropped or read as zero.** Guarded: `sources/tests/reply.rs` (a reply that does not match its shape is a named mismatch); `book/tests/records.rs` `a_payload_the_mapping_cannot_read_is_kept_with_a_problem`; scan for `lenient`.

## Money and rates

- **Floating-point money**, summed with error compensation so five implementations would agree on the last digit. Guarded: scan, `f64` only in the statistics (`engine/src/stat`), `Dec::to_f64`, and the reading of the old database.
- **A constant 1.35 for a missing rate.** Guarded: scan for `1.35` and `FX_FALLBACK`; `rates.json`.
- **Every non-USD currency treated as CAD.** Guarded: case "any currency the Bank publishes converts; one it does not is never taken to be CAD" (`rates.json`).

## Corporate events and contracts

- **Split ratios inferred from the person's own fill prices.** Guarded: scan for `split_markers`; case "a split marker with its ratio from the record" (`events.json`). Pinned in the old crates by `model/tests/model.rs` `test_known_wrong_split_ratio_read_from_fill_prices`.
- **Every option contract taken to be 100 shares.** Guarded: scan for `option_multiplier` and `from_int(100)`; case "a contract of 150 shares, and one whose size is not stated" (`options.json`). Pinned in the old crates by `server/src/tests_orders.rs` `test_known_wrong_an_option_fill_nets_with_a_fixed_hundred_times_multiplier`.
- **Option terms read from the symbol's text.** Guarded: scan for `option_symbol`.
- **A contract count worked out from cash.** Guarded: scan for `infer_zero_qty_option_fills`, `is_clean_option_qty`.
- **Separate same-day orders folded into a roll.** Guarded: scan for `fold_option_rolls`; the roll cases in `options.json`.
- **A renamed holding found by its symbol.** Guarded: scan for `ticker_was_replaced`, `replacement_index`; case "a consolidation under a new security id" (`events.json`).
- **Shares arriving without cash booked at $0** (a stock dividend's value lost as income, a spin-off's child at no cost). Guarded: cases "a stock dividend is income…" and "a spin-off with two children…" (`events.json`).

## Figures

- **A position marked at the person's own fill** when no price was read. Guarded: scan for `last_fill`, `LastFill`.
- **A tolerance for coins sold beyond what was held.** Guarded: scan for `fn dust`, `0.01 *`.
- **A residue under a dollar dropped.** Guarded: scan for `< 1.0)`.
- **Payout frequency assumed monthly, or worked out from past distribution dates** (which showed Ninepoint's funds as monthly for six weeks after they went twice a month). Guarded: scan for `payments_per_year`; case "without the payer's own record nothing is worked out from the payments" (`positions_and_income.json`).

## Orders and brackets (stage 4)

- **An order whose send failed, a timeout included, is marked failed and never read back**, so an order Wealthsimple took can rest unseen. Not guarded yet.
- **A fill booked twice by two read-backs at once**: the booking and its "booked" mark are two writes with no shared lock. Not guarded yet.
- **Order handling sees only the newest 200 orders**, so a waiting bracket's entry past that is cancelled without a word. Not guarded yet.
- **The HTTP client re-sends a request once on a fresh connection** when a reused one fails, order POSTs included, so an order can be sent twice (`net/src/client.rs`). Not guarded yet.
- **A sale from the ticket does not wait for the stop's cancel to be confirmed.** Not guarded yet. Pinned in the old crates by `server/src/tests_brackets.rs` `test_known_wrong_a_sell_from_the_ticket_goes_out_before_the_stops_cancel_is_confirmed`.
- **A watched stop can fire while the target's cancel is unconfirmed.** Not guarded yet.
- **A bracket whose watched market sell is refused stays firing and is never retried.** Not guarded yet.

## Running it (stage 5)

- **With the container's port published beyond loopback, anyone on the network can place orders**: access is a loopback check. Not guarded yet.
- **An update's rollback can leave no working copy, and never restores the book.** Not guarded yet.
- **Failures discarded in dozens of places**, so a missing figure reads as a real one. Not guarded yet.
- **Every quote tick rebuilds every figure, and every open loads the whole model**, whatever is on screen. Not guarded yet.
