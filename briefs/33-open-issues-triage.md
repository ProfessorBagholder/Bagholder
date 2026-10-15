# Brief 33: the 106 open issues, by whether the failure can happen in use

**Asked (owner, 2026-10-15):** "how many of the so called bugs are total fabrications?"

**Method.** Three readers took every open issue against master `f89634de`. Each put an issue in one class and gave its evidence:
- **seen:** it failed in real use;
- **reachable:** an ordinary action, or ordinary data a source sends, triggers it through the real code, shown step by step;
- **hypothetical:** it needs something nobody has seen happen;
- **not a user bug:** structure, tests or documentation;
- **ruled out:** by an owner decision;
- **wrong:** the stated code fact is not true.

I then checked each issue listed as a real bug below in the code myself, and moved the reachable ones that are design questions, proposals or background cost into their own groups. The verdicts with evidence are in [33/verdicts.json](33/verdicts.json).

**Why so many are hypothetical.** Brief 15's review rated each finding by what could go wrong if it happened, not by whether it can happen in use. Stage P filed them with those ratings, and brief 24 checked only that each code fact was still true. That inflation is this review's, not the code's.

## The count

| Group | Issues | Count |
|---|---|---|
| Real bugs that can happen in use | 388, 389, 391, 393, 397, 427, 431, 452, 455, 461, 464, 468, 471, 472 | 14 |
| Seen once, its cause fixed in 2.5.7; the remaining paths not seen | 474, 516 | 2 |
| Behaves as `SPEC.md` or a plan says; a change is a spec decision | 403, 453, 459, 470 | 4 |
| Proposals and missing features (UI and UX) | 408, 409, 454, 456, 457, 458, 460, 462, 463, 465, 467 | 11 |
| Extra work in the background; nothing shown is wrong; the cost is not measured | 425, 429, 432, 434, 438, 439, 440, 444, 448, 449 | 10 |
| Hypothetical: real code, but needs something nobody has seen happen | 384, 385, 386, 387, 390, 392, 394, 401, 404, 405, 406, 407, 410, 411, 414, 415, 416, 421, 423, 426, 428, 435, 436, 442, 443, 445, 446, 447, 450, 451, 466, 469, 473, 475, 476, 477, 478, 479, 482, 509, 512, 513 | 42 |
| Not a user bug: structure, tests, documentation | 395, 396, 398, 399, 413, 422, 424, 430, 433, 437, 441, 480, 481, 483, 484, 485, 486 | 17 |
| Fixed since filed | 400 and 402 (close); 419 and 420 (close with #514, whose gate measures the depth case) | 4 |
| Ruled out by the owner (2026-10-15) | 383 | 1 |
| Unknown: needs a timing of one scheduler pass | 508 | 1 |

None is a fabrication in the strict sense: every issue states code that exists or existed when it was filed.

## The fourteen real bugs

| Issue | What happens |
|---|---|
| #388 | Clear all deletes the saved session but not the sign-in browser's profile, which keeps Wealthsimple's cookies. The next Connect can sign straight in, against the owner's decision that Clear all leaves nothing, the login included. |
| #389 | Every sign-in failure says "The Chrome window closed before a session showed up", including a browser that failed to start or crashed. |
| #391 | Editing a resting order's price in the Orders panel rounds it to the tick (12.345 becomes 12.35), and the notice repeats the typed price. |
| #393 | Orders placed in Wealthsimple's own app that have filled or been cancelled disappear from the Orders panel after a restart. They are kept only in memory. |
| #397 | A Canadian listing and a US listing with the same ticker share one stored daily history. The chart opened first fills it, the other shows those closes, and later refreshes mix the two. |
| #427 | After a reconnect, a list whose filter, sort or length changed since the page loaded shows its first-load version, then is corrected. |
| #431 | In a page left open, a holding's chart is served from the page-load memory and misses every bar since, until a reload. |
| #452 | Clearing the ticket's quantity or typing 0 silently makes it 1; a bad price silently empties. |
| #455 | An open trade with nothing sold shows its $0.00 P&L in green, as a gain. |
| #461 | A date preset (1M, 1D…) is not counted as a filter: the filter button shows no dot and Escape does not clear it, against `SPEC.md` filters-29. |
| #464 | Escape closes a trade's page but not a holding's page; on a holding page it clears the filters instead. |
| #468 | In the first year only, the benchmark comparison includes one session more than the account. The subtitle's year count is as `SPEC.md` defines it. |
| #471 | Yield on cost and its $/mo leave out every held payer without a stated schedule, and say nothing. |
| #472 | Max drawdown shows "−0.0%" in red and "−$0" when there was no drawdown. |
