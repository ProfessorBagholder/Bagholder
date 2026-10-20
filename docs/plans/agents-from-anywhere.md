# Plan: agents anywhere read the trade history through one agents-only address

Brief 72 (`architecture-briefs`, `briefs/72-agents-from-anywhere.md`, edb25c98, amended 5b36b5db §2.5). This plan covers the listener, the three ways in, approval, the storage of grants and keys, the tunnels, and the tests, each test named against the rule it holds. The plan comes before any code.

Paths are relative to `rust/crates` unless they start with `web/` or `docs/`. Facts about the code were read from `master` at the commit this PR is based on.

## For the owner to decide

Nothing is open. The owner's decisions of 2026-10-20 go into `docs/decisions.md` with this PR, in their words:
- cloud agents must work;
- there is no Bagholder-run relay;
- no router port is opened.

Together they replace the 2026-10-18 entry "Agents reach Bagholder from the same machine only, for now". `SPEC.md` agents-5 changes with them; see *What changes in the docs*.

The one step that needs the owner is the last one: a real connection from each agent. It can't be automated, and it's listed at the end with its steps.

## What exists today

- **`bagholder mcp`** (`server/src/mcp.rs`). It speaks stdio through `rmcp` 3.5.1, built with features `server`, `macros` and `transport-io` only. It offers four read-only tools: `trades`, `trade`, `positions` and `lists`.
  - The tools read the running server through `ReadOnly`, whose routes can only be built as `GetRoute`s (`server/src/http/mod.rs:182-225`): `/api/status`, `/api/view` and `/api/figures/detail`.
  - The tests that hold this are in `server/src/tests_mcp.rs`:
    - `every_route_the_read_client_names_is_a_get`;
    - `the_adapter_sends_one_request_a_get`, a source scan of `mcp.rs` that bans `POST`, `hyper::`, `TcpStream`, `Book::open` and more.
- **The server** is axum 0.8 on one listener, `127.0.0.1:8765-8767` (`server/src/main.rs:289-309`). Its `gate` (`http/mod.rs:350-400`) works like this:
  - it refuses a peer that isn't loopback;
  - it requires the Host header to be exactly `127.0.0.1:{port}`;
  - a write needs `sec-fetch-site: same-origin` or `X-Bagholder`.

  A tunnel pointed at this port is refused by the Host check, and must stay so.
- **Children tied to the server's life:** `server/src/exit_with.rs` (`BAGHOLDER_EXIT_WITH`, #555) uses a pidfd on Linux, kqueue on macOS and a process handle on Windows. It covers copies of Bagholder only.
- **There is no Settings page.** Settings sit in the header menu (`web/src/lib/Menu.svelte`), with submenus for Theme and Notifications. Changes go through typed `call('POST /api/…')` routes, and their state arrives on the `/api/status` stream.
- **No token or key is hashed and stored anywhere yet.** The precedent for a secret in the book is Web Push's VAPID key, in `settings` (`server/src/notify.rs:717`). The crates already used for this are `openssl` for `sha256`, `rand_bytes` and `memcmp::eq`. There is no `argon2`, `ring` or `subtle`.

## 1. The agents listener

- **A new module, `server/src/agents/`,** with its own axum `Router`. It shares no route with the page's router. Its files:
  - `listener.rs`: bind, the gate, switch on and off;
  - `mcp_http.rs`: the two MCP endpoints;
  - `api.rs`: the JSON API and its OpenAPI description;
  - `oauth.rs`: metadata, authorization, token;
  - `grants.rs`: storage;
  - `approve.rs`: the local approval routes;
  - `tunnel.rs`: the tunnel children.
