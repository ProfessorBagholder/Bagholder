# Brief 77: How a change to the book's reading is proven and shipped

For every book and every copy. Line numbers are from master at 692d1b1d (2.10.10). Nothing here asks the owner to run anything; the one thing that could be shown on a page is marked as the owner's decision.

## 0. Why

Of the releases 2.10.0 to 2.10.9, five were wrong on a real book (2.10.0, 2.10.2, 2.10.3, 2.10.4, 2.10.9; `docs/decisions.md`), each after passing its tests. The proofs were circular: the stand-in stated rows the CTO wrote, not Wealthsimple's, so a rule could be right about the stand-in and wrong about the feed, and nothing between a merge and a user's page could tell the difference. Each wrong release cost a full cycle of hours, and each fix was a guess at the owner's symptoms. The owner, 2026-10-26: no user's numbers are needed to debug anything if the shapes are known; every case must be accounted for; hours per iteration are unacceptable; the app must be designed for every book.

The statement shapes are now recorded (2026-10-26, every form Wealthsimple writes, in made-up form). This brief sets four mechanisms that make a reading change provable before it ships, and names the first three changes to go under them. Where a mechanism names code, what exists is cited and what is new is said to be new.

## 1. Fixtures are recorded, never typed

- Every row the stand-in (`rust/crates/wealthsimple/src/generated.rs`) states is a recorded reply of its kind with its fields set; its text is a recorded form with the made-up figures substituted. The forms live in the parser's table (`broker/src/codes.rs:67-112`, each code with the row that showed it).
- Two tests, both new. The first: for every statement row the stand-in states with `Shape::Statements` on, `codes::kind(code)` is `Some`, and for a fill row `codes::fill_of` (or the contract parser) returns the symbol and units the stand-in moved. The second walks the table's codes and fails for any code the stand-in with every shape on never states. With them, `statements.rs:634-639` reports a fill row it cannot read instead of skipping it.
- Any row the app meets that is not a recorded form is reported as unread, never guessed at; its form is then recorded and the list grows. The list is complete by construction.
- Sequences are shapes too: an assignment (the feed's `OPTIONS_ASSIGN`, mapped at `mapping.rs:594-597`; the statement's `ASSIGN` row at cash 0 and its plain "Sold N shares" row at the strike one to four days later), a conversion booked from a statement before the feed states its paid side, a pending card spend. Each is a named shape in the stand-in, from the recorded forms. `SHAPES` holds six of the nine variants today; the fixtures PR restores the three fill shapes with the recorded forms.

## 2. Reproduce, then fix

A change to how the book is read starts from a test over the stand-in that fails on the current code and names the shape. The PR shows the test failing before and passing after. No diagnosis of a user's symptoms is accepted as the cause of anything; the stand-in reproduction is.

## 3. The figures are golden

- New: a digest of every account's cash per currency and units per symbol, and the header's lines, for the stand-in at twelve seeds with every shape on, built by the pull as a user's book is (the stand-in is buildable from the server crate, `capacity.rs:33`, `:455`). It sits beside, not inside, the test helper `figures_digest` (`capacity.rs:1352`), which digests trades and positions only.
- New: `rust/crates/server/tests/golden/figures.json` holds it, blessed and compared as `tests_routes_golden.rs:226-248` does for the routes, except that a missing file fails the test instead of passing on an empty default.
- A reading change that moves any figure updates the file in the same PR, so the diff of that file is the list of every figure the change moves, reviewable line by line. A figure that moves without being in the diff fails CI. This is the differential test of the old build against the new, with no second build: the file is the old build's result.

## 4. A re-derivation that disagrees with the broker is not applied

- What exists: a mapping's version (`book/src/mapping.rs:33-34`; WealthsimpleMapping at 17, `mapping.rs:109-111`) and `Book::rederive` (`records.rs:457-477`), which re-derives every live record below that version inside `Book::atomically` (`lib.rs:152`: one IMMEDIATE transaction, nested calls join, an error rolls back). It runs from the server at open and at the first pass (`figures.rs:351`, `:394`, `:417`), not from the pull. The broker's last stated cash, holds and units per account are in the book (`book/src/statements.rs:42-56`, `stated`, `stated_latest`), and the engine's broker check compares the book's cash less sized holds and its units with them (`equity.rs:130-214`).
- New, in the server beside `rederive_all`, inside one `book.atomically`: the broker check before, the re-derivation, the check after; "worse" defined over the engine's `Difference` lists per account (a currency or symbol that agreed, or was within dust, and no longer does; pending differences not counted); on worse, return a sentinel that rolls the transaction back and is an Ok outcome, so the engine builds from the previous derivation; a settings key naming the refused version and the account; and a retry after a pull stores new statements or at the next open (today the version query would retry only at process start). **Whether the page says anything when a version is refused is the owner's decision.** The brief requires only that wrong figures never replace right ones.

## 5. One reading change per release

Each change to the pull, the mappings or the statements walk ships alone, versioned, with its shape, its failing-then-passing test and its figures diff named in the PR. A release carries nothing else that reads the broker.

## 6. The canary, if the owner says yes

