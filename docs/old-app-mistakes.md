# The old app's mistakes

One line per known mistake of the old app (the Python app and the Rust port of it), each with what fails if it comes back. "Not guarded yet" names the stage that builds the guard. A scan means `rust/crates/engine/tests/no_guesses.rs`, which reads the new build's crates (`core`, `book`, `engine`, `sources`) for the old names and shapes.

A test in the old crates that records one of these as it behaves today is named `test_known_wrong_…` and points here; it goes with the old code at the switch.

## Identity and the record

- **Things identified by Wealthsimple's ids and bare symbols**, so a share could take a coin's price. Guarded: case "two instruments that share a ticker never share a lot" (`shares.json`); case "a quote from another kind's source prices nothing" (`positions_and_income.json`).
- **Trade ids built from dates, quantities and prices**, so a revised row or a re-inferred split renamed earlier trades and cut their notes loose. Guarded: `identity.json` cases; `book/tests/links.rs` `a_superseding_record_takes_the_trade_and_its_note`.
- **Broker rows rewritten at every start** (option buys relabelled "to open", multi-leg orders by their cash's sign). Guarded: `book/tests/records.rs` `a_changed_payload_is_a_revision_and_is_derived_again`; scan for `relabel`. Pinned in the old crates by `store/tests/relabel.rs`.
- **Bad rows dropped or read as zero.** Guarded: `sources/tests/reply.rs` (a reply that does not match its shape is a named mismatch); `book/tests/records.rs` `a_payload_the_mapping_cannot_read_is_kept_with_a_problem`; scan for `lenient`.
- **An incremental pull from a guessed overlap (fourteen days), with no record of a full read**, so a row posted or revised later than the overlap was never read, and the broker check could not tell a missing row from a real difference. Guarded: `wealthsimple/tests/pull.rs` (the first pull whole, then from the last full read and any row not yet final); `book/tests/statements.rs` (`only_a_read_of_every_page_is_a_full_read`).
- **A trade's cash signed by Wealthsimple's `amountSign`**, which is not the cash's direction (a buy is `positive`; a multi-leg row's sign is the opposite of its legs'), so every multi-leg roll's cash was booked backwards. Guarded: `wealthsimple/tests/mapping.rs` (`a_buy_pays_and_a_sale_receives_whatever_the_sign_says`, `a_multi_leg_order_is_its_legs_and_their_cash`).
- **A card purchase kept under its pending id after Wealthsimple posted it under another**, so each such purchase counted twice. Guarded: `wealthsimple/tests/pull.rs` (`a_pending_row_posted_again_under_another_id_leaves_the_book`, `an_imported_row_the_broker_no_longer_lists_leaves_the_book_on_the_first_full_read`).

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

- **No open trade, and the sold part of a held position made a trade of its own**, so a partial sale was scored as a finished trade and the position's later sales as another. Guarded: the cases in `open_trades.json` (a partly sold position is one open trade; win rate and expectancy count only closed trades).
- **A position marked at the person's own fill** when no price was read. Guarded: scan for `last_fill`, `LastFill`.
- **A tolerance for coins sold beyond what was held.** Guarded: scan for `fn dust`, `0.01 *`.
- **A residue under a dollar dropped.** Guarded: scan for `< 1.0)`.
- **A price-only index beside a total return.** The yearly return keeps dividends in the account's value, while its S&P 500 and S&P/TSX benchmarks were index levels without dividends (FRED, TMX), so every year flattered the account by the index's yield. Guarded: the tracker's total return in CAD (`engine/src/stat/benchmark.rs`), checked against Yahoo's adjusted close on recorded replies; case "the index is a total return in CAD" (`returns_filters_checks.json`).
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
