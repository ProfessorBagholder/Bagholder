# Plan: replace the hand-rolled web stack with Rust (axum) + Svelte, incrementally

**Superseded** by `docs/architecture.md` (the design) and the order of work in `docs/design-review.md`. Kept as history; nothing here is checked against.

A program-level done-contract for the frontend/backend rewrite. This is the authoritative plan the phased PRs are checked against; each phase copies `PLAN.template.md` for its own per-task contract. Mark it up — nothing here is built until a phase's PR lands green.

## Why

`ledger.html` is a single 5,289-line file whose inline `<script>` hand-implements a reactive framework: a positional DOM reconciler (`morph`/`morphChildren`/`morphNode`), tile memoization (`tile`/`_tileCache`, `coreKey`/`liveKey`), and a growing pile of special cases for externally-mounted nodes (`MORPH_SKIP` + the `keepChart`/`keepHeat`/`keepLogin`/`keepTab` dance). This is a worse reimplementation of what mature frameworks solve, and it manufactures a specific bug class: **reconciliation by sibling position without stable keys**. Two recent shipped regressions are the same root cause —

- **v1.46.3** — the markets heatmap (`#heatBox`) was left mounted on other tabs after visiting Markets (it appeared under Portfolio→Sectors and Cashflow→Cashflow Positions until a reload), because `morphNode` skipped an externally-mounted node by its *old* id while nodes are paired by *position*.
- **v1.46.2** — the selected tab's underline re-faded on every redraw because `morphNode` stripped the indicator's inline opacity.

Every new interactive surface must correctly join `MORPH_SKIP`, `coreKey`/`liveKey`, the keep-dance, and manual focus/scroll restore — a large, undocumented invariant surface where these bugs live unnoticed. The backend has the parallel problem: a bespoke `match path` router on `tiny_http`, hand-parsing routes the ecosystem already solves.

The fix is to stop reinventing wheels: adopt a real reactive framework and a real web framework.

## Decision (settled)

- **Backend: Rust**, as the single surviving port (retire Python and Go). Chosen for compile-time correctness against the recurring-regression problem, lean local/Pi runtime, and because the Rust port already exists and passes the shared model cases — this is "retire two ports," not "build from scratch."
- **Backend framework: `axum` + `tower`/`tower-http` on `tokio`**, replacing the bespoke `tiny_http` `match path` router. Async is a genuine benefit here, not overhead: the concurrent rate-limited data fan-out (TMX/Yahoo/Coinbase/SEDAR+/EDGAR/FINRA), the planned server-side delta/WSS push (render redesign step 2), and the still-open multi-user hosted path are all concurrency/streaming workloads. `axum` sits on `hyper`; if anything is ever awkward at the `axum` level the escape hatch is dropping to `hyper`/`tower` in the same codebase — never a framework switch.
- **Frontend: Svelte (SPA) in TypeScript, built by Vite** — **not** SvelteKit (we do not want its Node server; the Rust backend is the server). Compiled fine-grained updates fit the Pi/phone/1-minute-tick constraint; `use:` actions are the right primitive for the imperative islands and dissolve the `MORPH_SKIP`/keep-dance entirely.
- **Client/server seam: `ts-rs`** — generate the TypeScript API types from the Rust structs so a shape mismatch is a build error, not a runtime bug.
- **Transport: HTTP/JSON, unchanged.** **Tauri is rejected for now**: bundling the backend into a GUI process with IPC would break the Pi/Docker/browser server model. A Proton-style thin desktop shell *over HTTP* (additive, never a replacement for the server) stays a future option if a hosted service matures.
- **Rejected alternatives:** React (VDOM re-render is the wrong trade against the tick-heavy Pi/phone constraint; `useEffect`/StrictMode make imperative-island interop the hardest of the three); all-Rust WASM (Leptos/Dioxus) (loses on JS-library interop for charts/heatmap/MJPEG and on maturity); a hand-hardened keyed reconciler (still writing a framework by hand).
- **Scope: web app only.** Throughout this plan "phone"/"phone-class" means the webapp opened in a phone browser — a valid low-end web target — not the native iOS/Android apps, which are paused and out of scope. No decision here is made to preserve mobile parity; the server-side model is kept for web-only reasons (tested code, trust boundary, data locality), not for the phones.

## Baseline (what exists today), with file anchors

