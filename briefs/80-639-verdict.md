# Brief 80: #639 (agents anywhere) at e95e5190: not cleared, the changes

Line numbers are from the PR head e95e5190. Read area by area against plan #572 and brief 73 by seven readers. Each finding was checked by one or two independent checkers who tried to refute it in the code. Of 62 findings, 5 were refuted and are left out; the 57 confirmed are all below, several merged where they share a cause. Run here: the PR's own tests pass (agents 47, `tunnel_guard`, net 12, mcp 7, `http::tests` 10, clear 9; nothing left behind). Reproduced here: the stop of §1.2.

What is sound and stays: the listener on 127.0.0.1 with its own router; read-only held by `ReadOnly` and the source scan; PKCE S256 only; resource binding; refresh rotation and reuse ending the grant; hashed tokens and keys; approval only on the page's listener; Funnel in the foreground; the page never answering the agents' name; the live run's fixes.

Each change below comes with its test, and the plant named fails that test. The PR text lists them. Subtract where a change says so: the simplest bounded design, not a guard on an unbounded one.

## 1. Must fix

1. **The page's status never writes, and never takes the book while holding `app.state`.** `status.rs:176` calls `http::agents::status` inside the `Status` literal while `st` (`:142`) holds `app.state`; that reaches `approve::waiting` → `end_expired` (`approve.rs:81`), a BEGIN IMMEDIATE on a fresh connection, on every status step of every page, Agents on or off. With a CSV import holding a write transaction and calling `progress` (which needs `app.state`) per row, each side waits on the other until the 30 s busy timeout, every group. On a book not in WAL mode the empty commit fires the commit hook and the status step loops. Before this PR status never wrote (`main.rs:166`). The change: compute the agents' part before `st` is taken; expiry is filtered in the read (`made_at > now - REQUEST_LIFE`), and rows are removed only on a write the person or an agent makes. Test: with Agents off and on, the store's stamp is unchanged across `status::status(&app)`; plant: `end_expired` back in `waiting`.
2. **A stop sent to the tunnel guard stops the tunnel.** `main.rs:373-374` starts the generic `BAGHOLDER_EXIT_WITH` watch for every subcommand before dispatch, so the guard has a thread created before `block_stops` (`tunnel.rs:367`) with TERM, INT and HUP unblocked and no handler. `Tunnels::stop` sends SIGTERM (`tunnel.rs:290-296`): the default action ends the guard and `end_group` never runs. Reproduced here: a guard of `sh -c 'sleep 600 & echo $!; wait'` sent SIGTERM ended, and the tunnel's own child kept running. On macOS (no PDEATHSIG) the tunnel itself survives and the Funnel stays public, forwarding to whatever next binds the port. The same holds for Ctrl-C, and the SIGKILL race between the two watchers. The change: the guard installs its own handling before any thread starts (the generic watch not run for `tunnel-guard`). Tests: SIGTERM to the guard of that `sh` stand-in, the echoed child gone; the same after the stand-in server is killed. `tests/tunnel_guard.rs` uses that grandchild, since a direct child is ended by PDEATHSIG alone and the test passes with `end_group` deleted. Plant: `end_group(pid)` deleted at `tunnel.rs:399`.

## 2. Before sign-in, work and storage are bounded (brief 73, 1)

