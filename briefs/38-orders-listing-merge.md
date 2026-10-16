# Brief 38: #520 at `068fb766`: merge; close #519 as superseded

**Reviewed:** commit `068fb766` against brief 37's six changes. CI is green on all eight checks. Each change was undone in a throwaway copy, and its test was run.

## Verdict

**Merge #520. Close #519 rather than merging it first.** #520 carries the plan file itself, with the build's verification filled in. The two branches share no history, so merging #519 first would make #520 conflict on that file.

## Each of brief 37's changes, undone

| Change undone | Caught by |
|---|---|
| A page with no venue takes the server's exchange and id, and Buy uses it | `signedin.listingorders.spec.ts:116`: "Buy UBER" shown on `UBER@` |
| The jump to a holding for any address | the same test, on `TD@` |
| A summary with no currency or symbol accepted, or the page's currency used | `an_order_is_recorded_in_wealthsimples_words_for_its_listing_never_the_pages` |
| The preview keeps the page's currency | `the_preview_is_in_the_listings_currency_never_the_pages` |
| An id recorded whatever its kind or currency | `an_id_is_recorded_only_for_a_share_in_the_instruments_own_currency` (both cases) |
| The late-answer guard counts tickets, not requests | Vitest: of two listings asked for, only the later click's opens |
| Max matched by security only | Vitest: a Sell's Max is the shares of the holding it was opened from |
| A Wealthsimple host hard-coded in the newer client or its token refresh | `every_wealthsimple_address_comes_from_the_one_place_a_stand_in_redirects` |

Baseline counts:

| Suite | Result |
|---|---|
| `npm run check` | 0 errors |
| `npm test` | 148 |
| signed-in e2e | 5 |
| `ticket` and `orderspanel` e2e | 54 |
| server order and boundary tests | 75 |

**A run with the offline switch removed.** The signed-in server was run without `BAGHOLDER_OFFLINE`, with every outbound connection traced and logged:
- Every Wealthsimple call went to the stand-in on loopback: the token refresh, the token check and the sync's reads.
- No connection reached any Wealthsimple host.

## For the next PR, not this one

- **The host test is a source scan.** Brief 37 asked for a test against a running server. The run above shows the running server behaves.
  - The scan exempts all of `ws/src/session.rs`. Narrow the exemption to the function that reads the base.
  - The e2e itself would not notice a leak; with a host hard-coded, it still passed. Add the running-server check to the e2e: a proxy that refuses and records, asserting it saw nothing.
- **The e2e leaves its temporary homes behind until the next run starts**, one of them with `session.json`. Remove them when a run ends too.
- **The preview test's data has no USD rate.** It checks only the "waiting on the rate" outcome. Give it a rate so a converted figure is checked as well.
