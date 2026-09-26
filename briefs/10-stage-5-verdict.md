# Brief 10: verdict on the stage 5 plan

**Reviewed:** `docs/plans/stage-5-interface-and-running.md` at `46b3cffc`. Checked against `master` at `09327c99` and against outside practice (`briefs/reference.md`).

**Verdict: Go with changes.**
- **Part A** is being built now: make change A1 before going further.
- **Parts B, C and D:** build them with the changes below. C and D also wait for the owner's answers on items 1 and 3.

**The gate comes before the build.**
- A heavy lift is built after its verdict (brief 01). Part A started before this one, and stage 4's plan never came to the gate.
- Stage 4 was read afterwards for the four order defects of brief 01. All four are addressed:
  - a lost answer is read back, not marked Failed;
  - no POST is re-sent;
  - no list is cut at 200;
  - rows being sent count as in flight.

## The owner's items

Under brief 09's decision 1, only items 1 and 3 are the owner's; they go to the owner. Items 2 and 4 are engineering, decided here:

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

**C1. MCP tools are chosen, not generated one per route.**
- **Anthropic's guidance on tools for agents:**
  - "A common error we've observed is tools that merely wrap existing software functionality or API endpoints"
  - "More tools don't always lead to better outcomes"
  - "Too many tools or overlapping tools can also distract agents"
  - Its advice is "a few thoughtful tools targeting specific high-impact workflows".
- **What becomes a tool:**
  - A route becomes a tool only when the table marks it for agents, with a description written for an agent.
  - Routes shaped for the page are not tools: screen subscriptions, the ticket's quote, a card's document.
  - The marked set is the few operations an agent needs:
    - reading the figures and trades in scope, a position, and the journal;
    - the market reads;
    - writing the journal and the watchlist;
    - orders, per the owner's answer on item 3.
- It is still declared once, in the same table. There is no second interface.
- **Criterion:** `tools/list` equals the marked set, and a test fails on a route marked for agents without a description.

**C2. The keychain, decided on evidence.**
- **The risk on the Mac:**
  - A keychain item trusts the program that created it. A release binary replaced by an update may be asked about again.
  - As the plan already notes, it may be refused while the app is started at login before the keychain unlocks.
- **Verification:** a real update of a release build on the Mac, then started at login, reads the session with no prompt and no refusal.
- If it can't, the 0600 file is the store on that platform. Report it; don't work around it.

**D1. Starting at login follows the owner's answer on item 1.** Record the owner's answer in `docs/decisions.md`. Until the owner has answered, a release sets nothing up at login.

## For the owner

- **Item 1: starting at login.**
- **Item 3: whether AI agents can place orders.**