The listener's global bucket bounds answers, not work: every refusal is a book write.
1. **Refusals by an unknown caller are counted in memory**, by reason, never written per request: a wrong Host (421, which today stores the caller's Host verbatim, `listener.rs:196`), no key or a wrong key (`:287`, `:303`), a wrong token (`oauth.rs:82`), each 429 over the bucket (`listener.rs:229`). The panel shows the counts. A refusal of a known agent (over its rate) goes in that agent's own log, bounded as today. Any logged text (a Host, a read's arguments) is cut to a named length. Tests: a 64 KiB Host is refused with nothing written; 1,000 wrong-key calls leave the book's stamp where it was.
2. **A metadata-document client is not stored until Allow.** Today `keep_client` (`oauth.rs:187`) stores it on every `/oauth/authorize` before the redirect match, PKCE and the pending bound, and nothing prunes `kind='metadata'` rows (`book/src/agents.rs:155-162`, `:190-194`): up to 86,400 rows, about 700 MB a day, from refused calls. Until Allow it lives in its pending request, which is bounded at 5. Test: refused authorizations leave `agent_clients` unchanged; plant: `keep_client` back before the checks.
3. **An answered request ends.** `end_expired` removes only unanswered rows (`book/src/agents.rs:189`): a denied request answers forever, and an allowed one whose code is never exchanged mints a new code on every poll (`oauth.rs:342-353`), so `CODE_LIFE` limits nothing. A denied request ends when its page has read the denial; an allowed one when its code is exchanged or `CODE_LIFE` passes; the code is minted once. Polls of a known id are bounded by a named per-id interval (the exemption from the bucket, `listener.rs:101`, has no bound today). Tests for each.
4. **The metadata-document fetch** (`fetch.rs`, `net/src/client.rs`): the 8 KiB limit applies to the decoded body, decoded through a capped reader (today a gzip body under 8 KiB inflates to about 8 MB before `fetch.rs:144` checks it); the 5 s deadline covers the name's resolution (`fetch.rs:123` resolves before the deadline starts); the refusal page names no address the resolver returned and no connect error (today it maps the person's LAN and tailnet names for a stranger, `fetch.rs:127`, shown at `oauth.rs:240`). `public()` also refuses 192.0.0.0/24, fec0::/10 and every address of this machine's own interfaces. A test for each.

## 3. Sign-in

1. **A redirect host is a DNS name with a dot and a last label that is not all digits, or a public IP literal in its canonical form.** `redirect_ok` (`oauth.rs:110-114`) accepts `https://2130706433/cb`, `127.1`, `0x7f.1`, `localhost.`, `0.0.0.0`, `%6Cocalhost` and `localhost\x` (the authority ends at a backslash for browsers, not for `https_host`). Each refused at registration and at authorization.
2. **The loopback test's authorization half reaches the loopback check**: a metadata-document client whose document lists `http://localhost:33418/cb`. Plant: `oauth.rs:177` deleted. Today the exact match refuses every case first.
3. **`client_id` is required at the token endpoint** and must be the client the code or refresh token was issued to (`oauth.rs:403`, `:431`; RFC 6749 §4.1.3).
4. **The public page names the redirect host for a metadata client too** (plan §3; `oauth.rs:279-291`).
5. Tests: who is asking, with the client_id's host and the callback's host different (`tests.rs:21-22` share one); the plan's e2e for a metadata-document client; the status route asked with another request's id while that request is allowed (plant: the id filter dropped).

## 4. The gate and its names

1. **A public name or proxy Host that is loopback, `localhost` or an IP literal is refused** (`tunnel.rs:81-87`). Today `127.0.0.1:8765` as the own proxy's name makes the page's gate refuse the page itself, across restarts, and the target Host (`127.0.0.1:{agents.port}`, nginx's default) can be admitted. Test: choosing it is refused and `/api/status` stays 200.
2. Tests: Host equality is exact (plant: `ends_with` at `listener.rs:194`); each provider's gate name derived from its setup, Tailscale on 8443 included (plant: the `:8443` form dropped at `tunnel.rs:114`); the route-table scan reads every `.route(` on a line (plant: a second route chained on one line).
3. A port the OS chooses in 8765-8767 is not kept (`http/agents.rs:142-143`).

## 5. Tunnels

1. **With Agents on, the tunnel starts again when Bagholder starts.** Today `open()` re-binds the listener and nothing restarts the tunnel, so after an update or a reboot every agent is cut off with Agents still on (`http/agents.rs:151-160`). Test.
2. **Ready lasts only while the tunnel runs.** When it exits on its own, Ready and the address given to agents go (`http/agents.rs:84-89`, `:256`). Test.
3. One reachability check at a time; a second waits for the first (`tunnel.rs:511-517`).
4. Output that is not UTF-8 is read lossily and never stops the reader (`tunnel.rs:252`).
5. **Funnel serves on 443 unless the person chooses 8443.** Any local listener on 443 at the tailnet address triggers the move today (`agents/mod.rs:182-184`), whether or not it takes Funnel's traffic, and Claude.ai did not reach 8443 in the live run. When a program answers there, the panel says so and offers 8443 as the person's choice, as the plan has it ("it asks to use 8443"). Test.
6. On Linux, the step a person who is not root or Tailscale's operator still needs is shown as not done (from `status --json` where it says so, or from the refusal's own text at that step), never Done before a refused start.
7. `every_copy_the_repository_starts_on_another_folder_ends_with_its_starter` holds the variable actually passed, not a doc comment (plant: `guarded()` returning no env, `tunnel.rs:316`).

## 6. MCP, the tools and storage

1. **One MCP service per path, for GET and POST alike.** `listener.rs:117-118` builds two `StreamableHttpService`s, each with its own `LocalSessionManager`, so a session opened by POST is unknown to GET. Test: a session opened by POST is accepted by GET.
2. The per-agent rate is charged per tool call, not per HTTP request (initialize, list and notifications are not reads).
3. A test calls a tool over `/mcp` and over `/mcp/key` and finds the read in that agent's log (plant: the log call removed).
4. `AgentsBook` reads and writes only the four agents settings. Test.
5. Tests the plan names and the PR lacks: Clear's Agents kind removes the four settings and stops the listener and tunnel; the 1,000-line trim; the OpenAPI description validated and each described route called with its described arguments.

## 7. Reachability (brief 73, 7)

1. An answer that is one of this machine's own addresses never counts as the public path: the panel says "confirm from your agent" (`tunnel.rs:549-567`). Every public DNS answer must be public, as the plan's same check requires; today any one suffices.
2. A test of `PublicDns` over recorded DNS-over-HTTPS answers (A and AAAA, as 1.1.1.1 and 8.8.8.8 return them), so the shipped resolver and its parsing are held (`fetch.rs:163-208` are never exercised).

## 8. The page and the docs

1. **"Copy steps" never makes a key.** It uses the key the person made for that agent and way in, or says to make one (plan §3: keys are named by the person). Today it quietly makes a live key, and reuses one made for the other way in. The copy happens within the click, which Safari requires.
2. `docs/architecture.md` §12: the line saying remote access to the page is not built contradicts the line above it and `BAGHOLDER_HOSTS` (2.11.0).

## 9. The bounds' numbers

Tests hold the constants at the plan's values (5 pending, 20 unapproved, 60 a minute, 16 KiB, 10 minutes), and the 61st-call test covers each pre-sign-in route, not only the `.well-known` documents.

## Then

One new head with all of this, the PR text updated. I re-review the changed parts with the plants above, CI once, and #639 merges on clearance and green checks as 2.12.0.

## Re-review at d6349a88 (6fe224e5 and its TIMED_WAITS line), 2026-10-27

Eight readers checked each section against this brief, each gap checked by a refuter (29 confirmed, 2 refuted). Run here: the PR's tests pass (agents 78, `tunnel_guard` 2, `agents_restart` 1, net 13, mcp 8, `http::tests` 11, clear 10; nothing left). **49 plants, each an exact edit, run here: 47 caught by the test the PR names, among them §1.1, both halves of §1.2, every §2.1 to §2.4 rule, §3.1 to §3.5, all of §4, §5.1 to §5.7, §6.1 to §6.5 and §9.** The two not caught were written to show gaps (below). The changes:

### Must fix
1. **A copy of the book never starts the person's tunnel.** `open()` starts the tunnel whenever `agents.on` is 1 (`http/agents.rs:179-201`), so a copy run for some work (a scratch build on a copied book, the release check on the owner's Mac) starts their real tunnel: a Cloudflare named tunnel joins as a second replica answering on copied grants. A copy started with `BAGHOLDER_EXIT_WITH` (the repository's rule for every scratch copy) starts no tunnel. Test and plant.
2. **A tunnel the person stopped stays stopped across a restart.** Nothing records a Stop (`stop_tunnel`, `:316-325`), so the next start brings the public address back. The tunnel starts again only if it was running when Bagholder stopped, kept in the agents' own settings. Test and plant.
3. **The JSON API cuts a read's arguments** to `LOGGED_TEXT` as MCP does (`api.rs:99` is untouched; a 60 KB `sort` is stored whole, about 64 MB per key). Test over `/api/agents/*`; plant: the cut removed.
4. **A host whose last label is a number in any form browsers read is refused** (WHATWG's "ends in a number": all digits, or `0x` with hex digits). `0x7f.0x1`, `0x7f.0.0.0x1`, `0X7F.0X1` and `127.0x` pass `dns_name` (`fetch.rs:90`) and browsers send them to 127.0.0.x. At registration, authorization and the gate's names (§4.1). Tests with each; plant: the rule removed.

### Tests that must hold their rule
5. **The decoded ceiling in the path that ships:** a coded body through `get_at` (plant: `get_at` passing `u64::MAX` to `decoded`, `client.rs:811`).
6. **`public()` asks this machine's addresses:** a seam for the own-address list, so the test does not rest on CI's address being private (plant 49 here: `public()` not asking passed every test). `this_machine` reads the interfaces' addresses, not a bind (a bind succeeds for any address under `ip_nonlocal_bind`).
7. **`PublicDns::resolve` runs in a test**, over recorded DNS-over-HTTPS answers through a seam for its fetch: the A and AAAA loop, each server's path, the merge and the error rule (plant 48 here: AAAA dropped from the loop passed every test). Record an AAAA answer with an address from 8.8.8.8 (the recordings hold a CNAME only).
8. **Clear with a tunnel running:** a stand-in started first, gone after Clear (plant: the tunnel's stop removed from `switch_off`). Clear stops the listener and tunnel before it empties the tables.
9. **OpenAPI:** each described route called with every parameter its description lists, built from the description (plant: `parameters()` skipping the optional ones).

### Small, in the same head
10. The status poll's per-id interval is checked before the book is opened.
11. A refusal of a metadata document says only that it was not fetched, whatever the name resolved to.
12. An approved agent's open MCP sessions are bounded by a named constant.
13. The one-service test covers `/mcp` as well as `/mcp/key`.
14. The 'never written' test checks the book's stamp, as §2.1 asked; the component test for who is asking uses two hosts and checks the redirect host; a test of the canonical-form rule for IP literals; Grok Bot's key match tested.
15. The PR text no longer says the app moves Funnel to 8443.

Accepted as they stand: §3.5's metadata e2e replaced by the component test (an e2e run has no public address); §5.5's server half held by reading; §5.4's and §5.6's tests on their helpers; `every_copy` held by `the_tunnel_s_guard_is_given_this_process_to_end_with`; 'within the click' held by reading.
