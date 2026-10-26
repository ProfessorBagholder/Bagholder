# Brief 77: How a change to the book's reading is proven and shipped

For every book and every copy. Line numbers are from master at 692d1b1d (2.10.10). Nothing here asks the owner to run anything; the one thing shown on a page is marked as the owner's decision.

## 0. Why

From 2.10.0 to 2.10.9 every release that changed how the book is read from Wealthsimple was proven on generated books and passed, and six of them were wrong on real books. The proofs were circular: the stand-in stated rows the CTO wrote, not Wealthsimple's, so a rule could be right about the stand-in and wrong about the feed, and nothing between a merge and a user's page could tell the difference. Each wrong release cost a full cycle of hours, and the fix for each was a guess at the owner's symptoms. The owner on 2026-10-26: no user's numbers are needed to debug anything if the shapes are known; every case must be accounted for; hours per iteration are unacceptable; and the app must be designed for every book.

The statement shapes are now recorded (2026-10-26, every form Wealthsimple writes, in made-up form). This brief sets the four mechanisms that make a reading change provable before it ships, and names the first three changes to go under them.

## 1. Fixtures are recorded, never typed

- Every row the stand-in (`rust/crates/wealthsimple/src/generated.rs`) states is a recorded reply of its kind with its fields set; its text is a recorded form with the made-up figures substituted. A test fails if the stand-in states any row the parsers would not read as a recorded form, and a second test fails if any form in the parser's table (`broker/src/codes.rs`) has no recorded example.
- Any row the app meets that is not a recorded form is reported as unread, never guessed at; its form is then recorded and the list grows. The list is complete by construction, not by promise.
- Sequences are shapes too: an assignment (the feed's `OPTIONS_ASSIGN`, the statement's `ASSIGN` row at cash 0 and its plain "Sold N shares" row at the strike one to four days later), a conversion booked from a statement before the feed states its paid side, a pending card spend. Each lives in the stand-in as a named shape, from the recorded forms.

## 2. Reproduce, then fix

A change to how the book is read starts from a test over the stand-in that fails on the current code and names the shape. The PR shows the test failing before and passing after. No diagnosis of a user's symptoms is accepted as the cause of anything; the stand-in reproduction is.

## 3. The figures are golden

- `rust/crates/server/tests/golden/figures.json` holds the digest of every account's cash per currency and units per symbol, and the header's lines, for the stand-in at twelve seeds with every shape on, built by the pull as a user's book is (the digest `figures_digest` in `capacity.rs` already computes, extended to the seeds and shapes).
- A test compares the build's digest with the file. A reading change that moves any figure updates the file in the same PR, so the diff of that file is the list of every figure the change moves, reviewable line by line and named in the PR. A figure that moves without being in the diff fails CI. This is the differential test old build against new, with no second build: the file is the old build's result.

## 4. A re-derivation that disagrees with the broker is not applied

When a rule set bump re-derives a book's records (as 15, 16 and 17 did), the book runs the re-derivation inside one transaction (`Book::atomically`), compares each account's cash less holds and units with the broker's last stated figures before and after, and commits only if no account's agreement got worse; otherwise it rolls back, the previous derivation stands, and the book notes which rule set was refused. The next pull tries again. This is the check the header's broker check already makes (`engine/src/equity.rs`), applied before the figures change instead of after. **Whether the page says anything when a rule set is refused is the owner's decision**; the brief requires only that wrong figures never replace right ones.

## 5. One reading change per release

Each change to the pull, the mappings or the statements walk ships alone, versioned, with its shape, its failing-then-passing test and its figures diff named in the PR. A release carries nothing else that reads the broker.

## 6. The canary, if the owner says yes

The CTO proposed running each release candidate on a read-only copy of a real book on the Mac, offline, and blocking the release if any header line is new or worse. It is in addition to §1 to §4, never in their place, and it reads a copy, never the live app.

## 7. The first three changes under this brief

1. A pending card spend (`SPEND/PREPAID`, not final) maps to a hold, as a pending withdrawal does (`mapping.rs`, `HoldKind::Withdrawal`); the broker check compares the book's cash less holds with the stated cash, which is net of it (`equity.rs:152`). No derived transaction, no change to the comparison.
2. Before the walk, beside `restore_withdrawn`: a live statement-booked row that a feed transaction of the same account and currency now states, by the walk's own pairing (`match_rows`: cash equal, within the posting window, one to one, kind not compared), is withdrawn wherever it stands. The walk's own withdraw step runs only once a base month is found, and duplicates leave no month reconciling, so it never ran on them.
3. The June line: a statement's plain "Sold N shares" row within four days after an `ASSIGN` row of the same symbol, for the contract's shares at the strike, is the assignment's delivery, paired with the feed's assignment leg; never booked as a sale and never a stop.

Each has its stand-in test first, its own PR and release, and its figures diff.

## 8. Alongside, not under this brief

The test and CI speed-ups the owner's handoff of 2026-10-26 lists (a lightly optimized dev profile, test programs side by side, the capacity cases in the capacity job, named tests while iterating): build setup only, one PR, timings before and after.

## 9. Not included

- The agents-anywhere build (plan #572) and brief 75's PR 4 and PR 5: after the three changes in §7.
- Anything shown on a page beyond the owner's decision in §4.
- Any capture from anyone's account beyond the one made on 2026-10-26.
