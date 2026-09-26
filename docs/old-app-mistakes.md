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
- **Every option contract taken to be 100 shares.** Guarded: scan for `option_multiplier` and `from_int(100)`; case "a contract of 150 shares, and one whose size is not stated" (`options.json`).
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

- **An order whose send failed, a timeout included, is marked failed and never read back**, so an order Wealthsimple took can rest unseen, and a bracket placed its exit again beside it. Guarded: `core/src/order.rs` (`an_answer_that_is_not_the_brokers_leaves_the_order_unconfirmed_until_read_back`); `server/src/tests_execution.rs` (`an_order_whose_answer_is_lost_after_the_broker_took_it_ends_working_by_read_back_never_failed`, `a_lost_answer_on_an_exit_the_broker_never_got_places_it_again_exactly_once`).
- **A fill booked twice by two read-backs at once**: the booking and its "booked" mark were two writes with no shared lock. Gone since 3c (the fill is Wealthsimple's own row); the filled quantity only rises, under one transaction: `tests_execution.rs` `an_order_filling_in_three_parts_read_twice_and_at_once_holds_exactly_what_filled`.
- **Order handling saw only the newest 200 orders**, so a waiting bracket's entry past that was cancelled without a word. Guarded: live orders and brackets are found by state through an index (`book/src/orders.rs`); `tests_execution.rs` `five_thousand_orders_leave_the_oldest_live_bracket_followed`.
- **The HTTP client re-sent a request once on a fresh connection** when a reused one failed, order POSTs included, so an order could go out twice. Guarded: order mutations go by `net::client::request_once` (`net/tests/kept_connection.rs`), and only through `server/src/orders/gate.rs` (`tests_boundary.rs`).
- **A sale from the ticket did not wait for the stop's cancel to be confirmed, and one that failed left neither stop nor sale.** Guarded: `tests_execution.rs` (`a_sale_from_the_ticket_goes_out_only_once_the_stops_cancel_is_confirmed`, `a_sale_whose_stop_cancel_is_not_confirmed_sells_nothing_and_the_stop_rests_again`, `a_sale_refused_puts_the_stop_back`).
- **A watched stop could fire while the target's cancel was unconfirmed.** Guarded: nothing is sent past an exit whose own answer is not known (`core/src/bracket.rs` `decide`; `core/tests/brackets.rs` `under_any_run_of_broker_answers_and_prices_no_exit_is_placed_while_one_is_in_flight_and_every_tick_settles`).
- **A bracket whose watched market sell was refused stayed firing and was never retried.** Guarded: `tests_execution.rs` `a_market_sell_rejected_after_it_was_taken_puts_the_stop_back`.
- **Rows being sent were not counted as in flight**, so an exit left sending by a crash did not stop a second. Guarded: `OrderState::in_flight` (`core/src/order.rs` `in_flight_is_every_state_the_broker_may_still_act_on`); the gate's `Held::InFlight` (`tests_execution.rs` `no_exit_is_placed_while_another_of_its_bracket_is_in_flight_and_nothing_is_written_for_it`).
- **A position left unwatched while its stop was being placed again** (after a trail move, a roll, an edit or an expiry). Guarded: a live bracket with no stop resting watches the level (`core/src/bracket.rs`); `tests_execution.rs` `the_target_reached_cancels_the_stop_and_a_lapsed_session_leaves_the_stop_watched_until_the_target_goes_out`.
- **Two writers**: the routes and the engine's tick changed a bracket at the same time. Guarded: every change is an event appended under the bracket's lock (`gate::bracket_lock`), and a move not allowed from the state the log says is refused and kept (`core/src/order.rs` `every_move_in_the_table_is_made_and_every_other_event_moves_nothing`; `book.states_disagreeing` checked after each scenario).
- **The retry wait counted from the bracket's last change**, so any edit reset it. Guarded: it counts from the last refusal's event (`Bracket::may_retry`; `core/tests/brackets.rs` `a_refused_exit_is_tried_again_after_a_minute_five_fifteen_then_hourly`).
- **A dry ticket with legs made a bracket that waited forever.** Guarded: `tests_execution.rs` `with_orders_off_a_ticket_with_legs_is_recorded_and_no_bracket_waits`.
- **Order answers and the page's numbers read leniently.** Guarded: `gate::read_extended` and the feed reader refuse what does not match (`tests_execution.rs` `wealthsimples_order_answer_is_read_strictly`; `orders/readback.rs` `a_feed_node_is_read_strictly`); the ticket reads exact decimals (`orders::PageDec`).
- **The page worked out order values and leg amounts from floats, a contract's size guessed from its symbol.** Guarded: the orders document carries every amount (`orders/doc.rs`); `web/src/no_money_arithmetic.test.ts` has no Orders panel entry.
- **No record of who asked, no history.** Guarded: every order and bracket event carries its asker (`tests_execution.rs` `who_asked_is_kept_with_every_request`).

## Running it (stage 5)

- **With the container's port published beyond loopback, anyone on the network can place orders**: access is a loopback check. Not guarded yet.
- **An update's rollback can leave no working copy, and never restores the book.** Not guarded yet.
- **Failures discarded in dozens of places**, so a missing figure reads as a real one. Not guarded yet.
- **Every quote tick rebuilds every figure, and every open loads the whole model**, whatever is on screen. Not guarded yet.
