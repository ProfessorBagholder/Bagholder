# Plan: a made-up book of any size, a capacity test that holds the server to a budget for a Raspberry Pi, and the rest of brief 15's process stage

## For the owner to decide

Nothing open.

## Scope

Brief 15 set six stages; the first, "Stage P: the process files", is half done. The `CLAUDE.md` changes it wrote are on `master`; the stage's other parts are not. This plan builds the rest:

- **A. A made-up book of any size**, built by the real broker pull, so that a cost can be measured at the owner's size and at a multiple of it.
- **B. The capacity test**: startup, the broker read every figure pass makes (`engine_inputs::brokers`), one price change, one stream step, one import and one added trade, each held to a budget for a Raspberry Pi, plus a rule that work on one change does not grow with the book. It runs in CI on an arm64 Linux machine.
- **C. The workflows**: dependency audits, actions pinned by digest, the image started in CI on the made-up book with a dry order placed through the page, and layer caching.
- **D. `docs/architecture.md` and `docs/decisions.md`** brought into line: the cost budget in §16, unbuilt items marked where they stand, and no brief numbers cited.
- **E. `SPEC.md`**: one rule per line, each with a permanent id, held by a test, without changing a word of meaning.
- **F. Known defects as issues**: every brief 15 finding no merged stage has fixed is filed as a GitHub issue, with its severity, the product that sets the standard, and the test that will hold the fix.

Nothing on screen changes. Out of scope on purpose: fixing anything the capacity test finds over budget. Those fixes are Stage 3 of brief 15 ("light at the owner's size"); this plan lists them (Part B, the over-budget list) and files them (Part F), and Stage 3's plan takes them up.

## The old app here

The old app had no capacity test, no cost budget and a fixed demo book (`rust/crates/store/src/bin/demo_book.rs`: four accounts, 138 transactions). That book writes the old `bagholder.db`, which the server carries into its own book on first start. It stays as the browser tests' and README's book: its figures are what `web/e2e` asserts and what `server/src/demo_facts.rs` states, and changing it would change every one of those tests for no gain. It is never used to measure a cost (`CLAUDE.md`: "a cost is measured at the owner's size, never on the demo book alone"). The sized book in Part A is new and is not written in the old store's format, because Stage 2 removes that store (`store`, `model`).

Nothing is carried over from the old app.

## How the leading products do it

Trading journals do not publish how they test their servers' capacity, so the sources here are the engineering practice for performance budgets and CI, each read on 2026-10-08:

