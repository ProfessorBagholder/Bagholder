# The owner's decisions

Every decision the owner has made about the app and the work, newest first, one line each with its reason and where it was made. A decision is written here and pushed the moment it is made, before any work relies on it: a decision that lives only in a local plan is invisible to the reviewer. Where a brief, a plan or a doc disagrees with this file, this file wins and the other is fixed. A decision here is settled: it is never asked again or reopened by a suggestion. Each names the test that fails if it is broken, or says "review only" where no test can hold it (brief 07).

## 2026-09-25

- **A managed account's trades make no round trips.** The buys and sales Wealthsimple makes in an account it manages count in holdings, cash, income, account value and the broker check, and are in no Trades list, journal or trade statistic: they are not the person's trades. (Owner; brief 08.) Held by: `engine/tests/cases/open_trades.json` (a managed account's buys and sales make no round trip, and its holding stays).

## 2026-09-24

- **Calls to Wealthsimple's unofficial API are kept to what is necessary.** The first pull reads each account in full once; after that only what changed is read, and an account is re-read only over the span a broker-check difference points to. Held by a test counting a pull's requests (stage 3b). (Owner; brief 07.) Held by: `wealthsimple/tests/pull.rs` (a pull with nothing new, a pull with one new trade).
- **No settings hook as a backstop (brief 05 §4 declined).** A script policing commands before they run is tedious and brittle; the rule in `CLAUDE.md` that every action is the best course for its objective is what holds. (Owner.) Held by: review only.
- **A trade runs from a position's first fill until it is flat; a partly sold position is an open trade in the Trades list; a partial sale's P&L counts on its day** in realized P&L and the monthly figures, while win rate and the other closed-trade statistics count a trade once it has closed. As Tradervue, TraderSync and TradeZella do (sources in `docs/plans/trade-open-to-flat.md`). Replaces `SPEC.md`'s "there is no open trade" and the engine's trade of the sold part. (Owner, after the blind case check.) Held by: `engine/tests/cases/open_trades.json`.
- **Every definition is checked against how the leading journals and the industry do it, with sources**, before it is recommended or planned; every plan carries that section, and the reviewer checks it at the gate. (Owner.) Held by: review only (a checkbox test cannot show research was done; owner, 2026-09-24).
- **The equity series, returns and drawdown come from Wealthsimple's stated daily account value.** Each account's daily value and net deposits are the broker's historical financials, which the pull already reads (`SPEC.md` §2, "NAV from sync"); the total is the sum of the accounts; returns are chain-linked daily returns net of deposits and withdrawals. The engine keeps the value per account, so an account with no broker statement can get a computed value later; that is not built now. Recorded option closes, the after-close option read, and past closes of every holding for the equity series stop; only the past closes a chart shows and an option's underlying's close on its expiry day (the expiry rule) are read. "Equity is Bagholder's own" comes off the switchover list. Replaces the 2026-09-24 decision that the app reads each option contract's close after the close. (Owner; brief 06 §1.) Held by: `engine/tests/cases/equity.json`; `book/tests/statements.rs`.
- **Option prices on screen stay Cboe's delayed chains, fetched only when needed.** A price is stored with its own time and fetched again only when a screen shows it, its market is open, and the stored one is older than the source can improve on; after the close the stored final price stands until the next session. A page opening or reloading never causes a fetch by itself. (Owner; brief 06 §2.) Held by: `sources/tests/options.rs` (`a_later_read_asks_only_for_a_newer_chain_and_a_304_keeps_the_prices_held`, `a_shown_contracts_chain_is_due_by_what_the_chain_held_states`).
- **Each benchmark is a total return in CAD**, like the owner's own figure (account value net of flows, dividends staying in it): the dividend-adjusted close of an ETF tracking the index, converted daily at the Bank of Canada's rate. Price-only FRED and TMX index levels stop being benchmarks. (Owner; brief 06 §3.) Held by: the tracker test against Yahoo's adjusted close in `server/src/read_sources.rs`.
- **Stage 3b: the facts no feed carries are entered by the owner**, as Sharesight and similar trackers do: the cost of a holding moved in (an opening balance in "Add trade"), and a spin-off's or return of capital's allocation from the issuer's published figure (on the trade detail beside the journal), each defined in `SPEC.md` first. 3b has no per-issuer corporate-event readers: the official Canadian corporate-action feeds are paid. (Owner; brief 06 §4.) Held by: `engine/tests/cases/entries.json`; `book/tests/person.rs`.
- **The user-facing UI and UX stay at parity; the engineering underneath is redesigned.** Nothing the person sees or does changes unless `SPEC.md` changes on purpose. (Brief 01 §0.) Held by: the page's screenshot baselines (`web/`); otherwise review only.
- **Make the app work properly as it is; nothing is built for expansion the owner hasn't asked for.** (Brief 01 §0.) Held by: review only.
- **Correctness over speed; no deadline.** (Brief 01 §0.) Held by: review only.
- **The work is the migration.** Nothing is done, proposed or asked about for the app that runs today or for the period before the Rust+Svelte build replaces it, unless it bears directly on building the new version properly. (Briefs 01–03, revised d08567e.) Held by: review only.
- **The known order defects are fixed in the new build, in stage 4**, not in the running app (`docs/old-app-mistakes.md`, "Orders and brackets"). (Brief 01 §0.) Held by: review only until stage 4's misbehaving-broker tests.
- **Real figures and tickers from the owner's data in committed docs and fixtures are acceptable.** (Brief 01 §0; owner, 2026-09-23.) Held by: review only.
- **On open, the page shows its last figures at once and updates only when newer data arrives.** (Brief 01 §0, §2.5.) Held by: review only until stage 5's second-open test.
- **Where a fund company's publication cannot be read at all** (its site is behind a bot check the app never gets past: Mackenzie for QCN and QUU, WisdomTree for WQTM), distributions come from the exchange-side record (TMX for a Canadian listing, Yahoo's dividend events for a US one), marked with that source; the schedule only where a source states it, otherwise the fund's annual income is a gap naming why. A fixed list, never a fallback for a failed read. (Owner; brief 02 #1.) Held by: `sources/tests/payers.rs`.
- **No hold on a quote that moved far from the last close.** A real price is never withheld on numbers nobody chose. (Brief 02 #3.) Held by: `sources/tests/quotes.rs`.
- **Only held positions' payers are read.** (Brief 02 #4.) Held by: `sources/tests/payers.rs`.

## 2026-09-23

- **A fund's schedule and distributions come from its fund company's own publication, never worked out from past dates.** Dates lag a change: when Ninepoint's funds went from monthly to twice a month, the dates would have shown monthly for six weeks and halved the income figure. (Owner; stage 3a plan.) Held by: `sources/tests/payers*.rs`; the scan for `payments_per_year` (`engine/tests/no_guesses.rs`).
- **A payer's rate is the cash per unit of its latest distribution gone ex**; a reinvested part is stored as stated and pays nothing. No invented kinds of distribution. (Owner; stage 3a plan.) Held by: `engine/tests/cases/positions_and_income.json`.
- **A missing fact a figure needs is found out, never assumed or shown as unknown.** Payout frequency is looked up from the sources that state it; a fact still unfound is a problem the app keeps working on. (Owner.) Held by: `engine/tests/cases/positions_and_income.json` (a payer with no stated frequency waits); otherwise review only.
- **Design from facts known to exist.** The Bank of Canada publishes a rate for every business day: the only failure is ours (the read failed), handled and shown as a source failure, never a data case with a default. (Owner.) Held by: `engine/tests/cases/rates.json`.
- **Rates are the Bank of Canada's, for every currency it publishes**, on the day of each transaction, the previous business day's on a weekend or holiday; never a constant (the old 1.35), and no currency is taken to be CAD. (Owner; `docs/architecture.md` §6.) Held by: `engine/tests/cases/rates.json`; the scan for `1.35` and `FX_FALLBACK` (`engine/tests/no_guesses.rs`).
- **A live mark uses the latest rate the Bank of Canada has published.** (Owner; stage 2 plan, b0b934e.) Held by: `engine/tests/cases/rates.json` (the live rate).
- **External answers are read strictly against the shape the source really sends**; a mismatch is a visible error naming the source and field, never coerced. (Owner.) Held by: `sources/tests/reply.rs`; `wealthsimple/tests/mapping.rs` (`a_reply_of_another_shape_is_unreadable_and_named`); the scan for `lenient`.
- **A second brokerage is expected.** Wealthsimple sits behind one adapter so another fits beside it; nothing is built for a second one until asked (2026-09-24 above). Self-hosted comes first and always. (Owner.) Held by: `core/tests/boundaries.rs` (the broker interface depends on no adapter).
- **The watched stop stays a market sell.** A limit can fail to fill, and the stop exists to get out. (Owner; d0a95f8.) Held by: `server/src/tests_brackets.rs` (`test_a_watched_stop_fires_as_a_market_sell_when_wealthsimple_takes_no_stop_order`).
- **Clear data does what `SPEC.md` and its confirmation say.** (Owner.) Held by: review only until 3c.
- **No backups feature** unless the owner asks for one; it is never slipped into a design as if agreed. (Owner.) Held by: review only.
- **Autonomous agent trading is the person's choice.** (Owner; `docs/architecture.md` §14.) Held by: review only.

## 2026-09-22

- **A failure is always visible**: never a silent skip, a swallowed error or a log line only. (Owner.) Held by: `wealthsimple/tests/pull.rs` (a part that failed is named in the report); otherwise review only.
- **A case is correct for every input, not only the owner's book.** The owner's data never excuses a wrong case. (Owner.) Held by: review only.

## 2026-09-20

- **The Rust server with the Svelte page is the only build going forward**; the Python and Go apps and `ledger.html` are frozen and removed at cutover. (Owner.) Held by: review only.
- **One value changes, only its element updates**: never a screen, card, table or section redrawn for one value. (Owner; `docs/architecture.md` §13.) Held by: `web/src/lib/tick.svelte.test.ts`.
- **Load and compute only what changed, when it changed**: no blind timers, no full reloads. A timer that remains is argued for in `TIMED_WAITS` (`rust/crates/server/src/tests_misc.rs`). (Owner.) Held by: `server/src/tests_misc.rs` (`test_no_wait_on_a_clock_that_is_not_accounted_for`); `wealthsimple/tests/pull.rs`.

## Earlier

- **Exits rest at the broker good till cancelled**: a protective order never depends on the app being up. (Owner, 2026-09-10.) Held by: `server/src/tests_brackets.rs` (`test_exits_go_out_good_till_cancelled_whatever_the_entry_was`).
- **Only what was asked**: no captions, tooltips, notes, helper text or `title` attributes. (Owner; `SPEC.md` §1.) Held by: review only.
- **Per-instrument figures in the instrument's own currency; aggregates in CAD, never labelled.** (Owner; `SPEC.md` §1.) Held by: review only.
- **Raw broker rows are never rewritten.** (Owner; `SPEC.md` §1.) Held by: `book/tests/records.rs` (`a_changed_payload_is_a_revision_and_is_derived_again`); the scan for `relabel`.
- **Nothing synthetic on a chart.** (Owner; `SPEC.md`.) Held by: review only.
- **No visible browser window in any design**; routes are found over HTTP first. (Owner, 2026-09-13.) Held by: review only.
