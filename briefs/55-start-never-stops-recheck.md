# Brief 55: #523 at `60ef5e04`: one change left. A store mended while the server runs reopens only through `GET /api/status`, which the page never asks

## Verdict

**Not yet, one small change.** Brief 50's seven changes are built well:
- the header is answered whatever the cache does;
- each failure is one true sentence;
- "not an error" is gone;
- the guard against building from records not derived again has its own test;
- the source scan holds the start for any step added later;
- CI 39170475713 and capacity 39170475614 are green.

One gap remains. The reopen that change 2 asks for runs only on the legacy status route. The page never takes that route.

The owner's OK on #523 stands. It takes effect once the next brief says merge.

## A correction to brief 50

Brief 50 said that a market cache that cannot be opened leaves the page with no header and no update button. **That was wrong.** The page takes its header from the stream (`http/stream.rs`, `feed.step(&status::status)`), and the stream never touched the cache. The CTO showed it: `startfailure.spec.ts` passes with change 1 undone. Change 1 is still right. `GET /api/status` no longer refuses whole, and the versions are typed absent rather than made up. But it changes what other readers of that route get, not what the page shows. The CTO's `start_failures` test holds it.

## The gap

`open_figures` is called in two places only:
- at start (`main.rs`);
- in `status::answer` (`status.rs`), which serves `GET /api/status`.

The page's header is `status::status`, run on each stream step, and that never calls it. `web/src/` asks `/api/status` nowhere outside its tests. So with the page open, a book or cache mended while the server runs is still said as broken, and the figures stay closed, until something asks the legacy route or the server restarts.

`a_store_mended_while_the_server_runs_opens_at_the_next_header_read` passes because its `header()` reads `GET /api/status`, the one path that reopens.

## Required change

1. **Reopen on the path the page reads.** Move the `open_tried` → `open_figures` call from `status::answer` into `status::status`, which every header read goes through, stream or legacy:
   - a stream step already runs on the blocking pool;
   - `single_flight` keeps two steps from opening at once;
   - with the figures open, the call costs one `OnceLock` check.

   Nothing is added on a timer. A step runs on a signal, or when a page connects, so a person who reloads after mending gets the reopen.
   *Tests:*
   - change the mended-store test to read the header from the stream, as the page does. The first message of a new `GET /api/stream`, or `feed.step(&status::status)`, will do. It must not read `/api/status`;
   - extend `startfailure.spec.ts`: mend the cache while the server runs, reload the page, and the error line clears and a figure shows;
   - *plant:* leave the call in `answer` only, and both tests fail.

   Push once. The run on that push is the merge evidence.