- **A performance budget is a set of limits held by the build.** web.dev: "A performance budget is a set of limits imposed on metrics that affect site performance", enforced in the build process ([web.dev, Performance budgets 101](https://web.dev/articles/performance-budgets-101)). Grafana k6 fails a run whose thresholds are not met, with a non-zero exit code ([k6 thresholds](https://grafana.com/docs/k6/latest/using-k6/thresholds/)). This plan does the same: a missed budget fails the job.
- **Where the budgets come from.** Where a person waits, Google's RAIL model: respond to input within 100 ms, load within 5 s on a mid-range device the first time ([web.dev, RAIL](https://web.dev/articles/rail)). RAIL is a model for a browser page answering a person, so it is not a source for a server loop no one waits on. Those loops are held to the app's own cadences instead: quotes every 60 s (`due.rs` `QUOTES_EVERY`), cash and buying power every 5 minutes while a page is open (`broker_reads.rs` `BALANCES_EVERY`). One pass must finish inside its cadence at four times the owner's size on a Pi (brief 21, change 1). No budget is a figure fitted to one book.
- **Machine-independent counts in CI.** Benchmark harnesses built for CI count work instead of timing it. Gungraun counts instructions so that results are comparable across systems, "completely negating the noise of the environment" ([gungraun README](https://github.com/iai-callgrind/iai-callgrind)). Part B counts SQLite's virtual-machine steps and allocations the same way, using SQLite's progress handler, to hold the growth rule. Wall time is kept for the absolute budget.
- **An arm64 machine in CI.** GitHub's `ubuntu-24.04-arm` runners are free for public repositories and generally available since August 2025 ([changelog](https://github.blog/changelog/2025-08-07-arm64-hosted-runners-for-public-repositories-are-now-generally-available/)). They run on Azure Cobalt 100, 4 vCPU ([changelog, January 2025](https://github.blog/changelog/2025-01-16-linux-arm64-hosted-runners-now-available-for-free-in-public-repositories-public-preview/)).
- **How a runner's time maps to a Pi's.** Raspberry Pi's own measurement of the Pi 5 averages 764 in Geekbench 6 single-core over a hundred runs ([Raspberry Pi, Benchmarking Raspberry Pi 5](https://www.raspberrypi.com/news/benchmarking-raspberry-pi-5/)). Published Cobalt 100 runs score 1,620 and 1,639 ([Geekbench Browser 18567785](https://browser.geekbench.com/v6/cpu/18567785), [18567788](https://browser.geekbench.com/v6/cpu/18567788)). A Pi is therefore taken as 2.1 times slower than the runner on one core. This is a named parameter, `PI_SLOWDOWN`, marked "CPU only; not measured on hardware". It does not cover storage: the same Raspberry Pi page measures 4K random reads at 4.6 MB/s on the Pi 5's SD card, which is where this app's cost lies on a large book. So IO is counted, not timed (Part B).
- **Actions pinned by digest.** GitHub: "Pinning an action to a full-length commit SHA is currently the only way to use an action as an immutable release" ([Security hardening for GitHub Actions](https://docs.github.com/en/actions/security-for-github-actions/security-guides/security-hardening-for-github-actions)). Dependabot keeps the pins current.
- **Dependency audits.** `cargo audit` checks `Cargo.lock` against the RustSec advisory database ([rustsec.org](https://rustsec.org/)); `npm audit` does the same for `package-lock.json`.
- **A list of known misses that only shrinks.** This repository already holds remaining timers this way (`TIMED_WAITS`, `server/src/tests_misc.rs`): a named list, each entry with its reason, and a test that fails if the list and the code disagree in either direction. The over-budget list in Part B works the same way.

## Open questions

Brief 21 (Go with changes) is applied throughout: budgets from the app's own cadences where no person waits, the Pi factor for CPU only with IO counted, the built books cached, invisible anchors for spec ids, one issue per root cause, `cargo audit`'s failure stated, and brief 20's transfer rule built separately.


1. **The Pi factor.** *Objective:* a budget stated for a Pi and checked on a CI runner. *Known:* the two Geekbench sources above. They are single-core scores from different Geekbench builds, and the Cobalt runs were on a Windows VM. *Best course:* use 2.1 as `PI_SLOWDOWN`, with its sources in the code, and replace it with a measured ratio if anyone ever runs `capacity measure` on a real Pi (the command prints the same numbers on any machine). Buying or borrowing hardware to settle a factor the growth rule doesn't depend on would cost more than it tells. Nothing is asked of the owner.
2. **The owner's size.** *Objective:* measure at the owner's size, as `CLAUDE.md` requires. *Known:* counted on a copy of the owner's book on 2026-10-08: 29 accounts (21 with activity), 210 instruments, 7,128 transactions over three years, 14,606 broker records, 646 trades, 24,245 account-days back to 2020, and the balances rows (statements) growing by about 890 a day across the accounts. *Best course:* these counts become the generator's `owner` preset, which is a fact about the size of the book and nothing more. Its balances history is set to one year at that rate, because that is the size the book reaches in normal use. Brief 15's measurements found the costs climbing at exactly that size: `brokers()` 14 s and an added trade 36 to 40 s with a year of balances rows. The test runs the preset and four times it.

## Approach

**A. The sized book: `capacity build <dir> [--accounts N --instruments N --trades N --months N --balances-days N --seed N | --preset owner]`**

- A new binary, `capacity`, in the server crate (`rust/crates/server/src/bin/capacity.rs`). It is not one of the release archive's binaries; `release.yml` names them explicitly and is unchanged.
- It builds a book by running `bagholder_broker::pull::pull` and `pull::balances` (`broker/src/pull.rs:87`, `:651`) against a generated `BrokerAdapter` (`broker/src/lib.rs:197`). Every row therefore goes through the same mapping, identity, linking and trade assignment the real pull uses, and the derived tables (`transactions`, `links`, `trades`, `statements`, `account_days`) are what the app writes, not a separate seed.
- The adapter answers in Wealthsimple's reply shapes, built from the recorded replies already in `rust/crates/wealthsimple/tests`, and delegates `mapping()` to the Wealthsimple mapping. Its rows are generated from the seed: buys and sells that open and close round trips across stocks, ETFs, options (expiry and assignment), crypto with staking, dividends, deposits, withdrawals, transfers between accounts, currency conversions, and corporate events. Instruments are generated listings on generated venues in every currency the money type accepts, never a real ticker as a case of its own.
- The market cache is filled through the cache's own writers (`sources/src/cache.rs`): a quote per instrument, closes over the book's span, and the benchmark series.
- Balances history: `pull::balances` is called once per generated pass, as many passes a day as the owner's measured rate, over `--balances-days`.
- Deterministic: the same seed and sizes give the same book, which a test checks (two builds, the same table counts and the same figures digest).

**B. The capacity test: `capacity measure <dir>`, and the CI job**

What is measured, on the release build:

| Operation | How | Budget on a Pi (RAIL) |
| --- | --- | --- |
| Startup to the first figure | start `bagholder` on the folder; time from exec to the first figures message on `/api/events` | ≤ 5 s ("load within 5 s on a mid-range device") |
| An added trade, to its figures on the stream | `POST /api/entries`, time to the stream's patch | ≤ 1 s (RAIL: past 1 s the person loses the thread of the task); the request's own answer ≤ 100 ms |
| One import of a small file, to its figures | `POST /api/import` of a generated 20-row file, time to `imported` and the patch | ≤ 1 s |
| `engine_inputs::brokers` | called directly, in process | ≤ `BALANCES_EVERY` ÷ accounts at four times the owner's size (one balances pass touches every account inside its cadence) |
| One `Figures::price_changed` | one instrument's quote changed, called directly | ≤ `QUOTES_EVERY` ÷ priced holdings at four times the owner's size (one quote pass prices every holding inside its minute) |
| One `Feed::step` for one signal | a stream holding the page's subscription set, one signal | ≤ `QUOTES_EVERY` ÷ the signals a quote pass sends at that size (one signal per instrument whose quote moved, the most a pass sends) |

- **What fails a PR is the growth rule (below), which is counted and machine-independent.** Wall time is the median of seven runs on `ubuntu-24.04-arm`, multiplied by `PI_SLOWDOWN`. It is reported for every operation and holds only the three a person waits on (startup, the added trade, the import). The table marks each operation as CPU-bound or IO-bound. For the IO-bound ones it also counts SQLite page reads (`SQLITE_DBSTATUS_CACHE_MISS` on each connection the operation used), so an IO regression shows up as a counted number rather than a timing. Each budget is a named constant in `capacity.rs`, and each constant's line names the fact it comes from (a RAIL quote, or a cadence and the size it is divided by). The budgets are copied into `docs/architecture.md` §16 (Part D) by a test that reads both, so the two never disagree.
- **Growth rule** (`docs/architecture.md`: work happens only because something changed). For each per-change operation (`brokers`, `price_changed`, `Feed::step`, the added trade's write path, the import's linking), the test counts SQLite virtual-machine steps through `Connection::progress_handler(1, …)` (rusqlite's `hooks` feature, already on in `book`), plus allocations through a counting global allocator in the `capacity` binary. It does this on the owner preset and on four times it, with the changed thing held the same. Work proportional to the book grows about 4×; work through an index grows by the B-tree's depth, well under 2×. The rule asserts under 2×, a cut-off that separates linear growth from logarithmic. Startup is allowed to grow with the book; its wall budget holds it.
- **The over-budget list.** `OVER_BUDGET: [(operation, issue, reason)]` in `capacity.rs`. The job fails when an operation not on the list misses its budget or the growth rule, and also when an operation on the list now meets both, so the entry has to be removed. It starts with what the first run on the owner preset finds. Brief 15's measurements predict `brokers`, `price_changed`, the added trade and startup, because the balances history is read whole (gap1-1) and the import's linking scans every CSV row ever imported (gap1-3). Each entry names its Part F issue. Stage 3 empties the list.
- **CI:** a `capacity` job in `tests.yml` on `ubuntu-24.04-arm`. It runs on every pull request that touches `rust/` or `web/`. The two built books (the owner preset and four times it) are kept in `actions/cache`, keyed by a hash of the generator's source, the seed and the sizes, so a PR that leaves the generator alone pays only for the measurement. The build is deterministic, which is what makes the cache sound. The job then measures, and prints the table above with the runner's times, the Pi-scaled times and each budget, so the PR can paste it as `CLAUDE.md`'s cost line asks.
- **In the normal suite:** `cargo test -p bagholder-server --test capacity_small` runs `build` and `measure` on a small preset (so the binary and generator cannot rot unseen) and checks the generator's determinism. It asserts the growth rule only, never wall time, because developer machines differ.

**C. Workflows**

- `cargo audit` (installed with `--locked` at a pinned version) and `npm audit` in `tests.yml`. A vulnerability advisory fails the job, which is `cargo audit`'s default exit status. An unmaintained-crate or yanked-crate warning is printed and does not fail it, because the project cannot always act on one. `npm audit` fails at any severity.
- Every `uses:` in every workflow pinned to a full commit SHA with the tag as a comment; `.github/dependabot.yml` for `github-actions`, `cargo` and `npm`, so the pins and lockfiles get update PRs.
- `docker.yml`: build with `cache-from`/`cache-to: type=gha`. A `container` job in `tests.yml` builds the image, starts it on the made-up book (`BAGHOLDER_DRY_ORDERS=1`, offline), and drives Playwright against it for one dry order through the order ticket, the page's own path. It asserts the order's dry answer on the Orders panel.
- The release already publishes only once every archive is attached (`release.yml`: created as a prerelease, made latest by the `publish` job after `rust` and `docker`). Nothing changes there.

**D. `docs/architecture.md` and `docs/decisions.md`**

- §16 gets the cost budget: the table above, `PI_SLOWDOWN` and its sources, the growth rule, and "held by the `capacity` job". §16's "installs itself as the operating system's service" contradicts the owner's decision of 2026-09-26 ("The app never sets itself up to start at login"), and signed releases are not built. Both are corrected or marked "not built" in place, as are §12's keychain and per-page write token and §14's MCP server.
- Brief numbers cited in the text (§13's "brief 13") are replaced by what they stood for.
- `docs/decisions.md`'s header gains the rule brief 15 set: a decision carries the owner's own words and where they were said, and one proposed by a brief or a session is marked "unconfirmed" until the owner confirms it. Every existing entry already quotes the owner. The brief's proposed 2026-09-29 entry has no words of the owner's to carry, and `CLAUDE.md`'s first rule already states it, so it is not added.

**E. `SPEC.md`: one rule per line, with ids**

- Each rule goes on its own line, beginning with an invisible anchor of the form `<a id="figures.avg-cost"></a>`, made from its section and its subject. GitHub renders nothing for it, so the rendered spec carries no visible tag, and each rule becomes linkable as `SPEC.md#figures.avg-cost`. (Pandoc's `{#…}` syntax would render as literal text on GitHub.) Paragraphs that hold several rules are split at sentence boundaries; no word is changed. A check script compares the old and new text with ids and line breaks stripped and requires them to be identical. It runs once, in the PR, as evidence.
- The test `spec_ids` (in `rust/crates/server/tests/`, next to the other repository checks) asserts: every rule line carries an id; ids are unique; and an id removed from `SPEC.md` is listed in `docs/spec-retired-ids.md`, so an id is never reused. Brief 15 also asked for a line-length test. It is replaced by "one id per rule line", because a length limit would have no source and one rule per line is what the limit was for (Right to refuse, below).

**F. Known defects as issues**

- Each finding in brief 15 (§2 to §5 and the six gap reviews) is checked against `master`, by reading the code at the cited line, to see whether a merged stage fixed it. The findings not fixed are grouped by root cause (for example, the balances table read whole, the cache keyed by symbol, the constant write token), and **one issue is filed per root cause**. Each issue carries every finding id it covers, the severity of the worst of them, the product or standard named, the test that will hold the fix, and a `stage-N` label. The security gap at `docs/old-app-mistakes.md:63`, gap6-1 and gap6-2 go first.
- The PR carries the map from each finding to its issue, and lists each finding a merged stage fixed with the commit that fixed it.
- Brief 20's corrected item 3 (a transfer across a registered boundary is a disposition at the broker's stated value) is not filed here. It is a required change of the approved figures plan, built in its own PR before this stage (brief 21, change 7).

What stays the same: the fixed demo book and every browser test on it; every screen; the release workflow; the cadence of every loop.

## Acceptance criteria

- [ ] `RUSTFLAGS="-D warnings" cargo test --workspace` green in `rust/`, including `capacity_small` and `spec_ids`; `RUSTFLAGS="-D warnings" cargo clippy --workspace --lib --bins` clean.
- [ ] `capacity build` on the same seed and sizes twice gives identical table counts and an identical figures digest (test).
- [ ] The owner preset's table counts match the counts in "Open questions" 2 for every count named there, within the rounding of the generator's sizes, printed in the PR.
- [ ] Every generated record is read by the Wealthsimple mapping with no record problem, unless it was generated to raise one (test: `record_problems` holds only the generated ones).
- [ ] The `capacity` job runs on `ubuntu-24.04-arm` and its table (runner time, Pi-scaled time, budget, growth ratio, VM steps, allocations) is pasted in the PR.
- [ ] Each budget in `capacity.rs` equals §16's table (test).
- [ ] The over-budget check fails on a planted regression: a run with an extra full scan in `price_changed` is shown failing, in the PR. It also fails on a listed operation that now meets its budget, with that run shown too.
- [ ] Every `OVER_BUDGET` entry names an open issue filed in Part F.
- [ ] `cargo audit` and `npm audit` run in CI and pass, or each advisory found is fixed in this PR; a `cargo audit` warning is printed and does not fail the job.
- [ ] No `uses:` line in `.github/workflows` without a 40-hex SHA (a check in `tests.yml`, so it stays enforced); `dependabot.yml` present.
- [ ] The `container` job places a dry order through the page in the image started on the made-up book, green in CI.
- [ ] `SPEC.md`: the strip-and-compare script shows the text unchanged except for ids and line breaks; `spec_ids` green.
- [ ] `docs/architecture.md` cites no brief by number; §16 holds the budget; every unbuilt item it describes is marked "not built" where it stands; nothing in it contradicts `docs/decisions.md`.
- [ ] One issue per root cause of the unfixed brief 15 findings; the PR maps every finding id to its issue or to the commit that fixed it.
- [ ] Wall budgets fail the job only for startup, the added trade and the import; every other operation's wall time is reported, and the growth rule (VM steps, allocations, and page reads for IO-bound operations) is what fails it.
- [ ] A second run of the `capacity` job on an unchanged generator restores both books from the cache (the log shows the cache hit).
- [ ] The rendered `SPEC.md` on GitHub shows no id text (screenshot in the PR).

## Surfaces to check beyond the diff

`.github/workflows/release.yml` (the binaries named, so `capacity` stays out of the archives); `TIMED_WAITS` (the capacity runner waits on the stream, never on a clock; any wait it needs is argued there); `rust/crates/wealthsimple/tests` (the recorded replies the generator's shapes come from); `web/e2e/serve.mjs` (the fixed demo book is unchanged); `docs/decisions.md` "Held by" lines naming tests this touches; the docker image's entrypoint for the `container` job.

## Right to refuse

- Brief 15 asked for "a test on line length" for `SPEC.md`. A length limit would be a number with no source, which `CLAUDE.md` forbids; one rule per line with an id holds what the limit was for. Argued here for the reviewer.
- Brief 15's proposed 2026-09-29 decision entry has no words from the owner; `docs/decisions.md` admits only the owner's own words, and the rule it states is already `CLAUDE.md`'s first rule. Not added.
- If an operation is so far over budget that seven runs of it at four times the owner's size would not fit in a CI job (brief 15 measured an added trade at 40 s), the job runs that operation once, records it as over budget and keeps it listed. Every operation is still measured, and none is stopped part way.

## Anti-stub self-check

Initialled at build (Claude): no definition nobody references; no field written and never read; no branch only the switch knows. The suite was run, the capacity run was made on this machine at the owner's size and four times it, and the made-up book was compared table by table with a copy of the owner's book.

## Verification

### What was built differently from the approach above, and why

- **`bagholder capacity`, a subcommand, not a separate `capacity` binary.** What it measures is the server's own code (`figures`, `events`, `entries`, `csv_import`), which only the server binary holds; `demo-facts` and `pull-broker` are subcommands for the same reason. The release archive therefore carries it, at the cost of its code in the binary; nothing runs unless it is asked for.
- **`capacity build <folder> [--times N] [--seed S]` and `capacity run <work folder>`** in place of the per-count flags and `capacity measure`: the sizes are the owner's (`OWNER`, counted on a copy of the owner's book) times N, so no size is chosen by hand.
- **The book also holds the older activity from Wealthsimple's activity export, imported through the import.** A copy of the owner's book holds 6,156 rows of that export beside 7,994 feed records, and every import is linked against them (#419); a book without them would understate an import. The export is written from the same made-up replies, in the export's own words, as the owner's imported rows state them.
- **Bytes read are counted from Linux's `/proc/self/io` (`rchar`), not `SQLITE_DBSTATUS_CACHE_MISS`.** SQLite reads its pages through `read()`, so the bytes the process reads are its page reads past its own cache, on every connection without reaching into each; the runner is Linux, and elsewhere the column says "not counted here".
- **Startup is timed from the exec to the figures built**, not to the first message on `/api/events`: the stream's first message is sent from the figures once built, and a probe that started an HTTP server and a browser stream would time the loopback as well.
- **An operation listed over budget is measured once, not seven times.** Its counts are the same every run and one run over budget keeps it over; only a run within budget is taken to the median of seven, to say it comes off the list. This is the "Right to refuse" case, decided by the list rather than by a time limit.
- **Each probe runs on a fresh copy of the built folder.** Every probe writes (a quote, a trade, an import, what a start settles); without a copy the cached books would drift between runs.
- **The normal suite's check is unit tests in `capacity.rs`**, not a `capacity_small` integration test: the probes are the server's own functions, which an integration test of the binary cannot call.
- **SPEC ids are `<section>-<n>`**, not made from the rule's subject: an id must not change when the wording does, and a subject derived from words would. Every id ever given is listed in `docs/spec-ids.md` (standing or retired), so the test catches an id dropped without being retired as well as one given twice.
- **The `capacity` job runs on every pull request the Tests workflow runs on**, not only those touching `rust/` or `web/`: the workflow's own filter already skips documentation, apart from the three markdown files a test reads (`SPEC.md`, `docs/spec-ids.md`, `docs/architecture.md`), which now run the tests (a test holds the list).
- **Every operation is over budget today**, so the planted-regression run cannot be shown on an operation off the list. The verdict's logic is held by `test_an_operation_over_budget_fails_unless_listed_and_a_listed_one_within_fails_too` (a linear count planted on `price-changed`), and the first run here, with `OVER_BUDGET` empty, failed on every operation (below).

### Found while building

- **An import row named a share by its symbol, and the match took every instrument seen under that symbol.** Wealthsimple names each option contract by its underlying's symbol, so once a contract was held, the share's rows matched several instruments and the import made a new instrument and a second copy of each trade and dividend; a contract written by its terms was never matched to the contract the feed made. Fixed in this PR for both paths that look an instrument up by symbol (`entries::held_by_symbol`, now with the row's kind inside the match, and `entries::contract_by_terms`); `test_the_activity_export_links_to_the_feed_it_repeats` fails without the fix and passes with it. The owner's book is not affected: its exports came in through the earlier import.
- **An asset movement is read as holdings by the feed and as cash by the import**, so the two never link (a copy of the owner's book holds one such row). Added to #405.

### Commands and numbers

- `RUSTFLAGS="-D warnings" cargo test --workspace` (in `rust/`): every test binary passed. `cargo clippy --workspace --lib --bins`: clean. `npm run check`: 0 errors. `npm test`: 28 files, 141 tests passed.
- `SPEC.md`: `spec_ids.py same` (the converter's check, run once here and not kept) printed "same text: only ids and line breaks differ"; 964 ids, each listed in `docs/spec-ids.md`.
- The made-up owner-size book against a copy of the owner's book (`capacity build`, 28 s on an M-series Mac):

  | Count | Owner's book | Made-up |
  | --- | --- | --- |
  | Accounts | 29 (21 with activity) | 29 (29 with activity) |
  | Instruments | 210 | 203 |
  | Transactions | 7,137 | 7,807 |
  | Feed records | 7,994 | 7,674 |
  | Activity-export records | 6,156 | 5,909 |
  | Trades | 646 | 627 |
  | Balances rows over a year | about 325,000 (890 a day) | 328,193 |
  | Record problems | | 0 |

  Every account has activity in the made-up book, so it holds more account-days (31,813 against 24,245): the larger of the two, which is the side a budget should err on.
- `bagholder capacity run` on this machine (an M-series Mac, so the "Runner" column is this Mac, not the arm64 runner; the CI job's table is in the pull request), 11 minutes including both builds:

  | Operation | Here at ×4 (ms) | Pi (ms) | Budget (ms) | VM steps ×1 → ×4 | Allocations ×1 → ×4 |
  | --- | --- | --- | --- | --- | --- |
  | startup | 25,135 | 52,784 | 5,000 | 67,557,866 → 955,460,070 | 1,510,838 → 5,943,452 |
  | add-trade | 25,448 | 53,441 | 100 | 69,301,613 → 962,400,105 | 1,691,780 → 6,729,708 |
  | import | 23,345 | 49,024 | 1,000 | 69,707,069 → 964,008,512 | 1,718,557 → 6,797,813 |
  | brokers | 20,874 | 43,835 | 2,586 | 66,393,553 → 950,626,357 | 866,177 → 3,466,072 |
  | price-changed | 19,513 | 40,977 | 95 | 66,417,442 → 950,675,707 | 879,537 → 3,489,107 |
  | feed-step | 49 | 104 | 95 | 1,562 → 1,562 | 148,316 → 712,199 |

  The work grows 14 times when the book grows four times: `brokers()` reads every balances row of every account, and every other operation but the stream step runs through it. The first run, with `OVER_BUDGET` empty, failed naming all six operations; with them listed against their issues it passes.

## Handoff

- `OVER_BUDGET` lists every operation: startup and `brokers` (#417), `price-changed` (#418), `import` (#419), `add-trade` (#420), `feed-step` (#426). Stage 3 of brief 15 takes them off one by one; the job fails when one is within budget and still listed.
- Part F: 104 issues, #383 to #486, one per root cause, each with its findings, severity, standard and the test that will hold the fix.

**Nothing left running.**