- **Served verbatim, no build step**, by all three ports, each resolving the repo root from the binary: Python `python/bagholder.py:7581`; Rust `rust/crates/server/src/main.rs:319` (root walk `main.rs:40`); Go `go/internal/app/server.go:302` (`go/static.go` disk-first / `go:embed`). "Edit the root file, reload" is the current dev loop — a build step is the biggest DX cost (open question 1).
- **Frontend surface:** hash router (`ledger.html:4687`, dispatch `:4400`); tabs Dashboard (`:2464`), Trades (`:3091`), Portfolio (`:4167`), Markets (`:3865`), Cashflow (`:4192`); overlays: order ticket (`:1799`), Orders panel (`:2184`), Notifications (`:1025`), modals, filter popover, login view.
- **Five imperative islands** any migration must interop with via lifecycle hooks: lightweight-charts (`mountTradeChart` `:2559`), the div-treemap heatmap (`mountHeatmap` `:3411`), the MJPEG login stream (`loginFrameLoop` `:1179`, input `:1184`), the digit reels (`mountReels` `:4516`), the tab indicator (`placeTabIndicator` `:4669`).
- **API is the firewall.** The *authoritative financial computation* (FIFO matching, P&L, cost basis, FX, aggregation) stays server-side — it is the tested model that owns the trust boundary and lives with the data — and the API returns raw numbers, not pre-formatted strings. The frontend is a **rich view** over `/api/*` JSON: it formats, sorts, filters, holds interaction state and (later) does optimistic updates, but it never re-derives the money math. It is a rich client, not a dumb renderer — the current page already does client-side formatting and sorting. Because the client never duplicates the model, the reactive layer can be replaced without touching the model, the API, or the backend-consolidation goal. Data layer to preserve behavior of: `loadModel`/`loadLive`/`pollStatus` (`ledger.html:775-898`), global `act()` dispatch (`:4865`). Streams: login MJPEG `/api/login/stream`, notifications SSE `/api/notifications/stream`.
- **Rust server today:** `tiny_http` (synchronous, thread-per-request), hand-written `match path` router (`main.rs:311`), `serde`/`serde_json`, `rusqlite` (bundled). Serving a built bundle and adding a `/v3` route is a small additive change next to the existing `/v2`.

## Strategy: incremental strangler (not a big-bang rewrite)

The surface is large and the correctness contract is exacting (SPEC figures, verified in review, not tests). A big-bang cutover puts all that risk in one PR with no safety net. Instead, stand up the new stack beside the old page and migrate one surface at a time behind the existing routes; the legacy page keeps working throughout, so **feature and bugfix work (including the parked crypto items) continues on the live app in parallel** and the migration never freezes the product.

Order of phases deliberately front-loads the two things that can actually go wrong — the imperative islands and the build/packaging cutover — into a cheap spike.

### Phase 0 — Spike / walking skeleton (`/v3`), ~3–5 days, one PR
Prove the whole toolchain and the hardest interop pattern for days of cost before committing weeks.
- Vite + Svelte (TS) project under `web/`, built to a static bundle.
- Rust `axum` route serving that bundle at **`/v3`** (alongside the untouched legacy page), via `tower-http::ServeDir`. The rest of the server stays `tiny_http` for now — `/v3` is served by a small `axum` app so we prove `axum` + static serving without yet migrating the whole router.
- Port, behavior-unchanged: the `state` store (`ledger.html:470-544`) to Svelte runes, the hash router, and the data layer (`loadModel`/`loadLive`/status poll).
- Migrate **one tab (Dashboard)** and **one island (its equity chart)** as a Svelte `use:` action with real mount/update-on-tick/cleanup.
- Stand up **Vitest + @testing-library/svelte + jsdom** and write the island-lifecycle test that reproduces the v1.46.3 / v1.46.2 bug class.

Acceptance:
- [ ] `/v3` Dashboard renders and its figures match the legacy Dashboard on the same scratch data at 1200/1340/1440/1680px (screenshot-compared).
- [ ] The equity chart is not recreated across a simulated quote tick (assert the chart instance is identity-stable), and its imperatively-set state survives a re-render.
- [ ] A Vitest test asserts an island node's imperatively-set style is preserved across a model update (fails on the old reconciler's behavior).
- [ ] `PROTOCOL` unchanged; legacy page unaffected.

### Phase 0.5 — Decision gate
Review the spike against the open questions below — chiefly the dev-loop cost. Go / no-go before further investment.