- **Bound to `127.0.0.1` only,** on a port of its own, never 8765-8767. The port is chosen free at switch-on and kept in the book's settings (`agents.port`), so the tunnel's target stays put across restarts. It's started only when the person has switched agents on (`agents.on` in settings). It's closed when they switch it off: in-flight requests finish and nothing new is accepted. Every grant and key is refused from then on, because the listener isn't there.
- **Its gate** (`agents/listener.rs`), separate from the page's:
  - **Host** must equal the configured public hostname exactly. A request for any other host gets 421 Misdirected Request.
  - The peer must be loopback. A tunnel's connector runs on this machine and connects from 127.0.0.1.
  - No forwarded header (`Forwarded`, `X-Forwarded-*`) is trusted for anything but the log, and then only from loopback.
  - Methods are only those each route declares (below). Everything else gets 405.
  - CORS is off: no `Access-Control-Allow-*` header is ever sent. Cloud agents call from their servers, not from a browser.
- **What it serves,** as the complete route table with nothing else on it:

| Route | Method | Answers |
| --- | --- | --- |
| `/.well-known/oauth-protected-resource` | GET | RFC 9728 metadata for `/mcp` |
| `/.well-known/oauth-authorization-server` | GET | RFC 8414 metadata |
| `/oauth/register` | POST | Dynamic Client Registration (RFC 7591), limited (§2.1) |
| `/oauth/authorize` | GET | the public approval page (§3) |
| `/oauth/authorize/status` | GET | whether the request was approved on the computer (polled by that page only) |
| `/oauth/token` | POST | code and refresh-token exchange |
| `/mcp` | POST, GET | MCP Streamable HTTP, OAuth bearer |
| `/mcp/key` | POST, GET | MCP Streamable HTTP, key bearer, no OAuth metadata |
| `/api/agents/openapi.json` | GET | OpenAPI 3.1 description |
| `/api/agents/trades`, `/trade`, `/positions`, `/lists` | GET | the four tools, key bearer |
| `/agents/check/{value}` | GET | the one-time reachability value (§4.4) |

- **MCP over HTTP needs POST.** Streamable HTTP carries JSON-RPC in `POST` bodies ([MCP transports, 2026-07-28](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports)). So read-only isn't held by "every route is a GET". It's held where `bagholder mcp` holds it:
  - every tool reads through `ReadOnly`, whose routes can only be `GetRoute`s;
  - the agents module has no handle to the `Book`, the app's state or any write handler. It is given only a `ReadOnly` pointed at the server's own loopback address.

  §6 names the tests.
- **The tools are shared, not copied.** The tool bodies move from `mcp.rs` into `agents/tools.rs`, used by both `bagholder mcp` (stdio) and the listener (HTTP). `THEIR_WORDS` and `answered()` go with them, and each answer is the page's own for the same filters (`SPEC.md` agents-3).
- **`rmcp` gains `transport-streamable-http-server`** (with `server-side-http`, as that version names it), pinned like today. The archive's size is measured before and after, as #535 did, and the figure goes in the build PR. If the feature pulls in a second HTTP stack beside our axum and hyper, the build PR serves Streamable HTTP with a small handler on axum instead, held by the MCP inspector's checks (§6). Whichever one is chosen, the PR gives its reason and the figures.

## 2. The three ways in

### 2.1 MCP with sign-in, at `/mcp`

This follows the MCP authorization specification ([2026-07-28](https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization)) exactly:
- **Discovery:** a request with no token gets 401 with `WWW-Authenticate: Bearer resource_metadata="https://{host}/.well-known/oauth-protected-resource"` (RFC 9728). The authorization server metadata (RFC 8414) names:
  - `code_challenge_methods_supported: ["S256"]`;
  - `grant_types_supported: ["authorization_code", "refresh_token"]`;
  - `client_id_metadata_document_supported: true`;
  - the registration endpoint.
- **Clients:**
  - Client ID Metadata Documents first: the `client_id` is an `https` URL, which Bagholder fetches once per authorization through `bagholder-net`, with a size limit.
  - Dynamic Client Registration is for clients that need it (Claude.ai offers both). A registered client is stored with its exact redirect URIs. Unapproved registrations expire with their request (§3) and are never listed as agents.
