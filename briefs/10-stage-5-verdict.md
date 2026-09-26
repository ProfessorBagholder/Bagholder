# Brief 10: verdict on the stage 5 plan

**Reviewed:** `docs/plans/stage-5-interface-and-running.md` at `46b3cffc`. Checked against `master` at `09327c99` and against outside practice (`briefs/reference.md`).

**Verdict: Go with changes.**
- **Part A** is being built now: make change A1 before going further.
- **Parts B, C and D:** build them with the changes below. C and D are narrowed by the owner's decisions.

**The gate comes before the build.**
- A heavy lift is built after its verdict (brief 01). Part A started before this one, and stage 4's plan never came to the gate.
- Stage 4 was read afterwards for the four order defects of brief 01. All four are addressed:
  - a lost answer is read back, not marked Failed;
  - no POST is re-sent;
  - no list is cut at 200;
  - rows being sent count as in flight.

## Owner decisions (2026-09-26)

Record both in `docs/decisions.md`.

1. **AI agents are a possible future feature, not part of the migration.**
   - Stage 5 builds nothing for agents:
     - no `bagholder mcp`;
     - no agent token;
     - no tools generated from the route table;
     - no order tools;
     - no JSON Schema, so `schemars` is not added.
   - The route table's new descriptions and kinds stay only where the page uses them.
   - Mark `docs/architecture.md` §14 as a possible future feature, not built. Brief 09's decision 7 is superseded.
   - **The filings MCP server that ships today stays as it is, with one fix.**
     - The server is `disclosures-mcp`, listed in `rust/mcp/manifest.json`.
     - Its `disclosures_document` tool writes the filing to whatever path the caller names (`market/src/bin/disclosures-mcp.rs:159-166`), and filing text is written by outsiders.
     - The fix: it writes only into a folder of the app's own, under a name it makes itself, and the `dest` argument goes.
2. **Starting at login is the administrator's choice, made with their platform's own means.**
   - The app offers no option for it, sets nothing up, and has no `service` command.
   - `README.md` says how to run it at login on each platform.
   - The container already restarts itself: `restart: unless-stopped` in `docker-compose.yml`.

## Decided here (engineering)

- **Item 2, a key for access beyond loopback: taken as recommended.** Jupyter does the same.
  - At start, the app prints its address with the key in it, as Jupyter prints its `?token=` link. A browser signs in by opening that link, and the cookie holds the key from then on.
  - `docker-compose.yml:14` already publishes the port on the host's loopback. Keep it.
- **Item 4, signed releases: taken as recommended.**
  - Sparkle and Tauri require a signature on every update.
  - If CTO cannot set the repository secret itself, that one command is the owner's.

## Required changes

**A1. No browser store. Drop `web/src/lib/store/` and IndexedDB.**
- **It buys nothing here.**
  - The server runs on the same machine and holds the figures in memory, so answering them over loopback is as fast as reading the browser's database.
  - The page cannot load at all without the server. So the browser's copy never draws anything the server couldn't draw first.
- **It costs:**
  - a second copy of the person's financial data in every browser that ever opened the app, which Clear all cannot reach (brief 09, decision 4: nothing is left after Clear all);
  - a schema to migrate on every protocol change;
  - eviction, which Safari does after seven days without use.
- **The server meets the owner's requirement instead.** The requirement is that figures draw at once, and then only what changed arrives.
  - While running, the server answers from memory.
  - For a restart, measure how long the engine takes to build on the owner's book and state the number.
    - If it is well under a second, nothing more is needed.
    - If it isn't, the server answers from the figures it last saved until the engine is built, then sends what changed.
- **The second-open criterion becomes:** time from opening the page to the figures drawn, stated in Verification with a budget each case must meet.
  - It is measured with the server running, and again just after a restart.
  - The stream then sends only changes.
- Brief 01 §13 asked for the browser store. That part of brief 01 is withdrawn.
- **The rest of A stands:**
  - subscriptions per screen;
  - paging;
  - the change log, with resume inside its window;
  - conditional reads;
  - quotes on demand;
  - the markets context on the new wire;
  - the timers test.

**C1. The keychain, decided on evidence.**
- **The risk on the Mac:**
  - A keychain item trusts the program that created it. A release binary replaced by an update may be asked about again.
  - As the plan already notes, it may be refused while the app is started at login before the keychain unlocks.
- **Verification:** a real update of a release build on the Mac, then started at login, reads the session with no prompt and no refusal.
- If it can't, the 0600 file is the store on that platform. Report it; don't work around it.

## For the owner

Nothing.
