# Brief 72: Agents anywhere read the trade history: one agents-only address, reached through the person's own tunnel

## The owner's decisions (2026-10-20)

- **The agent connection isn't complete until cloud agents can use it.** "It can't only work for locally running agents, it has to work with Muse, Grok Bot, etc." The agents named are Meta's Muse, xAI's Grok Bot, OpenAI's Dots, and Claude. "Ideally, all of them should be made to work so that they can request Bagholder data."
- **There is no Bagholder-run relay.** The relay was offered as the one way that works for everyone, with no install and no account. The owner's answer: "no i'm not gonna do that." So the public address comes from a tunnel the person chooses.
- **An open router port was rejected.** It was proposed with a Let's Encrypt IP certificate, and the owner called it "insane".

Record these in `docs/decisions.md` in the owner's words, as part of the plan's PR. They replace the 2026-10-18 entry "Agents reach Bagholder from the same machine only, for now", and `SPEC.md` agents-5.

## 1. Should this exist, and how do the best teams build it?

This section comes first in every brief from now on, with a source for every claim.

**It should exist.** Each of these agents runs in its vendor's cloud. None can reach a program on the person's computer:
- **Grok Bot** "runs on a cloud computer and cannot reach your laptop". It takes a custom remote MCP server by its `https` address, with sign-in (OAuth) through a connect card, or a key it sends as a header ([Composio guide](https://composio.dev/content/how-to-add-mcp-servers-to-grok-bot)). Sign-in in Grok Bot is reported failing before any sign-in screen appears ([forum report](https://forum.cursor.com/t/grok-bot-custom-remote-mcp-oauth-never-starts-fetch-failed-same-url-works-in-cursor-ide/168188)). No official xAI page on Grok Bot's connectors was found.
- **Muse.** Meta's own help page says that for a service not on its list, Muse builds a custom connector, "which can involve retrieving API information from the service", and keeps credentials in its "Secure Credentials Store" ([Meta Help](https://www.meta.com/help/artificial-intelligence/1687253048996149/)). Third-party sources disagree on whether Muse takes an MCP address directly ([AnythingMCP](https://anythingmcp.com/guides/clients/muse) says yes, with sign-in; [AI Agents Library](https://www.aiagentslibrary.com/blog/meta-muse-mcp/) says no). Muse Code takes remote MCP over Streamable HTTP with OAuth 2.1 (same source).
- **Dots** uses the apps connected in ChatGPT ([vendor page](https://www.upload-post.com/openai-dots-social-media/); no official OpenAI page found). ChatGPT takes a public MCP address, or OpenAI's Secure MCP Tunnel ([OpenAI](https://developers.openai.com/api/docs/guides/secure-mcp-tunnels)).
- **Claude.ai** takes a public MCP address, with OAuth (Dynamic Client Registration or Client ID Metadata Documents) or none ([Anthropic help](https://support.anthropic.com/en/articles/11503834-building-custom-connectors-via-remote-mcp-servers)).

**The model to copy is Home Assistant's MCP server,** a self-hosted app that cloud agents read ([Home Assistant](https://www.home-assistant.io/integrations/mcp_server/)):
- it uses OAuth with PKCE and Client ID Metadata Documents, and the person approves in their own instance;
- agents see only what is exposed to them;
- outside access comes from its own cloud relay, or from any tunnel the person runs.

Bagholder has no relay (the owner's decision), so it takes the second route.

**This keeps the standing design.** `docs/architecture.md` §173–174 already says remote access goes through "a secure tunnel or reverse proxy the person already trusts". It also says the app doesn't manage certificates itself.

## 2. The design

### 2.1 One agents-only address, off until the person switches it on

- **A second listener**, bound to 127.0.0.1 only, serving the agent surface and nothing else.
  - It is never the page's port. The page, its interface and every write route are unreachable through it.
  - The tunnel forwards to this listener alone.
- **The same four read-only tools as `bagholder mcp`** (`SPEC.md` agents-2, -3), served in the server's own process. They read through the same `ReadOnly` client and its `GET` routes on loopback, so the guarantee from brief 54 holds unchanged:
  - the adapter's type;
  - the route test;
  - the source scan.
- **Off by default.** Switching it off closes the listener and ends every agent's access at once.

### 2.2 Three ways in, the same tools behind each

No single way of connecting covers every agent named, so all three are served:

1. **MCP with sign-in,** at `/mcp`, over Streamable HTTP. It follows the MCP specification's authorization exactly ([2026-07-28](https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization)):
   - protected resource metadata (RFC 9728) and a 401 carrying `resource_metadata`;
   - authorization server metadata (RFC 8414) with `code_challenge_methods_supported: ["S256"]`;
   - Client ID Metadata Documents, and Dynamic Client Registration for the clients that need it;
   - PKCE S256 only;
   - tokens bound to this resource (RFC 8707) and refused otherwise;
   - short-lived access tokens, with refresh tokens rotated on use;
   - exact redirect URI matching;
   - the hostname of a redirect URI shown on the approval screen.

   For Claude.ai, ChatGPT and Dots, Muse Code, and Grok Bot's connect card once it works.
2. **MCP with a key,** at its own path, `/mcp/key`. It advertises no OAuth metadata, so a client given a key isn't pushed into a sign-in flow. For Grok Bot today.
3. **A plain read-only JSON API,** at `/api/agents/…`, with an OpenAPI 3.1 description at a fixed path, and a key in the `Authorization` header. It has one route per tool, with the same arguments and answers. For Muse's custom connector, which is built from a service's API.

### 2.3 Approval happens only on the person's own Bagholder page

The person signing in an agent may be on a phone, and the address is public. So the approval page served through the tunnel grants nothing by itself:
- It shows the agent's name, its redirect hostname and a short code, and says "Approve this in Bagholder on your computer."
- The person's own page on the computer, which the tunnel can't reach, shows the same request and code with Allow and Deny.
- Only Allow there completes the grant.
- A request not approved within 10 minutes expires.

No password or account is added to Bagholder. Approval is proved by being at the computer that runs it. This follows the device authorization grant's model (RFC 8628) of approving on a trusted device.

Keys for ways 2 and 3 are made only on that same local page:
- each is named ("Grok Bot", "Muse");
- each is shown once and stored only as a hash;
- each can be withdrawn on its own.

### 2.4 Connected agents, in Settings

- One list holding every grant and every key, with:
  - the name;
  - the way in;
  - when it was made and last used;
  - a log of what it read (tool and arguments, never the answers);
  - one button to cut it off.
- Each agent is limited in how fast it can read.
- Every refusal is logged.

### 2.5 The public address: the person's own tunnel, with the common ones set up for them

Researched 2026-10-20:

| Tunnel | Account | Address | Who can read the traffic | Source |
|---|---|---|---|---|
| **Tailscale Funnel** | free | fixed, `*.ts.net` | no one: Funnel relays "cannot decrypt" | [Tailscale](https://tailscale.com/kb/1223/funnel) (beta) |
| **Cloudflare Tunnel** | free, plus the person's own domain | fixed | Cloudflare (TLS ends at its edge) | [Cloudflare](https://developers.cloudflare.com/cloudflare-one/connections/connect-networks/) |
| **ngrok** | free | one assigned dev domain | ngrok | [ngrok free plan](https://ngrok.com/docs/pricing-limits/free-plan-limits) |
| **zrok** | free (2 reserved shares) | fixed when reserved | per zrok's docs; confirm in the plan | [zrok](https://docs.zrok.io/docs/getting-started) |
| **The person's own reverse proxy** | none | their domain | only them | |

Ruled out:
- Cloudflare's quick tunnels: a new random address on every start, "for testing and development" only ([Cloudflare](https://developers.cloudflare.com/cloudflare-one/connections/connect-networks/do-more-with-tunnels/trycloudflare/)).
- Pinggy's free tier, Tunnelmole and localtunnel: random or rotating addresses, time limits, or unmaintained.
- Every agent saves the address when connected, so an address that changes breaks every agent.

**In Settings,** "Agents anywhere":
- **The person chooses; Bagholder never picks for them.** Most people have no tunnel yet, and a tool found on the machine says nothing about what they want for this. So Settings shows every supported tunnel side by side:
  - the account it needs;
  - the cost;
  - whether the address stays fixed;
  - who can read the traffic.

  Tailscale is marked as recommended because no one in between can read the traffic, but the choice is the person's.
- **Then it walks them through the one they chose, from nothing:**
  - installing it from its maker's official installer or download, linked from the step itself;
  - signing in to that provider;
  - only then, starting it.

  What's already done on the machine is checked only to skip that step, never to choose for them.
- **Bagholder starts the chosen tunnel itself, as its own child process,** pointed at the agents-only listener alone, and reads back the public address.
  - The child's lifetime is tied to the server's (`BAGHOLDER_EXIT_WITH`, #555), so the server stopping leaves no tunnel process running (no leftover processes).
  - A second start is refused while one runs.
  - A public address can also be entered by hand, for a reverse proxy.
- **Each provider's row says plainly who can read the traffic,** as in the table.
- **Before showing the address as ready,** Bagholder confirms it reaches this listener: a one-time value fetched from outside, through the address.
- **The listener answers only for the configured public hostname** (the `Host` check), and trusts no forwarded header from anywhere but loopback.
- **Copy buttons, one per agent,** with the exact steps from §1 for that agent.

### 2.6 What it costs (§16)

- **Idle:**
  - nothing while switched off;
  - while on, one listener and one tunnel process, no polling.
- **Each read** is the same `GET` the page makes.
- **The plan measures:**
  - the tunnel's memory on the Mac and on a Pi-class profile;
  - the added dependencies' build time and binary size: rmcp's `transport-streamable-http-server` feature, and any OAuth library chosen over a small typed implementation, with the reason.

## 3. Plan first, through the gate

This is a new surface on the public internet, so a plan PR comes before code. The plan states:
- the listener;
- each of the three ways in;
- the approval flow;
- the token and key storage;
- the tunnel handling per provider;
- the tests, with each test named against its rule.

**Tests the plan must hold, at least:**
- **Read-only:**
  - every route on the agents listener is a `GET` of a tool, or an OAuth endpoint;
  - no route on it reaches the page's interface;
  - the brief 54 scan extended to the new module.
- **Authorization:**
  - no token, a token for another resource, an expired token, a withdrawn grant and a wrong key are each refused with the status the specification names;
  - PKCE `plain` and a missing `code_challenge` are refused;
  - a redirect URI not exactly registered is refused;
  - a refresh token used twice ends the grant.
- **Approval:**
  - the public approval page alone grants nothing;
  - a request approved on the local page completes, a denied or expired one doesn't;
  - the local page is unreachable through the agents listener.
- **The key path:** `/mcp/key` serves no OAuth metadata.
- **Lifetimes:**
  - stopping the server stops the tunnel;
  - switching off closes the listener and refuses every agent at once;
  - a second tunnel start is refused.
- **Conformance:** the MCP inspector's checks against `/mcp`, and the OpenAPI description validated against the routes it describes.

**The last step needs the owner and can't be automated:** one real connection each from Grok Bot, Muse, Dots and Claude.ai, through the person's tunnel. The PR lists it as the owner's check, with the exact steps.

## Not included

- Any write by an agent.
- The page reached from another device (remote access to the app itself). That's still not now (`docs/architecture.md` §173).
- A Bagholder-run relay (the owner's decision).
- An open router port.
- The phone apps (on hold).

## Order

1. After D18, ahead of the rest of the queue: the owner asked for the agent work to be finished first.
2. Then the plan PR, the build, and the owner's check.
