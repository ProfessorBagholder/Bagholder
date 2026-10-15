# Brief 37: #520 at `e478ef8a`, after brief 36's changes: merge after six small changes

**Reviewed:** commit `e478ef8a` against brief 36. Two readers took the server and the page. Each ran the suites, undid each change in a throwaway copy, and tried to break what the tests leave open. I checked the findings below in the code. Copies, build folders and the e2e's temporary homes are deleted.

What was run:
- **Server:** 91 order, golden and type tests; the market and venue tests.
- **Page:** `npm run check`; `npm test` (146 tests); the signed-in project (5); `ticket` and `orderspanel` (54).
- **Full e2e:** 311 of 312. The one failure is `marketcards.spec.ts:617`, whose popup to `https://example.com/a` cannot load in the sandbox. It touches no listing or ticker code.

## Verdict

Merge after six small changes, all tests or fallbacks; none changes the design.

Brief 36's two serious defects are fixed and hold:
- **NYSE Arca and NYSE American listings resolve.** All 40 shares and ETFs in Wealthsimple's recorded replies were each put beside the other listings of their ticker. Every one of the 133 directory rows tried found its own listing. That includes VTI, GLDM, MSTY and CTM from Nasdaq's `PSE` and `AMEX` codes, and TSX against TSX-V both ways. US class shares (BRK.A and BRK.B) stay apart. A bare `BRK`, `BRK/B` or `BRK-B` is refused, never answered with another class.
- **A listing page with no venue offers no Buy** and keeps its address. It takes no exchange or security id from the server's answer. `listing:UBER@`, `TD@` and `T@` were checked in the browser.

Treating the US as one market is right. US listing markets allocate ticker symbols under one national plan, which addresses how "to preclude duplicative symbols from being selected and reserved" (SEC Release 34-56037, 2007). Should two US results of one ticker ever appear, the count rule refuses rather than picks.

Also in and held:
- **The order record:** the summary by id (`listing_by_id`) is caught when undone.
- **The request types:** `SecurityId` cannot be empty, and `Ticket`, `PreviewRequest` and `ListingOf` have no `Default` outside tests. No "no listing" branch is left.
- **Sell defaults** come from the holding id alone.
- **A late listing answer** no longer replaces a ticket opened since.
- **The demo book** holds both TD listings for real (TSX in the TFSA, NYSE in the RRSP). The signed-in e2e places dry orders against them, asserting the account, quantity, previews' ids and the order's listing.
- **Smaller items:** an id another instrument holds is opened, not recorded; refusals name venues in words; SPEC filters-21 and -24 updated.

## Required changes

1. **The e2e test for a listing page with no venue holds nothing.**
   - **What happens:** `signedin.listingorders.spec.ts:116–125` asserts no Buy and the same address before `/api/listing` has answered. Two plants pass every test. Undoing all of brief 36 item 2's page change shows "Buy UBER" on `listing:UBER@`. Jumping to a holding's page for any address sends `TD@` to the NYSE TD holding.
   - **Required:**
     - Wait for the `/api/listing` answer before asserting.
     - Add a ticker the book holds (`TD@`), and assert no jump and no `/api/order/listing` sent.

2. **The order and the preview still take the page's currency where Wealthsimple states none.** At `ticket.rs:881` the currency falls back to `t.currency`, and `preview.rs:242–245` keeps the page's quote currency. Reproduced: a summary with no currency and no symbol recorded the order as `("sec-s-nocur", "", CAD)`. **Required:** refuse when the listing states no currency or symbol, and remove both fallbacks and `Ticket.currency`.

3. **Two of the new rules are held by no test.**
   - The preview's override of the page's currency by the listing's.
   - Recording an id only for a share or fund in the instrument's own currency.

   **Required:**
   - A preview test where the page's currency differs from the listing's.
   - A resolve test where the found listing's currency differs from the instrument's, and one on an instrument that is not a share. Each asserts that nothing is recorded.

4. **The late-answer guard counts tickets, not requests** (`ticket.svelte.ts:208`, :232–236). Reproduced: Buy on the ENB row, then Buy on another row whose answer came later. The ENB ticket opened, and the later click's answer was dropped. **Required:** a request counter, as `refreshPreview` uses `asked`.

5. **Max stays off on a Sell from a holding with no recorded id.** `heldIn` still matches by security (`ticket.svelte.ts:125`, :132–134). SPEC order-ticket-15 says a Sell's Max is the position's shares. **Required:** match by the holding id, as the defaults now do.

6. **Nothing in the e2e makes the real Wealthsimple host unreachable except `BAGHOLDER_OFFLINE`.**
   - **What happens:** `BAGHOLDER_WS_BASE` is read only by the old `ws` crate (`ws/src/session.rs:26–38`). `bagholder_wealthsimple` hard-codes the real host (`client.rs:17`, :148; `session.rs:21`, :165), and it does the token refresh, the sign-in takeover and the sync. The signed-in server holds a `session.json`. Today only the offline switch keeps those calls in.
   - **Required:**
     - Both crates read the base.
     - A test that a signed-in e2e server sends nothing to any host but the stand-in.

## Smaller, in the same PR

- **Refusal words.** "Wealthsimple does not know the listing sec-…" names a raw id (`ticket.rs:197`), and the quote document's refusal is lowercase "no listing named" (`ticket.rs:124`). Name the listing as the ticket shows it.
- **The e2e leaves its temporary homes behind** (`/tmp/bagholder-e2e-*`, some with `session.json`): Playwright stops `serve.mjs` before its cleanup runs. Remove them at the next start, or in a global teardown.

## Left for brief 34's next plan

On a page with no venue, the name, currency, kind, fills and price still come from the server's match by ticker (`feeds.rs:2685–2705`). `listing:UBER@` showed UBER's fills, and the NYSE TD's chart would show the TSX TD's bars, which are stored by ticker. These are display, not orders. They are the market readers' rekeying, brief 34 sections 2 and 3, whose plan must also cover the demo book.