- **PKCE:** S256 only. `plain` and a missing `code_challenge` are refused with `invalid_request`.
- **Resource binding (RFC 8707):** `resource` must be `https://{host}/mcp`. A token is bound to it, and a token for another resource is refused with 401 `invalid_token`.
- **Redirect URIs:** matched exactly, character for character, against the registered or metadata-declared list. There is no prefix or pattern matching. Each redirect URI's hostname is shown on both approval screens.
- **Tokens:**
  - **Access tokens:** opaque, 32 random bytes, valid for 1 hour.
  - **Refresh tokens:** opaque, 32 random bytes, rotated on every use. A refresh token used a second time ends the whole grant (OAuth 2.0 Security BCP, RFC 9700 §4.14.2), so a stolen one is only good until its owner next refreshes.
  - **Expiry** is checked when a token is used, never on a timer (§5).
- **For:** Claude.ai, ChatGPT and through it Dots, Muse Code, and Grok Bot's connect card once it works.

### 2.2 MCP with a key, at `/mcp/key`

- **The same Streamable HTTP and the same tools,** with `Authorization: Bearer <key>`.
- **No OAuth metadata.** No `WWW-Authenticate` with `resource_metadata` is sent, and no `.well-known` document names this path, so a client given a key isn't pushed into a sign-in flow.
- **For:** Grok Bot today (its sign-in is reported failing before any screen appears; brief §1).

### 2.3 The read-only JSON API, at `/api/agents/…`

- **One `GET` route per tool,** taking the tool's arguments as query parameters and returning the tool's answer as is.
- **The key** goes in `Authorization: Bearer <key>`.
- **An OpenAPI 3.1 description** at `/api/agents/openapi.json`, generated from the same argument types the MCP tools use. The routes and the description can't drift: §6 tests one against the other.
- **For:** Muse's custom connector, which "can involve retrieving API information from the service" ([Meta Help](https://www.meta.com/help/artificial-intelligence/1687253048996149/)).

## 3. Approval only on the person's own computer

The address is public, and the person approving may be on their phone, so nothing reachable through the tunnel grants anything by itself:
1. **The agent opens `/oauth/authorize`** on the public address. Bagholder records a pending request with:
   - the client's name and redirect hostname;
   - a six-character code (letters only, no lookalikes);
   - when it was made.
2. **The public page** shows the agent's name, the redirect hostname, the code, and "Approve this in Bagholder on your computer." It has no button that grants anything. It checks `/oauth/authorize/status` while open, and redirects with the code once the request is approved on the computer.
3. **The person's own page,** on the main loopback listener (which the tunnel can't reach), shows the same request:
   - a notice in the header, as other things that wait on the person do;
   - the same panel in the menu's new **Agents** submenu.

   **Allow** and **Deny** are writes on the main listener (`POST /api/agents/requests/{id}/allow` and `/deny`), behind its existing gate: loopback, the exact Host, and `X-Bagholder`.
4. **Allow completes the grant.** Deny, or no answer within 10 minutes, ends the request. Expiry is checked when the request is next touched, by either page or the token endpoint. No timer runs (§5).

This follows the device authorization grant's model of approving on a trusted device ([RFC 8628](https://www.rfc-editor.org/rfc/rfc8628)). No password or account is added to Bagholder.

**Keys** for ways 2 and 3 are made only on the local page:
- each one is named by the person ("Grok Bot", "Muse");
- it is shown once, with a copy button;
- only its hash is stored;
- each can be withdrawn on its own.

## 4. Settings: Agents, in the menu

The menu gains an **Agents** submenu, beside Theme and Notifications, opening a panel. There's no Settings page to put it in, and the brief's "in Settings" is this panel. It holds:

### 4.1 Connected agents

- **One list** of every grant (way 1) and every key (ways 2 and 3), each with:
  - its name;
  - its way in;
  - when it was made and last used;
  - **Cut off**, which withdraws it at once.
- **A log of reads per agent:** tool and arguments, never the answers. It keeps the last 1,000 per agent, trimmed when a read is written.
- **Refusals are logged:** bad token, wrong key, wrong host, over the rate.
- **A rate limit per agent:** 60 reads a minute, with a burst of 10. It's a token bucket computed when a read arrives, from the last read's time; nothing refills it on a clock. Over the limit, the answer is 429 with `Retry-After`.

### 4.2 Choose a tunnel

