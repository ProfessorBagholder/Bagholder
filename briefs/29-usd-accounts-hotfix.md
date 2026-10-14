# Brief 29: the 2.5.7 hotfix (#515, `usd-accounts-2.5.7` at `8a11daed`): merge after four small fixes

**Reviewed:** the whole diff against master `e340e0d7` (2.5.6). Read: `server/src/{engine_inputs.rs, figures.rs (open, ensure_built, rebuild, state_zone, pulled, broker_changed), due.rs (run, settle, pass), docs.rs, events.rs (watch), http/stream.rs, status.rs, main.rs, wire/build.rs (accounts, book_doc), orders/ticket.rs}`, `engine/src/{equity.rs, fx.rs, scope.rs, input.rs}`, `engine/tests/stated_currency.rs`, `wealthsimple/src/{client.rs, read.rs}`, `wealthsimple/graphql/FetchAllAccounts.graphql`, `web/src/lib/{ticket/ticket.svelte.ts, Portfolio.svelte}`, and the `SPEC.md` changes. I merged `deep-history` (#514, `1a550d63`) into this head on a throwaway worktree to answer the rebase question. CI at the time of writing: web, audit, rust-windows and capacity green; rust, rust-macos, browser and container still running.

## Verdict

Merge after four small fixes, all in this PR and none widening it. The cause is fixed in the right place. The broker's statements keep the currency they are stated in, every sum puts each account in CAD, and a figures build that fails no longer stops the server. PATCH is the right bump.

## The three questions

1. **The CAD return with the rate's move in it is right.** Aggregates are in CAD (the owner's rule), and a return reported in one currency carries that currency's move. GIPS 2020 for Firms, 4.A.12: "All required and recommended information in the GIPS Composite Report must be presented in the same currency." It is also what Wealthsimple already sends. The history is asked in CAD (`wealthsimple/src/client.rs:288`), so a US-dollar account's days arrive in CAD at Wealthsimple's rate, with the rate's move in them. Buying power is asked in CAD too (`client.rs:281`), and a read in any other currency is refused (`read.rs:195`). The only figure the read gets in US dollars is the value now: `currentCombined` takes no currency argument in Wealthsimple's own document (`FetchAllAccounts.graphql:22`). That is the row 2.5.6 stopped on ("states what it is worth in USD"); 2.5.6 checks the days first. So the equity series' conversion is a safeguard today's reads do not reach (fixes 3 and 4).

2. **Trying again when something changes is the right retry; as built, no change triggers it.** A build that failed on the book fails the same way until the book changes, so retrying on a change is right, and a timer or backoff would only repeat the same failure. Building under the engine's write lock is what `rebuild` and `state_zone` already do. But in the one state `ensure_built` exists for, nothing that changes the book triggers a pass (fix 1).

3. **No rebase.** Merge #515 first. Then merge master into `deep-history` with a merge commit; a pushed branch is never rebased. I ran that merge. One file conflicts, `server/src/figures.rs`, in two hunks:
   - `pulled`: #514 wraps the broker reads in a closure, and #515 changed `broker_account`'s signature. Keep #514's closure with #515's call, `broker_account(&book.stated(*a)…)`, which takes no account and no `?`.
   - the tests: keep both sets.

   Everything else merges cleanly. The merged head must then build and pass both suites. Brief 30's verdict on #514 is for that head.

## Required changes

1. **A change to the book must trigger the retry, and a build that succeeds must clear the header.** After a failed pass, `settle` returns no deadline (`due.rs:94`), so the scheduler parks until something wakes it. With no engine, `pulled` returns at `figures.rs:602`, before its wake at :624, so a sync that changes the book never retries. A page connecting does build the figures, through `state_zone`'s no-engine branch (`figures.rs:543–548`), but that branch does not wake the scheduler either. The header then keeps "The figures could not be brought up to date: …" while the figures are built, and no read runs, until a page opens a priced view (`docs.rs:193`) or the next pull lands. Required:
   - `pulled` wakes the scheduler when there is no engine, since the change is what can make the build succeed.
   - `state_zone` wakes it after installing, as `ensure_built` and `rebuild` do.
   - A test: start on a book that fails, mend the row, then `record_changed`; the next pass builds and `figures_error` is empty. The same test through `state_zone`.

   The cost is at most one failed build per pull, and only while the header says the figures failed.

2. **The ticket's position size must not be worked over only the accounts that converted.** `accounts()` gives an account no `nav` when its live rate is missing (`wire/build.rs:407`, `.ok()`), and `nav_total` adds only the accounts that have one (:455–456). With the US rate missing, the ticket shows "x% of net asset value" over a smaller total, as a number. `SPEC.md` order-ticket-66 says "over the net liquidation value of every open account". Meanwhile the NAV tile, built from the same values, waits on the rate (`fig_sum` in `scope.rs`). Required: `nav_total` waits on the rate whenever any account's value cannot be put in CAD, with a test on the `stated_currency` case without its rate.

3. **A change in the currency the deposits are stated in must form no flow.** The other-currency branch of `account_equity` (`equity.rs:65–68`) converts the later total at its own day's rate and the earlier total at the earlier day's rate. The rate's move on the whole deposit base then counts as money moved in or out. Converting both at one rate is wrong too, since a total restated in another currency is not a deposit. No test reaches the branch. Required: when the two days are stated in different currencies, the day forms no flow, as a day with no stated deposits forms none, and the gaps say so. Add a test.

4. **`SPEC.md` must describe the read as it is.** equity-and-returns-1 says a US-dollar account "states its value and deposits in US dollars". The history is asked in CAD, so Wealthsimple states it in CAD. portfolio-8 converts a buying power stated in another currency, but the read refuses one. Required:
   - equity-and-returns-1 and portfolio-34: the history is asked in CAD, and a day stated in another currency anyway is put in CAD at that day's rate.
   - portfolio-8: drop the addition, or state the refusal.
   - portfolio-5 (the value now, at the live rate) is right as written.

## Outside this PR

- **Installs stuck on 2.5.6 cannot take 2.5.7 from inside the app.** The update is a button in the page header, and the update check starts at `main.rs:241`, after `Figures::open` at :155, where 2.5.6 exits. Each such install has to get 2.5.7 from outside the app: the release archive, or a fresh image pull for Docker.
- **The same kind of crash loop is still possible.** `Figures::open` still stops the server when `rederive_all` fails (`figures.rs:291`), and `main` stops it when `carry::from_old_store` fails (`main.rs:159–162`). Both depend on the data, and a service manager restarts the server into the same failure, with no page and so no update button. This PR's own reasoning ("it never stops the server, which a service manager would only start again into the same failure") applies to both. File an issue: no step at start that depends on the data stops the server. Each failure is said in the header as the figures' build failure now is, and the page, with its update button, stays up.

## Accepted as is

- The broker's statements keep the currency they are stated in (`input.rs`), and `broker_account` no longer refuses one.
- The value now and buying power are converted at the live rate, and a past day's value at that day's rate.
- `fig_sum` waits on the first failure instead of adding a value as though it were CAD.
- The start goes on without figures and logs why. The first pass says it in the header and keeps it until a pass succeeds. Two failures of the same build come before it is said.
- The two figures tests and `stated_currency.rs` hold what they claim.
- `live_to_cad` on the ticket's buying power is harmless: the read is always CAD.