### Phase 1 — Wrap the remaining islands, ~3–5 days
All five islands as actions/components before any more tabs, because they are shared and the hard part: heatmap treemap, MJPEG login stream, digit reels, tab indicator (chart done in Phase 0). Acceptance: each mounts/updates/unmounts with no reconciler leakage, proven by a component test; the tab underline does not re-fade; the login `<img>.src` is stable across redraws.

### Phase 2 — Migrate tabs behind existing routes, ~1.5–2 weeks
Ascending risk: Dashboard → Cashflow → Portfolio → Trades → Markets (Markets last: most cards + most islands). Each ported tab consumes the same model JSON; un-ported tabs still render through the legacy path during the transition. Acceptance per tab: every SPEC figure matches the legacy render at all four widths, screenshot-diffed.

### Phase 3 — Overlays, then retire the reconciler, ~3–5 days
Port ticket / Orders / Notifications / modals / filter popover, then delete `morph*`, `tile*`, `coreKey`/`liveKey`, `MORPH_SKIP`, the keep-dance and manual focus/scroll restore. Acceptance: the inline reconciler is gone; all interaction preserved.

### Phase 4 — Backend + packaging cutover, ~1 week, overlaps
- Re-lay the whole Rust server on `axum`/`tower` (retire the `tiny_http` `match path` router); SQLite via `spawn_blocking` or an async pool; background sweeps/feeds/login-stream on tokio tasks.
- Swap the served artifact from `ledger.html` to the built `dist/`: each server's `/`, `/index.html`, `/ledger.html` route serves the built `index.html`; add a static handler for hashed `assets/*`; bundle `lightweight-charts` rather than serving it top-level.
- `python/tools/web_archive.py` (`ASSETS`/`shipped`) ships `dist/` instead of the raw page; `release.yml` (Rust job `:85`, Go `:120`) gains an `npm ci && npm run build` step and ships `dist/`; Go `//go:embed` embeds the `dist/` tree; the Dockerfiles gain a Node build stage. Version-equals-tag guards (`release.yml:31/63/112`, `docker.yml`) unaffected.
- Retire Python and Go as backends (timing is the separate consolidation call; the API firewall means this does not gate Phases 0–3).

## Surfaces to check beyond the diff
`python/tools/web_archive.py`; `.github/workflows/release.yml` and `docker.yml`; `go/static.go` (`go:embed`); the version constants (`python/bagholder.py`, `rust/crates/server/src/app.rs`, `go/internal/app/app.go`); `SPEC.md` §7 (page verification) and §Freshness (the render-scoping contract this replaces); each of the three ports' static-serving routes.

## Testing story
Today the page has essentially no behavioral tests (`python/tests/test_page.py` is parse-only + a no-`title` regex; `go/internal/app/page_test.go` only checks `PROTOCOL`) — neither could catch a parse-valid script that visually re-fades a node. Introduce **Vitest + @testing-library/svelte + jsdom** (Playwright component tests for pixel/animation-sensitive cases) covering: island lifecycle (identity + imperatively-set state preserved across updates — the bug class we hit); keyed list identity on sort/insert/remove; render-scoping (a live-only update touches only priced tiles); and a model→render contract snapshotting each tab's figures from a captured `/api/model` payload. Keep the no-`title` and `PROTOCOL` checks.

## Open questions (decide as we go)
1. **Dev loop.** A Vite build replaces "edit `ledger.html`, reload." Accept a `npm run dev` proxy loop (Vite dev server proxying `/api` to the running Rust backend)? This is the biggest cultural cost.
2. **Node/npm becomes a build-time dependency** (not shipped) in the release jobs and Dockerfiles. Confirmed acceptable.
3. **PROTOCOL / updater.** The in-app updater ships page bytes and expects a restart to reconcile `PROTOCOL`; confirm its expectations for a `dist/` tree (multiple hashed files) vs one `ledger.html` — the Python updater's flat-unpack assumption needs revisiting at Phase 4.
4. **Effort ≈ 4–6 focused weeks** across frontend + the Rust server relay + release/CI + Docker. Confirm appetite before Phase 0. *(Decided: proceed.)*

## Right to refuse
If a phase's acceptance cannot be met, or a SPEC figure cannot be matched by the new render, stop and report before continuing. A clean rewrite that shows a wrong number is still wrong.

## Verification
Per-phase, in that phase's own `PLAN.template.md`: the exact commands and the numbers, not "passes". Phase 0's proof is the screenshot comparison and the Vitest run, pasted.
