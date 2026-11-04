# Plan: prices are kept fresh while Bagholder runs, and each shows as it is read

## For the owner to decide

Nothing open: the owner's decision is in `docs/decisions.md` (2026-11-04). The held option contracts' chains read with no page open follow from it, and the 2026-09-24 entry's clause they replace is marked so (below).

## Scope

**In it:**
- What is held and what is followed (the holdings, the watchlist, the Markets tiles) is quoted every `QUOTES_EVERY` (a minute) while the app runs, whether or not a page is open. The option contracts held are read from their chains the same way, `options::chain_due` unchanged: one read per underlying while its session is open, one after the close, none until the next session.
- **Quiet markets, with no page open.** A listing whose market is not trading is not asked every minute:
  - on a venue whose calendar the app holds (TSX and TSX Venture, the CSE, Cboe Canada, the US exchanges), while its session is known closed and its quote was received after that session's close settled, until the next open;
  - otherwise, when its source states a trade time more than `QUIET_RECHECK` (15 minutes) before the read that kept it, until `QUIET_RECHECK` after that read.
  With a page open, every quote is asked every minute (the Architect's ruling on this plan): someone watching at the bell sees the open within a minute.
- Each quote reaches the figures and the page as soon as it is kept, not when the whole pass ends.
- `bagholder_core::sessions` gains the CSE's and Cboe Canada's calendars from their own published pages, and a session's next open and last close.
- SPEC (Price `position-7`; the schedules of `market-data-4`, `-5`, `-6`, `-8`, `-9`), `docs/architecture.md` §13, `docs/decisions.md`, `TIMED_WAITS`.
- The universes' section comment in `feeds.rs` that still says nothing is read at start (stale since 2.20.0).

**Out of it, and when each is taken up:**
- **Yahoo in one request.** Its batched form's reply has never been read here: Yahoo answered every request from this host with 429 on 2026-11-03 and 2026-11-04 (below). The batched form's reply is recorded the first time Yahoo answers from here, and its plan follows straight after this change if the owner's copy is being refused (below), or once this build's Yahoo requests are measured on the owner's copy if not.
- **TMX in one request for held listings.** `getQuoteForSymbols`'s `Quote` type has no `exDividendDate` and no `datetime`, and names a venue `exchange` where `getQuoteBySymbol` says `exchangeCode` (checked 2026-11-04 from the schema's own refusals). The reader keeps the ex-dividend date with each quote (SPEC `market-data-4`) and checks each answer's venue. Its plan follows this change: the minute's refresh through the batch (price, change, currency, venue), with the single quote kept once a day per listing for the ex-dividend date. It is sized by this build's pass time measured on the reference book.
- Yahoo's refusals: the limiter's rest after a refusal (`grown_rest`) is unchanged.

## What happens now

Read from the code, 2026-11-04, at f8806821:
- **No page, no quotes:** `due::quotes_owed` is `app.events.watchers() > 0` (2.11.3, 094292ee). A page opening shows the prices read when a page was last open, which can be hours old.
- **Nothing shows until the pass ends:** `due::pass` reads every owed listing in one `quotes::read_quotes` call, the holdings first and then what is followed, then calls `Figures::price_changed` for each. `due::run` signals the page once, after the pass. So every tile and every holding moves only when the last listing has been read.
- **The pass is long:** each listing is one request. Yahoo is paced two seconds apart (`YAHOO_PACE`), and every tile is quoted through Yahoo (indices, futures, rates and pairs, `read_quotes`), behind the holdings. A pass with N Yahoo listings takes at least 2N seconds before anything moves.
- **The chains:** `pass` reads the held contracts' chains only while `quotes_owed`. SPEC `market-data-9` still says "only when a screen shows the contract", which the code has not done since 2.11.3.

## What the sources state