The CTO proposed running each release candidate on a read-only copy of a real book on the Mac, offline, and blocking the release if any header line is new or worse. It is in addition to §1 to §4, never in their place, and reads a copy, never the live app.

## 7. The first three changes under this brief

1. **A pending card spend is a sized hold.** The feed kind (`SPEND`/`PREPAID`, recorded 2026-10-26) is not on master; its arm is written from the recorded row. A pending withdrawal maps to `HoldKind::Withdrawal` (`mapping.rs:359`, pending per `adapter.rs:329`), but that kind has no size and makes the currency's check wait (`equity.rs:237`, `:159`, `:172`); only a sized hold is subtracted before the comparison (`equity.rs:236`, `:167-169`). A pending spend states its amount and is final in size, so it maps to a sized hold: either `HoldKind::Buy`'s sizing applied to a card spend, or a new sized kind, which extends the `CHECK` lists at `v40.sql:456` and `:542` and is therefore a migration; the CTO proposes which, with the recorded row. The pull already reads a pending row again until final and removes it once posted again under another id (`pull.rs:156`, `:553-556`, `:334-336`). No derived transaction.
2. **A statement-booked row the feed now states is withdrawn before the walk,** beside `restore_withdrawn` (`statements.rs:190-192`), over every kept month: for each kept statement row that a feed transaction fits by the walk's own pairing (`match_rows`, `:1262-1294`: the same signed cash, dated 0 to 7 days after the feed's day first, then within 31 days, one to one, the feed's transaction before a booked one, kind only breaking a tie), a live record booked from that same row is withdrawn. The walk's own withdraw step (`:369-372`) runs only inside the loop over currencies with a base month (`:336-344`), and duplicates leave no month reconciling, so it never ran on them.
3. **The June line, reproduced (2026-10-26, the recorded forms over twelve seeds on 2.10.10's walk):** the six shapes agree everywhere, and so do the six with the assignment sequence, which is not a stop today; the three stated-otherwise shapes (a fee folded into the statement's cash, a purchase paid in CAD at the rate the statement states, a purchase the feed omits) stop at every seed with "a buy only the statement states, which is not booked from it". So the third change is #620's rule rebuilt on the recorded forms: a fill is one fill on both sides by its side, symbol or contract, units and day, never its cash; the statement's cash and units supersede the feed's, the fee counted once; a fill the feed omits is booked under the security the account holds by that name; and the `ASSIGN` sequence (the feed's `OPTIONS_ASSIGN`, `mapping.rs:594-597`; the statement's `ASSIGN` row at cash 0 and its plain "Sold N shares" row at the strike) is held never to book that row as a sale. No remainder row. The stated-otherwise shapes join `SHAPES` with it.

Each has its stand-in test first, its own PR and release, and its figures diff.

**7.3 withdrawn (2026-10-26, late).** The recorded forms show no fill row stated at other cash than the feed's: the ".TO" rows pair by cash (the feed names the Canadian listing "SYM.TO", the statement the bare symbol, the cash equal); a coin's 2024 form ("Fee charged") states the feed's amount; its 2025 form states the fill before the fee with its own FEE row, and the walk's combined pass pairs them; the two "omitted" coin sales are limit orders the feed dates at placement, 9 to 12 days before the statement's execution day, which the walk's 31-day pass pairs; and the June line was the doubled conversions 2.10.12 withdrew. `FeeFolded`, `FeedOmits` and `FxFunded` were typed situations with no recorded source and are removed with it. The third change is instead a defect of 2.10.8's units step found by the same forms: Wealthsimple can state a US-listed fill as a currency row of 0.0000 shares (with the FX rate) and a symbol row of N shares at no cash; the step read the currency row alone, found its units differ from the feed's, and superseded the feed's fill with one of no units (the recording had hidden it: every number became `N`). The rule: a row stating no units is no fill the step reads, so the feed's fill stands; a live statement fill record whose row now reads as no fill, or as the feed's units, is withdrawn by the step at the next pull (`Book::withdraw`, the feed's record live again), the step's own rule, not under the gate (whose `differing` guard is cash-only, and these records leave cash agreeing). Before it, a shapes PR with no reading change: `Interlisted`, `Coin2025`, `LimitOrder`, `SplitFill` from the recorded forms, the form notation keeping a zero as `0`, and the golden digesting units bought and sold per symbol (gross), since net units hide a vanished round trip. Open from the recorded sides, each its own change if a defect: whether the 2025 coin fills' feed units equal the statement's (the step pairs by cash and misses a fill stated before its fee); whether a limit order's feed row carries its execution date (the fill's date is then the mapping's change).

## 8. Alongside, not under this brief

The test and CI speed-ups the owner's handoff of 2026-10-26 lists (a lightly optimized dev profile, test programs side by side, the capacity cases in the capacity job, named tests while iterating): build setup only, one PR, timings before and after.

## 9. Not included

- The agents-anywhere build (plan #572) and brief 75's PR 4 and PR 5: after the three changes in §7.
- Anything shown on a page beyond the owner's decision in §4.
- Any capture from anyone's account beyond the one made on 2026-10-26.
