# Brief 69: #559 (a SEDAR+ document past the list's first page): the cause is right; two changes

## Verdict

**The diagnosis and the paging are right.** A document is matched only on the scoped list's first page, so it becomes unreadable once newer filings push it to page 2 (30 a page). The fix pages through the list with its own control, posts no form fields, and stops at the first page whose newest row is older than the document's day. That stop is exact, because the list is newest first. It was verified live: the document on page 2 downloads. #541 made the old silent miss visible, as it should.

## Required changes

1. **No cap of 40 pages.** `DOWNLOAD_PAGES = 40` is a size with no stated source (`CLAUDE.md`: a window or threshold comes from a stated source). The list states its own length ("Displaying 1-30 of 512 results"), so the walk is bounded by that: ceil(total ÷ page size), read from the page. The date stop usually ends it far sooner.
   **Before writing that, check one thing:** SEDAR+'s document search may take a date range, which its search page offers as a filter. If so, scope the search to the document's filed day: the document is then on the first page, in one request instead of several. That is the cheaper read, and the one to build if the filter is there. Say in the PR which one was found.
   *Test:* `next_page` and the total parsed from the recorded page; a walk that would pass the stated total stops there.
2. **A document no longer on SEDAR+ is a fact, not a failure.** Suppose the walk reaches the filed day, or the list's end, without finding the document. Then SEDAR+ no longer lists it: withdrawn, or replaced. Said as a header failure, it would stand forever, because nothing can resolve it (`docs/decisions.md`, 2026-10-09: a failure is said once confirmed, and stands until a read shows it resolved).
   - Keep it in the store as no longer listed, with the day that was found.
   - Show it on its row, in the Disclosures card's own words.
   - Never put it in the header.
   - A transport or session failure on the way is still a failure, said as now.
   *Test:* a walk that passes the day without a match gives "no longer listed", with nothing in the header. A refused page gives a failure.

Build in one push.