All supported tunnels are shown side by side, and **Tailscale Funnel is marked recommended** because no one in between can read the traffic. Bagholder never picks for the person (brief §2.5, amended).

| Tunnel | Account | Cost | Address stays fixed | Who can read the traffic | Source |
| --- | --- | --- | --- | --- | --- |
| **Tailscale Funnel** (recommended) | Tailscale, free | free | yes, `{machine}.{tailnet}.ts.net` | no one in between: Funnel relays pass encrypted TLS that ends on this machine | [Tailscale Funnel](https://tailscale.com/kb/1223/funnel) (beta) |
| **Cloudflare Tunnel** | Cloudflare, free, with the person's own domain on Cloudflare | free | yes, a name on their domain | Cloudflare: TLS ends at its edge | [Cloudflare Tunnel](https://developers.cloudflare.com/cloudflare-one/connections/connect-networks/) |
| **ngrok** | ngrok, free | free | yes, the one dev domain the account is given | ngrok: TLS ends at its edge | [ngrok free plan](https://ngrok.com/docs/pricing-limits/free-plan-limits) |
| **zrok** | zrok, free | free (2 reserved shares) | yes, when reserved | zrok's operators: a public share's TLS ends at zrok's frontend, whose load balancers log "all of the URLs that were accessed, as well as other information (headers, etc.)" | [zrok public sharing](https://netfoundry.io/docs/zrok/concepts/sharing-public) |
| **My own reverse proxy** | none | — | the person's own domain | only them | — |

Ruled out, as the brief did: Cloudflare quick tunnels, Pinggy free, Tunnelmole and localtunnel. Each gives a random or rotating address, or a time limit, and an agent keeps the address it was given.

### 4.3 Set up the one chosen, from nothing

Each provider has a walkthrough. A step already done on this machine is shown done and skipped, but never chosen for the person. Each step links to its maker's own page:

| Provider | Install | Sign in | Start (Bagholder's child) | Public address read from |
| --- | --- | --- | --- | --- |
| Tailscale | [official installer](https://tailscale.com/download) | `tailscale up` (the app's own sign-in); then turning on HTTPS certificates and the `funnel` node attribute, from the URL `tailscale funnel` prints when they're missing | `tailscale funnel --https=443 127.0.0.1:{agents.port}`, in the foreground, never `--bg`, which would persist past Bagholder | `tailscale status --json` → `Self.DNSName` |
| Cloudflare | [cloudflared download](https://developers.cloudflare.com/cloudflare-one/connections/connect-networks/downloads/) | `cloudflared tunnel login`; `cloudflared tunnel create bagholder-agents`; `cloudflared tunnel route dns bagholder-agents {name}` | `cloudflared tunnel run --url http://127.0.0.1:{agents.port} bagholder-agents` | the name the person entered for the DNS route |
| ngrok | [ngrok download](https://ngrok.com/download) | `ngrok config add-authtoken {token}` (copied from their dashboard) | `ngrok http --url={dev domain} 127.0.0.1:{agents.port}` | the dev domain shown in their dashboard, entered once |
| zrok | [zrok install](https://docs.zrok.io/docs/guides/install/) | `zrok enable {token}`; `zrok reserve public 127.0.0.1:{agents.port} --unique-name {name}` | `zrok share reserved {name}` | `https://{name}.share.zrok.io` |
| Own proxy | — | — | — (the proxy is theirs) | entered by hand |

- **Ports:** Funnel accepts only 443, 8443 and 10000 ([funnel CLI](https://tailscale.com/kb/1311/tailscale-funnel)). Bagholder uses 443 unless the person's Funnel already serves something there, in which case it asks to use 8443.
- **Commands run with fixed arguments,** never through a shell. The binary is found by the provider's documented install location, or on `PATH`, and is shown to the person before its first start.

### 4.4 Starting it, and knowing it works

- **One child at a time.** A second start is refused while one runs, and the refusal names the running one.
- **The child's life is tied to the server's.** Third-party binaries don't read `BAGHOLDER_EXIT_WITH`, so Bagholder starts them through `bagholder tunnel-guard -- {command}`. That's a small subcommand run with `BAGHOLDER_EXIT_WITH` (`exit_with::watch`, unchanged) which starts the tunnel as its own child in a new process group, and stops the group when either side ends:
  - on Linux, it also sets `PR_SET_PDEATHSIG`;
  - on macOS, the same kqueue `NOTE_EXIT` path `exit_with` uses;
  - on Windows, a job object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`.

  `every_copy_the_repository_starts_on_another_folder_ends_with_its_starter` extends to this spawn.
- **Its output is read as it arrives,** never polled. A child that exits is said in the panel with its last lines.
- **Before the address shows as ready,** Bagholder makes a one-time random value and fetches `https://{address}/agents/check/{value}` from outside, through `bagholder-net`. The listener answers that path only while the check is open. Only a match shows "Ready". A mismatch or failure is said with what was fetched.
- **Copy buttons, one per agent,** each with that agent's exact steps:
  - Claude.ai's custom connector;
  - ChatGPT's connector, which Dots uses;
  - Muse's custom connector;
  - Grok Bot's remote MCP.

  Each takes the right address (`/mcp`, `/mcp/key` or the OpenAPI URL) and, for a key, the key made for it.

## 5. Storage and what it costs

- **Book tables,** in one migration. That makes this a gated change of stored data, and the migration's snapshot test covers it:
  - `agent_clients`: registered and metadata clients, their redirect URIs;
  - `agent_requests`: pending authorizations, with code, client, redirect, PKCE challenge, `made_at`;
  - `agent_grants`: one per approved client;
  - `agent_tokens`: each a SHA-256 hash, kind (access or refresh), grant, `expires_at`, and `used_at` for refresh;
  - `agent_keys`: name, way, SHA-256 hash, made, last used, withdrawn;
  - `agent_reads`: agent, tool, arguments, at, or a refusal.
- **Settings:** `agents.on`, `agents.port`, `agents.tunnel` (the provider chosen), `agents.host`.
- **Hashing:** tokens and keys are 32 random bytes from `openssl::rand::rand_bytes`, so plain SHA-256 is enough to store them, with no password hash needed. They're compared with `openssl::memcmp::eq` on the hashes. Nothing new is added to the dependencies.
- **Clear data:** a new kind, **Agents**, empties every agent table, the four settings, and stops the listener and tunnel. Clear all includes it. `clear.rs`'s every-table test holds it.
- **Idle cost (§16):**
  - while off: nothing, no listener and no child;
  - while on: one listener and one tunnel child, with no timer and no polling;
  - every expiry (codes, tokens, requests) and the rate bucket are checked when touched;
  - `TIMED_WAITS` gains at most the listener's drain on stop, counted like the page listener's.
- **Each read** is the same `GET` the page makes, through `ReadOnly`.
- **The build PR measures:**
  - the release archive's size, with and without the new rmcp feature;
  - build time;
  - the tunnel's memory on the Mac and on a Pi-class profile, for Tailscale and cloudflared;
  - the capacity job: `agent-trades-page` stays reported-only (#554), and an HTTP read through the listener is added as a reported row.

## 6. Tests, each against its rule

**Read-only:**
- `agents::tests::the_route_table_is_exactly_the_planned_one`: the agents router's routes and methods equal §1's table, nothing more.
- `agents::tests::no_agents_route_reaches_the_page`: every page route (`http::route_table()`) asked on the agents listener gets 404, `GET` and `POST` alike.
- `agents::tests::the_agents_module_holds_no_write_handle`: a source scan of `server/src/agents/` in the shape of `the_adapter_sends_one_request_a_get`. It finds no `Book::open`, `AppState`, `Figures`, `client::send`, `"POST"` request, `hold_home` or `std::process::Command` outside `tunnel.rs`. The tools reach the server only through `ReadOnly`.
- `tests_mcp`'s two existing tests, run over `agents/tools.rs` as well.

**Authorization (the status the specification names, for each):**
- no token → 401 with `resource_metadata`;
- a token for another resource → 401 `invalid_token`;
- an expired token → 401;
- a token of a withdrawn grant → 401;
- a wrong key on `/mcp/key` and `/api/agents/*` → 401;
- PKCE `plain`, and a missing `code_challenge` → 400 `invalid_request`;
- a redirect URI not exactly registered (trailing slash, other port, other path, a prefix) → refused, never redirected;
- a refresh token used twice → the grant ends, and its access token is refused next;
- a Client ID Metadata Document whose `client_id` doesn't match its URL → refused.

**Approval:**
- `the_public_page_alone_grants_nothing`: every path on the agents listener, given the request's code, never yields an authorization code.
- `a_request_allowed_on_the_computer_completes`;
- `a_denied_request_does_not`;
- `an_expired_request_does_not` (with the clock set past 10 minutes, no sleep);
- `allow_and_deny_are_unreachable_through_the_agents_listener`.

**The key path:**
- `/mcp/key` sends no `WWW-Authenticate` naming `resource_metadata`;
- no `.well-known` document names it.

**Gate:**
- a request with another Host → 421;
- a peer that isn't loopback → refused;
- `X-Forwarded-For` from a non-loopback peer is not logged as the client;
- no `Access-Control-Allow-*` header on any answer.

**Lifetimes:**
- `stopping_the_server_stops_the_tunnel`: a stand-in tunnel binary is started through `tunnel-guard`, the server stops, and the stand-in is gone;
- `switching_off_closes_the_listener_and_refuses_every_agent_at_once`;
- `a_second_tunnel_start_is_refused_naming_the_first`;
- `every_copy_the_repository_starts_on_another_folder_ends_with_its_starter`, extended.

**Rate and log:**
- the 11th read in a burst gets 429 with `Retry-After`;
- the log keeps tool and arguments, never answers;
- refusals are logged.

**Conformance:**
- the MCP inspector's checks run against `/mcp` and `/mcp/key` in CI;
- the OpenAPI description is validated, and each described route is called with the described arguments.

**Clear:** Clear data's Agents kind empties every agent table, and the every-table test names them.

**The page (e2e):** `web/e2e/agents.spec.ts`:
- the Agents panel lists the tunnels side by side, with Tailscale marked recommended and nothing preselected;
- a pending request shows its code with Allow and Deny;
- a key is shown once.

## 7. The owner's check: one real connection each

These can't be automated, and the build PR lists them with these steps:
1. In Bagholder, switch on Agents, choose a tunnel, and follow its steps until it shows **Ready**.
2. **Claude.ai:** Settings → Connectors → Add custom connector, with the copied `/mcp` address. Approve the code shown in Bagholder on the computer, then ask Claude for the last five trades.
3. **ChatGPT, for Dots:** add the copied `/mcp` address as a connector, approve the same way, and ask Dots for the open positions.
4. **Muse:** make a key named "Muse" and give Muse the OpenAPI address and the key for a custom connector. Ask for the lists of accounts and tags.
5. **Grok Bot:** make a key named "Grok Bot" and add the `/mcp/key` address with the key as a header. Ask for one trade's executions. If its connect card works by then, try `/mcp` instead.
6. For each, check the Agents list shows its reads, then **Cut off** one and see its next read refused.

## What changes in the docs

These go in this PR:
- **`docs/decisions.md`, 2026-10-20,** in the owner's words:
  - "It can't only work for locally running agents, it has to work with Muse, Grok Bot, etc."
  - "no i'm not gonna do that" (the relay);
  - the open port called "insane".

  These replace the 2026-10-18 entry, which is marked superseded with a pointer.
- **`SPEC.md` agents-5** reads: agents elsewhere reach the same read-only tools through one agents-only address, off until the person switches it on, reached through a tunnel the person chooses, with approval only on their own computer (this plan).
- **`docs/architecture.md` §12 and §14:** remote *agents* are built through the agents-only listener. Remote access to the page itself is still not built.

## Not included

- Any write by an agent.
- The page reached from another device.
- A Bagholder-run relay.
- An open router port.
- The phone apps.

## Order

1. This plan PR, through the gate.
2. The build, as one PR:
   - the module;
   - the migration;
   - the menu panel;
   - the tests above;
   - the archive and memory figures.
3. The owner's check (§7). The release follows it.
