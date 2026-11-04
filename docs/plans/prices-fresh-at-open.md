# Plan: prices are kept fresh while Bagholder runs, and each shows as it is read

## For the owner to decide

Nothing open: the owner's decision is in `docs/decisions.md` (2026-11-04).

## Scope

**In it:**
- What is held and what is followed (the holdings, the watchlist, the Markets tiles) is quoted every `QUOTES_EVERY` (a minute) while the app runs, whether or not a page is open. The option contracts held are read from their chains the same way, with `options::chain_due` unchanged.
- With no page open, a quote whose source stated it more than `QUOTE_RESTS` (15 minutes) before the read that kept it is not asked again until `QUOTE_RESTS` after that read: its market isn't trading. A page opening asks every one at once, as now.
- Each quote reaches the figures and the page as soon as it is kept, not when the whole pass ends.
- SPEC (Price `position-7`; the schedules of `market-data-4`, `-5`, `-6`, `-8`, `-9`), `docs/architecture.md` §13, `docs/decisions.md`, `TIMED_WAITS`.
- The universes' section comment in `feeds.rs` that still says nothing is read at start (stale since 2.20.0).

**Out of it:**
- **TMX in one request for held listings.** `getQuoteForSymbols`'s `Quote` type has no `exDividendDate` and no `datetime`, and names a venue `exchange` where `getQuoteBySymbol` says `exchangeCode` (checked here 2026-11-04 from the schema's own refusals). The reader keeps the ex-dividend date with each quote (SPEC `market-data-4`) and checks each answer's venue, so the batch can't replace the single quote without a second read path. Not in this change.
- **Yahoo in one request.** Not read: Yahoo answered every request from here with 429 on 2026-11-03 and 2026-11-04, so no reply of a batched form has been seen. A form whose reply was never read is never built.
- Market calendars (opening hours, holidays): the source's own stated time decides, below.
- Yahoo's refusals: the rest after a refusal (`grown_rest`) is unchanged.

## What happens now

Read from the code, 2026-11-04, at f8806821:
- **No page, no quotes:** `due::quotes_owed` is `app.events.watchers() > 0` (2.11.3, 094292ee). A page opening shows the prices read when a page was last open, which can be hours old.
- **Nothing shows until the pass ends:** `due::pass` reads every owed listing in one `quotes::read_quotes` call, the holdings first and then what is followed, then calls `Figures::price_changed` for each. `due::run` signals the page once, after the pass. So every tile and every holding moves only when the last listing has been read.
- **The pass is long:** each listing is one request. Yahoo is paced two seconds apart (`YAHOO_PACE`), and every tile is quoted through Yahoo (indices, futures, rates and pairs, `read_quotes`), behind the holdings. A pass with N Yahoo listings takes at least 2N seconds before anything moves.
- **Yahoo refusing:** on 2026-11-03 every Yahoo request from here answered 429 for at least half an hour. After a refusal Yahoo is left alone for ten minutes, longer after more (`grown_rest`).

## What the sources state

- Yahoo's chart states `regularMarketTime`, the time of the last trade, and that is the quote's time (`yahoo::parse`, "the reply's regular market price at its `regularMarketTime`"). A market that isn't trading keeps answering its last trade's time. Recorded: `tests/replies/yahoo/CAD=X-range-1d.json`.
- Coinbase Exchange's ticker states its trade's time; the spot price is stamped with its reply's date less its allowance.
- Cboe Canada states its trade's time, and a listing with no trade yet stands at the previous close at the time it is read.
- TMX states no trade time: its quote's time is when it served it, so a TMX quote never rests.

## Approach

1. **Owed while the app runs.** `quotes_owed` goes. `demand` owes every held and followed listing on each pass, less the resting ones while no page is open. `failing_quotes` goes from `demand`: a failing quote is owed like any other, as it is today while a page is open, and a refused source is still left alone through its rest by the limiter. `rest_ends` stays in `next_due`. The option chains are read on every pass (`chain_due` as now).
2. **Resting.** `due::QUOTE_RESTS` (15 minutes, a named constant with its reason): a quote whose stated time is more than that before it was received is a market not trading (closed, a holiday, a halt), and asking it each minute brings back the same price. Computed from what the cache already keeps (`quoted_at`, `received_at`), so no new state. With no page open such a quote is asked again `QUOTE_RESTS` after it was received. The first read after a market opens is then at most 15 minutes late while nobody is looking, and at once when a page opens.
3. **Each as it lands.** The pass reads listing by listing. After each one, `price_changed`, and a signal to the pages when anything moved, so a tile or a holding's row is patched as its quote is kept (the stream patches only what moved, as it does now).
4. **When the next pass runs.** `next_due`: `QUOTES_EVERY` after a pass that asked anything; otherwise the earliest resting end; with the other deadlines as now.
5. **Cost:** with no page open, the quotes a page would cost, less every listing whose market isn't trading. `TIMED_WAITS` gains `QUOTE_RESTS` with its reason; `QUOTES_EVERY`'s reason is reworded.

## Acceptance criteria

- [ ] A started app with no page open owes every held and followed listing. Replaces `due::tests::what_is_held_and_followed_is_quoted_while_a_page_is_open_whatever_it_shows` (which asserted nothing with no page) and the no-page halves of `events::tests::test_a_page_opening_wakes_the_figure_path_with_quotes_owed` and `test_quotes_are_owed_while_a_page_is_open_whatever_it_shows`.
- [ ] With no page open, a quote stated more than `QUOTE_RESTS` before it was received is not owed until `QUOTE_RESTS` after; one stated fresh is owed; with a page open, both are owed. A test on the cache's stamps.
- [ ] Each quote is applied before the next listing is read. A test whose stand-in for the second listing's read finds the first listing's price already in the figures. Plant: `price_changed` moved back after the loop fails it.
- [ ] The held contracts' chains are read with no page open. Replaces `held_contracts_are_read_while_a_page_shows_a_holding_s_price`'s no-page half.
- [ ] `next_due` with every listing resting is the earliest resting end, not a minute. A unit test.
- [ ] `TIMED_WAITS` lists `QUOTE_RESTS`; `test_no_wait_on_a_clock_that_is_not_accounted_for` passes. Capacity passes.
- [ ] SPEC's schedules say what the code does, page or none.
