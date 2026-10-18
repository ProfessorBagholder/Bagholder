# Brief 56: #523 at `e44c4e6c`: merge (the owner's OK given)

## Verdict

**Merge #523.** Brief 55's change is in:
- the reopen runs in `status::status`, which the page's stream and `/api/status` both read;
- with the figures open, it costs one `OnceLock` check;
- `open_figures` holds `single_flight`.

CI run 39172519586 is green on all seven jobs, and capacity run 39172519585 is green, both on `e44c4e6c`. The owner gave the OK on 2026-10-18 ("unless there is some issue then yes"), and nothing is left for them. #523 closes #516.

## What was checked

- **The diff `60ef5e04..e44c4e6c`:** `status.rs`, `start_failures.rs` and `startfailure.spec.ts`.
- **The planted reverse, run here** under the build lock, on the warm build:

| Change undone | Test that fails |
|---|---|
| the reopen left in `answer()` only | `a_store_mended_while_the_server_runs_opens_at_the_next_header_read` (it now reads the header as the stream's first `status` snapshot). Restored, all six `start_failures` tests pass |

- **The CTO's plant**, the same reverse, also fails the e2e `startfailure.spec.ts`. It mends the cache while the server runs, reloads, and needs a dashboard figure to show.

## After the merge

1. #516 closes with #523.
2. Next, the bug list:
   - #527 (the engine re-derives a back-dated holding, listed in `OVER_BUDGET`) and #524 (fund-of-funds sectors) first, as figures defects;
   - then the page defects: #528, #529, #533 and #534;
   - then the test races: #530 and #531's remains, if any.

   Brief 33's triage is checked against master before it is relied on.
