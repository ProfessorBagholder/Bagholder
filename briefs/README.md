# Architecture briefs

Advisory briefs for the session building `svelte-migration`, from a separate reviewing session that writes no code. Each brief is numbered and stands until a later one replaces it. The owner's decisions recorded in a brief are the owner's; the rest is review.

Read from any checkout without switching branches:

    git fetch origin architecture-briefs
    git show origin/architecture-briefs:briefs/<file>

A session taking over the reviewing role starts with [HANDOFF.md](HANDOFF.md).

| # | Brief | Status |
|---|---|---|
| 01 | [Stage 3, and how we work from here](01-stage-3-and-the-gate.md) | Current; §2.2 replaced by 09 |
| 02 | [Verdict on the stage 3a plan: Go with changes](02-stage-3a-verdict.md) | Current; the named-fund exception replaced by 09 |
| 03 | [The instruction files](03-instructions-audit.md) | Current |
| 04 | [Options, trade marks, and a blind check of the cases](04-clean-room-case-check.md) | Current (rewritten after reading the engine) |
| 05 | [Every action is the best course for the objective](05-think-first.md) | Current |
| 06 | [The owner's decisions that change stage 3a](06-owner-decisions-3a.md) | Current |
| 07 | [Verdicts: stage 3b (Go with changes) and the trade plan (Go with one change)](07-stage-3b-and-trades-verdict.md) | Current |
| 08 | [Stage 3b as built: accepted with three fixes; managed accounts decided](08-stage-3b-built-verdict.md) | Current |
| 09 | [Verdict on the stage 3c plan: Go with changes](09-stage-3c-verdict.md) | Current |
| 10 | [Verdict on the stage 5 plan: Go with changes](10-stage-5-verdict.md) | A1 withdrawn by the owner (2026-09-27); see 13 |
| 11 | [Verdict on the statement-gaps plan: Go with changes](11-statement-gaps-verdict.md) | Current |
| 12 | [Verdict on the filters-everywhere plan: Go with changes](12-filters-everywhere-verdict.md) | Current |
| 13 | [Why screens still open empty: one read layer (#323: Stop, replaced)](13-one-read-layer.md) | Built (#324–#326); see 14 |
| 14 | [Brief 13 as built: accepted, one fix](14-brief-13-as-built.md) | Current |
| 15 | [The app against the standard](15-the-app-against-the-standard.md) | Current; full findings in [15/](15/) |
| 16 | [Verdict on the reserved-cash plan (#339): Go with changes](16-reserved-cash-verdict.md) | Superseded by 17 |
| 17 | [The reserved-cash plan resubmitted (#339): Go with changes](17-reserved-cash-resubmitted.md) | Current; source question settled by 18 |
| 18 | [The reserved-cash plan at eb7a0f05: no objection to the source, one addition](18-reserved-cash-source.md) | Current |
| 19 | [Verdict on the stage 1 plan, the money (#348): Go with changes](19-stage-money-verdict.md) | Current |
| 20 | [Verdict on the figures plan (#355): Go with changes](20-figures-category-verdict.md) | Current; item 3 corrected 2026-10-08, sources in [20/](20/) |
| 21 | [Verdict on the Stage P plan (capacity test and the process stage): Go with changes](21-stage-p-verdict.md) | Current |
| 22 | [Verdict on the Stage 3 statements plan (#503): Go with changes](22-stage-3-statements-verdict.md) | Current |
| 23 | [Debts held outside Wealthsimple, read from its net worth feature: findings and proposal](23-external-debt-from-wealthsimple.md) | Current; queries and sources in [23/](23/) ; "nothing matched by name" overruled by the owner 2026-10-19, built per 67 |
| 24 | [The 85 open issues re-checked against master d6824aa7: 83 stand, #412 closes, #442 narrows](24-issues-rechecked.md) | Corrected 2026-10-15: 17 more (#383–#399) were left out; all present; evidence in [24/](24/) |
| 25 | [The Stage 3 statements build as built (#503): accepted, one fix](25-stage-3-statements-as-built.md) | Current; verdicts in [25/](25/) |
| 26 | [Verdict on the Stage 3 record-by-change plan (#506): Go with changes](26-record-by-change-verdict.md) | Current; findings in [26/](26/) |
| 27 | [The record-by-change build as built (#507): sound, merge after the fixes](27-record-by-change-as-built.md) | Current; findings in [27/](27/) |
| 28 | [Verdict on the deep-history plan (#511), and brief 27's fixes as merged: Go with changes](28-deep-history-verdict.md) | Current |
| 29 | [The 2.5.7 hotfix (#515): merge now, four fixes right after](29-usd-accounts-hotfix.md) | Current; verdict corrected 2026-10-14 |
| 30 | [The deep-history build as built (#514): not yet, merge after the fixes](30-deep-history-as-built.md) | Current |
| 31 | [#514 at its merged head 023e0174: the merges keep both sides; brief 30 stands, not yet](31-deep-history-merged-head.md) | Superseded by 32; its master defect fixed by #518 |
| 32 | [Brief 30's fixes on #514, and #518: #518 merges; #514 after five small changes](32-deep-history-fixes.md) | Current |
| 33 | [The 106 open issues, by whether the failure can happen in use: 14 real bugs](33-open-issues-triage.md) | Current; verdicts in [33/](33/) |
| 34 | [Listings keyed by ticker: an order can go to the wrong listing, and 37 places mix listings that share a ticker](34-listings-keyed-by-ticker.md) | Current; inventories in [34/](34/) |
| 35 | [Verdict on plan #519, an order names its listing: Go with changes](35-orders-listing-plan-verdict.md) | Current |
| 36 | [The order-listing build as built (#520): not yet](36-orders-listing-as-built.md) | Superseded by 37 |
| 37 | [#520 after brief 36's changes: merge after six small changes](37-orders-listing-recheck.md) | Superseded by 38 |
| 38 | [#520 at 068fb766: merge; close #519 as superseded](38-orders-listing-merge.md) | Current |
| 39 | [#521: merge after two changes. Plan #522 (market data by listing): not yet](39-hosts-followups-and-market-data-plan.md) | Applied to #521; its #522 section superseded by 40 |
| 40 | [Plan #522: the design to build (market data by listing, filings per filer, the TSX interlisted list)](40-market-data-by-listing-design.md) | Built in #525 |
| 41 | [#522's SEDAR+ tie rule accepted with conditions; #521's CI red](41-sedar-tie-rule-and-521-ci.md) | Applied |
| 42 | [#525 as built: not yet](42-market-data-by-listing-as-built.md) | Applied |
| 43 | [#525's follow-up: a booked fill erases the venue; fix it in the name fold](43-held-venue-follow-up.md) | Applied |
| 44 | [#525 after briefs 42 and 43: not yet](44-market-data-by-listing-recheck.md) | Applied |
| 45 | [#525 after brief 44: not yet](45-market-data-by-listing-recheck-2.md) | Applied |
| 46 | [#525 after brief 45: a stale lock file and four defects](46-market-data-by-listing-recheck-3.md) | Applied |
| 47 | [#525 after brief 46: merge after four small changes](47-market-data-by-listing-merge-after.md) | Applied; checked in 49 |
| 48 | [After #525, build the page on Node 26 once it is long-term support (2026-10-28)](48-node-26.md) | Open |
| 49 | [#525 ready; merge #521 first, then bring it into #525](49-merge-521-then-525.md) | Applied; checked in 52; items 3–4 amended by 51 |
| 50 | [#523 as built: not yet; a market cache that cannot be opened still leaves no header and no update button](50-start-never-stops-as-built.md) | Applied at 60ef5e04; its change 1 premise corrected in 55 |
| 51 | [Tests touched run locally; one full CI run per head to be merged; the trial merge a8e3a84f resolves the three files right](51-verifying-on-two-cores.md) | Current; amends 49 items 3 and 4; corrected 2026-10-18 (one full CI run per head) |
| 52 | [#525 at e98e4176: the merge holds; fix the chart's timeframes (#531) in the same push, then merge](52-525-merge.md) | Applied; checked in 53 |
| 53 | [#525 at 2d887849: merge (owner OK given); then close plan #522](53-525-merge.md) | Applied (#525 merged, ddb5a3cc) |
| 54 | [Verdict on plan #535, agents read the trade history: Go with changes; same machine only](54-agents-read-plan-verdict.md) | Built in #535; released in v2.6.0 |
| 55 | [#523 at 60ef5e04: one change left; a mended store reopens only through the route the page never asks](55-start-never-stops-recheck.md) | Applied at e44c4e6c |
| 56 | [#523 at e44c4e6c: merge (owner OK given)](56-start-never-stops-merge.md) | Applied (#523 merged, af9a726a) |
| 57 | [Verdict on plan #536, a back-dated change costs what it changed (#527): Go with changes](57-backdated-change-plan-verdict.md) | Applied in #539 (change 1 corrected in 58) |
| 58 | [#527: the backdated-flat refusal accepted; list it, plan matching from a flat point next](58-backdated-flat-refusal.md) | Applied in #539 |
| 59 | [#539 ready; #540 accepted pending its run; #541 two changes](59-539-540-541.md) | Applied (#539, #540, #541 merged) |
| 60 | [Verdict on plan #548, a round trip added on a past flat day (#537): Go with changes](60-matching-from-a-flat-point-verdict.md) | Applied in #548 (change 1 withdrawn) |
| 61 | [#548 (#537) at 58ee5409: cleared](61-548-cleared.md) | Applied (#548 merged) |
| 62 | [Verdict on plan #550, a back-dated buy matched from the flat point before it (#538): Go with changes](62-continuing-from-a-cut-verdict.md) | Applied in #550 and #551 |
| 63 | [#538 closing: #549 and #550 merged, #551 cleared with the Time entry off](63-538-closing.md) | Applied (#551 merged, #538 closed) |
| 64 | [The Mac notifier never shows in the Dock; no copy of the app outlives the work that started it](64-notifier-and-leftover-servers.md) | Changes 1–2 superseded by 65 (wrong premise: a web app needs no helper); change 3 stands |
| 65 | [Notifications are the browser's (Notifications API, Service Worker, Web Push); native channels removed](65-notifications-are-the-browsers.md) | Built in #555; released in v2.6.0 |
| 66 | [Verdict on plan #555, notifications through the browser: Go with two changes](66-notifications-plan-verdict.md) | Applied in #555 |
| 67 | [Verdict on plan #557, debts held outside Wealthsimple: Go with four changes](67-outside-debts-plan-verdict.md) | Built in #557; released in v2.7.0 |
| 68 | [#557 as built: not yet; ownership of debts, and the balance sign from a real reply](68-outside-debts-as-built.md) | Applied in #557 (capture taken; figures replaced at 381d5cdc) |
| 69 | [#559, a SEDAR+ document past the first page: two changes](69-sedar-document-paging.md) | Built in #559; released in v2.7.0 |
| 70 | [Filings made readable, fast, on any device: regulators' data first, then one call to Qwen3.5-2B on the GPU](70-filings-readable-fast.md) | Current; kit and results in [70/](70/) |
| 71 | [Code carried over and never re-examined: four areas audited, defects to fix, code to delete](71-carried-over-audit.md) | Current |
| 72 | [Agents anywhere read the trade history: one agents-only address, through the person's own tunnel](72-agents-from-anywhere.md) | Planned in #572 (brief 73) |
| 73 | [Verdict on plan #572, agents anywhere: Go with seven changes](73-agents-anywhere-plan-verdict.md) | Current |
| 74 | [Verdict on plan #573, nothing of the earlier app kept: Go with four changes](74-remove-the-earlier-app-plan-verdict.md) | Current |
| 75 | [The earlier app gone, every account read one way, and the page saying when the numbers match](75-the-earlier-app-gone.md) | Shipped as 2.10.0 to 2.10.2 and withdrawn (wrong figures on a real book); 2.10.3 restores 2.9.0's build; the migrations stand; the record in [75/](75/). Next: the recorded capture set |
| 76 | [The shapes the broker's feed has, in the repository's own made-up Wealthsimple](76-the-shapes-the-feed-has.md) | Built 2026-10-25: PR 1 #612 with the four fixes the shapes found (#613 rule 16, #614 the base before an account's first month, #615 a closed account's statements, #616 the sides paid by day; 2.10.7), PR 2 #617 (the rebuild and migration tests), PR 3 #618 (capacity on the shaped books). Conditions every pull change. |
| 77 | [How a change to the book's reading is proven and shipped](77-how-a-reading-change-is-proven.md) | Sent to the CTO 2026-10-26: recorded-only fixtures with tests, reproduce then fix, golden figures, a re-derivation refused if it makes any account worse, one reading change per release; the first three changes named |
| 78 | [A link to a running copy, for the people developing the app](78-a-link-to-a-running-copy.md) | Sent to the CTO 2026-10-26: one generic app change, an opt-in `BAGHOLDER_HOSTS` list of exact names the server answers to (off by default, peer check unchanged, tests for a listed name, a near miss, an empty list, a bad entry; MINOR); the links themselves as tooling outside the repo (a preview per branch behind the owner's GitHub sign-in at a Cloudflare Access gate on a named tunnel, a fixed name for the main copy, demo links later from the container on made-up data, cleanup on every exit); the first live run of the app behind a link as the proof. Supersedes the 2026-10-04 "remote access: not now". §1 shipped as 2.11.0. **Amended 2026-10-27 (the owner's decision):** previews go over Tailscale Funnel on the server's Tailscale account, with no domain and no login, made-up books only; the login gate (Cloudflare Access, a domain) is only for the main copy's fixed link, later. |
| 79 | [Every amount as Wealthsimple states it](79-amounts-as-wealthsimple-states-them.md) | Sent to the CTO 2026-10-27 on the owner's ruling ("it should be completely impossible for Bagholder to somehow have the wrong amounts vs WS"): cash, units, book value, account value, buying power and past days are Wealthsimple's, never rebuilt from the history; the history feeds the journal only. Removes Bagholder's own cash, the broker check and its comparing header lines, the gate, the monthly statements whole (about 2,770 production lines) and `CashByDay`. One read-only capture first (which read states units now; book value currency; the cash Wealthsimple's app shows; past days; coins). After the agents build, in the coin units change's place. Supersedes brief 77 §7's open items and §4. |
| 80 | [#639 at e95e5190: not cleared, the changes](80-639-verdict.md) | Sent to the CTO 2026-10-27: 57 confirmed findings (seven areas, each checked by independent refuters). Must fix: the page's status writes the book while holding `app.state` (deadlocks a CSV import; status loops on a non-WAL book) and a stop sent to the tunnel guard leaves the tunnel running (reproduced here; on macOS the Funnel stays public). Then: strangers' work and storage bounded before sign-in (refusals counted in memory, metadata clients stored only at Allow, answered requests end, the fetch's limits on the decoded body), redirect hosts, the gate's names, tunnels restarting with Agents on, one MCP service per path, keys never made by Copy steps, and the plan's missing tests. Re-reviewed at d6349a88: 47 of 49 plants caught; must fix: a copy of the book never starts the person's tunnel, a stopped tunnel stays stopped, the JSON API cuts arguments, hex/number last labels refused; five tests to hold their rule; small items. Round 3 at 750eff32 (63044f54), then **cleared at 294fa7ba as 2.13.0** (all round-3 plants caught). |
| 81 | [Agents, 2.14.0: native clients, and sign-in by a typed code](81-agents-native-clients-and-the-typed-code.md) | Sent to the CTO 2026-10-28: brief 73 change 3 withdrawn (Grok Bot is a native client with a loopback redirect, which the MCP spec allows): loopback redirects accepted per RFC 8252 §7.3, matched exactly but for the port, the name marked unchecked, an end-to-end test; sign-in by a code typed on the public page: one code per request, 10 minutes, 3 wrong entries end it, 5 wrong in all pause sign-in until the person resumes; the panel rebuilt to the owner's reviewed proposal, Remove, screenshots judged. **#650 cleared at 3e559b8f as 2.14.0** (plus: Bagholder installs ngrok/cloudflared from the makers' hosts; Cloudflare removal deletes only Bagholder's record and tunnel after confirmation; agents read income, net worth, exposure and chequing/card transactions, named on the sign-in page; keys bound to one tunnel). |
| 82 | [Hardening Bagholder's container on the home server (a handoff)](82-docker-host-hardening-handoff.md) | Written 2026-10-31 for the owner's compose-editing session: secrets first, a network of Bagholder's own, Traefik's allow-list, compose hardening in a tested order (mem_limit 6g for the in-container summarizer; cap_drop expected fine; read_only/non-root copy-only, needs an image change), DOCKER-USER rules only with the owner's rules in hand, a pinned tag, and further low-hanging fruit (read-only Docker socket + exposedByDefault=false, log limits, healthcheck, daemon defaults, backups, clock). §9 lists the three Bagholder image changes to brief the CTO. |
