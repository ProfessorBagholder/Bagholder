# Brief 41: #522's SEDAR+ tie rule accepted with conditions; #521 at `6b1664ef` is red

## #521 at `6b1664ef`: fix CI, then merge

**The two changes hold.** I read the diff against `f03a9ad2`.
- Each real address is a constant inside the one function that reads `BAGHOLDER_WS_BASE`.
- The scan allows those functions only. Its planted case catches a top-level constant and a host inside any other function.
- The e2e compares the calls each client names with the calls the stand-in answered, and removes both logs at the end.

**CI's `rust` job fails.** The failing test is `tests_misc::test_the_terminal_says_only_what_the_person_needs` (`tests_misc.rs:763`). `named_call` prints from `wealthsimple/src/hosts.rs` and from a second copy in `ws/src/session.rs`. The terminal rule allows the request tracer to print only from `net/src/client.rs` and `wealthsimple/src/client.rs`.

**Required:**
- **One function, `bagholder_net::client::named_call`**, beside the `outbound` line it belongs with. Both clients call it, and both copies go.
- **`wealthsimple/src/client.rs` comes off `allowed_elsewhere`.** It no longer prints, and the list should say only what is true.

## #522: the SEDAR+ name rules and the tie rule

**Checked:** the recorded answers in `rust/crates/market/tests/replies/sedar-tsx60/`, and NI 51-102 from the BCSC's consolidated text of 2026-09-22.
- **s. 4.2:** annual financial statements are due by the 90th day after year end, or the 120th for a venture issuer.
- **s. 4.4:** an interim report is due by the 45th day after each interim period, or the 60th.

A reporting issuer meeting its obligations therefore files at least every quarter. A profile with no filing in 12 months + 120 days is not one.

**The name rules are accepted, on three conditions:**
1. **The same normalization is applied to both names.** It can only produce more ties, and every tie goes to the tie rule, never to the order SEDAR+ answered in.
2. **"Operating name: …" notes are dropped as well**, as a stated and tested rule. RBC's SEDAR+ name carries "Operating name: RBC".
3. **A short query can fill SEDAR+'s result page without an exact match.** That is a miss, said as one, never a pick.

**The tie rule is accepted, on five conditions:**
1. **It only narrows** a set of exact-name matches. It never adds a candidate.
2. **Exactly one profile must remain.** If none or several remain, nothing is picked, and the page says so.
3. **The bound is 12 months + 120 days before the check**, the loosest the two deadlines above allow.
4. **It is decided again** whenever the listing's filer is re-checked (`listing_filers.checked_at`), never kept for good.
5. **A rule test covers it:** generated tie sets with newest filings on either side of the bound. Keep the TELUS answer as a recorded fixture beside it. SPEC market-data-17 gains the clause.