- **Yahoo's chart** states `regularMarketTime`, the time of the last trade, and that is the quote's time (`yahoo::parse`). A market that isn't trading keeps answering its last trade's time. Recorded: `tests/replies/yahoo/CAD=X-range-1d.json`.
- **TMX** states no trade time: its `datetime` is when it served the quote. `tests/replies/tmx/edited-quote-ENB-price-null.json` states 2026-09-27T02:13:19-04:00, a Sunday at two in the morning, and `read_tmx` keeps it as `quoted_at`. So TMX's own time can never show a quiet market.
- **Cboe Canada** states its trade's time; a listing with no trade yet is kept at the previous close at the time it is read (`ctx.now`).
- **Coinbase:** the Exchange's ticker states its trade's time; the spot price is stamped with its reply's date less its allowance.
- **The calendars** (`bagholder_core::sessions`, held as data with their sources, a date past what an exchange published treated as open):
  - TSX and TSX Venture, NYSE and Nasdaq, US options: already held.
  - **The CSE** ("Trading Resources", thecse.com, Trading Hours and Holidays, read 2026-11-04): CSE-listed securities trade continuously 9:30 am ET to 4 pm ET. Closed 2026-01-01, 02-16, 04-03, 05-18, 07-01, 08-03, 09-07, 10-12, 12-25 and 12-28, and 2027-01-01. Open on 09-30 and 11-11 ("Open for Trading – Settlement Holiday only"); no early close listed. Published to 2027-01-01.
  - **Cboe Canada** ("Hours & Holidays", Canadian Equities, cboe.com, read 2026-11-04): Cboe Canada listed securities (NEO-L) trade continuously 9:30am–4:00pm. Closed 2026-01-01, 02-16, 04-03, 05-18, 07-01, 08-03, 09-07, 10-12, 12-25 and 12-28; 2027-01-01, 02-15, 03-26, 05-24, 07-01, 08-02, 09-06, 10-11, 12-27 and 12-28. 09-30 and 11-11 are settlement holidays, not closures. A "Christmas Early Close" on 12-24 of each year is listed without its time, so that day is held as a full session: the calendar never takes a session as closed while it may trade. Published to 2027-12-31.

## Yahoo's refusals, before the build

