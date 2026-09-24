# The owner's decisions

Every decision the owner has made about the app and the work, newest first, one line each with its reason and where it was made. A decision is written here and pushed the moment it is made, before any work relies on it: a decision that lives only in a local plan is invisible to the reviewer. Where a brief, a plan or a doc disagrees with this file, this file wins and the other is fixed. A decision here is settled: it is never asked again or reopened by a suggestion.

## 2026-09-24

- **The user-facing UI and UX stay at parity; the engineering underneath is redesigned.** Nothing the person sees or does changes unless `SPEC.md` changes on purpose. (Brief 01 §0.)
- **Make the app work properly as it is; nothing is built for expansion the owner hasn't asked for.** (Brief 01 §0.)
- **Correctness over speed; no deadline.** (Brief 01 §0.)
- **The work is the migration.** Nothing is done, proposed or asked about for the app that runs today or for the period before the Rust+Svelte build replaces it, unless it bears directly on building the new version properly. (Briefs 01–03, revised d08567e.)
- **The known order defects are fixed in the new build, in stage 4**, not in the running app (`docs/old-app-mistakes.md`, "Orders and brackets"). (Brief 01 §0.)
- **Real figures and tickers from the owner's data in committed docs and fixtures are acceptable.** (Brief 01 §0; owner, 2026-09-23.)
- **On open, the page shows its last figures at once and updates only when newer data arrives.** (Brief 01 §0, §2.5.)
- **Where a fund company's publication cannot be read at all** (its site is behind a bot check the app never gets past: Mackenzie for QCN and QUU, WisdomTree for WQTM), distributions come from the exchange-side record (TMX for a Canadian listing, Yahoo's dividend events for a US one), marked with that source; the schedule only where a source states it, otherwise the fund's annual income is a gap naming why. A fixed list, never a fallback for a failed read. (Owner; brief 02 #1.)
- **An option contract's close is read by the app itself each session day after the close**, like every due read. Nothing is set up outside the app to capture days before it runs; a day with no recorded close takes the broker's stated account value where there is one and waits where there is none. (Owner, answering brief 02 #2.)
- **No hold on a quote that moved far from the last close.** A real price is never withheld on numbers nobody chose. (Brief 02 #3.)
- **Only held positions' payers are read.** (Brief 02 #4.)

## 2026-09-23

- **A fund's schedule and distributions come from its fund company's own publication, never worked out from past dates.** Dates lag a change: when Ninepoint's funds went from monthly to twice a month, the dates would have shown monthly for six weeks and halved the income figure. (Owner; stage 3a plan.)
- **A payer's rate is the cash per unit of its latest distribution gone ex**; a reinvested part is stored as stated and pays nothing. No invented kinds of distribution. (Owner; stage 3a plan.)
- **A missing fact a figure needs is found out, never assumed or shown as unknown.** Payout frequency is looked up from the sources that state it; a fact still unfound is a problem the app keeps working on. (Owner.)
- **Design from facts known to exist.** The Bank of Canada publishes a rate for every business day: the only failure is ours (the read failed), handled and shown as a source failure, never a data case with a default. (Owner.)
- **Rates are the Bank of Canada's, for every currency it publishes**, on the day of each transaction, the previous business day's on a weekend or holiday; never a constant (the old 1.35), and no currency is taken to be CAD. (Owner; `docs/architecture.md` §6.)
- **A live mark uses the latest rate the Bank of Canada has published.** (Owner; stage 2 plan, b0b934e.)
- **External answers are read strictly against the shape the source really sends**; a mismatch is a visible error naming the source and field, never coerced. (Owner.)
- **A second brokerage is expected.** Wealthsimple sits behind one adapter so another fits beside it; nothing is built for a second one until asked (2026-09-24 above). Self-hosted comes first and always. (Owner.)
- **The watched stop stays a market sell.** A limit can fail to fill, and the stop exists to get out. (Owner; d0a95f8.)
- **Clear data does what `SPEC.md` and its confirmation say.** (Owner.)
- **No backups feature** unless the owner asks for one; it is never slipped into a design as if agreed. (Owner.)
- **Autonomous agent trading is the person's choice.** (Owner; `docs/architecture.md` §14.)

## 2026-09-22

- **A failure is always visible**: never a silent skip, a swallowed error or a log line only. (Owner.)
- **A case is correct for every input, not only the owner's book.** The owner's data never excuses a wrong case. (Owner.)

## 2026-09-20

- **The Rust server with the Svelte page is the only build going forward**; the Python and Go apps and `ledger.html` are frozen and removed at cutover. (Owner.)
- **One value changes, only its element updates**: never a screen, card, table or section redrawn for one value. (Owner; `docs/architecture.md` §13.)
- **Load and compute only what changed, when it changed**: no blind timers, no full reloads. A timer that remains is argued for in `TIMED_WAITS` (`rust/crates/server/src/tests_misc.rs`). (Owner.)

## Earlier

- **Exits rest at the broker good till cancelled**: a protective order never depends on the app being up. (Owner, 2026-09-10.)
- **Only what was asked**: no captions, tooltips, notes, helper text or `title` attributes. (Owner; `SPEC.md` §1.)
- **Per-instrument figures in the instrument's own currency; aggregates in CAD, never labelled.** (Owner; `SPEC.md` §1.)
- **Raw broker rows are never rewritten.** (Owner; `SPEC.md` §1.)
- **Nothing synthetic on a chart.** (Owner; `SPEC.md`.)
- **No visible browser window in any design**; routes are found over HTTP first. (Owner, 2026-09-13.)