Checked 2026-11-03 and 2026-11-04 (no figures, no symbols):
- **From this host:** every Yahoo request I made answered 429: the chart at 2026-11-03 14:52Z (three requests: with a browser's signature, without, and on `query2`), the batched form twice at 14:52Z, the chart at 15:15Z, and a plain request at 2026-11-04 14:34Z. Seven requests over two days, mine; a further check sending a browser's identity was refused here as bot-check circumvention, and is not tried again.
- **The owner's copy:** its own health, the header's failure line (`health::failures`, said after two failures in a row), states no Yahoo failure since its start at 2026-11-03 16:00Z. Today's rule asks Yahoo only while a page is open, which nothing outside the book shows, so the header's silence does not tell whether it was asked. Not established yet; the owner is asked to look at the header after two minutes on the Markets tab (2026-11-04). If it says "Yahoo Finance refused the request (too many)", the Yahoo batch is the next change.
- **What else on this host asks Yahoo:** the owner's copy. No scratch copy runs (checked). The test suites and the browser tests run offline (`BAGHOLDER_OFFLINE=1` in `web/e2e/serve.mjs`, `tests_common::home`). No other Claude session's transcript on this host fetched Yahoo. The other containers here are the owner's media and tooling stack, outside this work, and were not inspected.
- **Requests per hour this plan makes**, for Y listings quoted through Yahoo (tiles, US holdings, US watched listings), at most 1,800 an hour whatever Y (the two-second pace):
  - page open: 60·Y, as today;
  - no page, during a listing's session: 60 for it; outside it: none for a US listing (its calendar), 4 for an index (the quiet recheck), 60 for a future, pair or rate while it trades.
  - The default six tiles (four indices, a gold future, bitcoin) with ten US listings: 960 an hour with a page open all day, as today; with no page, 960 in the US session, about 140 overnight on a weekday, about 80 on a weekend (bitcoin trades through it).

## Approach

1. **Owed while the app runs.** `quotes_owed` goes. `demand` owes every held and followed listing on each pass, less the quiet ones while no page is open. `failing_quotes` goes from `demand`: a failing quote is owed like any other, as it is today while a page is open; a refused source is still left alone through its rest by the limiter, and `rest_ends` stays in `next_due`. The chains are read on every pass, `chain_due` as now.
2. **Quiet.** `due::quiet(listing, kept quote, now) -> Option<Timestamp>`, the instant it is next owed, from what the cache already keeps (`quoted_at`, `received_at`), so no new state:
   - a listing whose venue has a calendar (`sessions::venue_of`, which gains XCNQ and NEOE): quiet while `in_session` is `Some(false)` and the quote was received after the last session's close settled (`market::settled_at`); owed again at `sessions::next_open`. `None` (past the published calendar) is not quiet.
   - otherwise: quiet when `quoted_at` is more than `QUIET_RECHECK` before `received_at`; owed again at `received_at + QUIET_RECHECK`.
   With a page open, nothing is quiet.
3. **Each as it lands.** The pass reads listing by listing. After each one, `price_changed`, and a signal to the pages when anything moved, so a tile or a holding's row is patched as its quote is kept.
4. **When the next pass runs.** `next_due`: `QUOTES_EVERY` after a pass that asked anything; otherwise the earliest instant a quiet listing is owed again; with the other deadlines as now.
5. **Names.** "Quiet" is a market not trading; "rest" stays the limiter's word for a refused source (`grown_rest`, `rest_ends`). Code, `TIMED_WAITS` and SPEC keep the two apart.
6. **Cost:** with no page open, the quotes a page would cost, less every quiet listing. `TIMED_WAITS` gains `QUIET_RECHECK` and the wait to a session's next open, each with its reason; `QUOTES_EVERY`'s reason is reworded.

## Acceptance criteria

- [ ] A started app with no page open owes every held and followed listing. Replaces `due::tests::what_is_held_and_followed_is_quoted_while_a_page_is_open_whatever_it_shows` (which asserted nothing with no page) and the no-page halves of `events::tests::test_a_page_opening_wakes_the_figure_path_with_quotes_owed` and `test_quotes_are_owed_while_a_page_is_open_whatever_it_shows`.
- [ ] **The calendar, at a clock seam:** with no page open, a TSX holding whose quote was read after Friday's close settled is not owed at 02:00 Sunday Toronto time and is owed at 09:30 Monday; a CSE and a Cboe Canada holding the same; on a holiday each venue's own list decides; with a page open, all are owed.
- [ ] **The stated time:** with no page open, a quote stated more than `QUIET_RECHECK` before it was received, on a venue with no calendar, is not owed until `QUIET_RECHECK` after; one stated fresh is owed. A test on the cache's stamps.
- [ ] `sessions`: the CSE's and Cboe Canada's hours, holidays, settlement holidays (open) and published ends; `next_open` across a weekend and a holiday; a date past the published calendar is `None`.
- [ ] Each quote is applied before the next listing is read. A test whose stand-in for the second listing's read finds the first listing's price already in the figures. Plant: `price_changed` moved back after the loop fails it.
- [ ] The held contracts' chains are read with no page open. Replaces `held_contracts_are_read_while_a_page_shows_a_holding_s_price`'s no-page half.
- [ ] `next_due` with every listing quiet is the earliest instant one is owed again, not a minute. A unit test.
- [ ] `TIMED_WAITS` lists the new waits; `test_no_wait_on_a_clock_that_is_not_accounted_for` passes. Capacity passes.
- [ ] In the PR: the measured cost of a pass's per-listing applies on the capacity book, and the page's idle-minute budget (architecture §13).
- [ ] SPEC's schedules say what the code does, page or none; the 2026-09-24 decision's clause on the chains is marked replaced.
