# Brief 73: Verdict on plan #572 (agents anywhere): Go with seven changes

## Verdict

**Go with seven changes,** made in the plan before the build starts.

The plan follows brief 72 and the MCP authorization specification point by point:
- read-only is held where `bagholder mcp` already holds it: the `ReadOnly` type, an exact route table, a source scan;
- there are no timers;
- the person chooses the tunnel;
- the tunnel's life is tied to the server's.

What it's missing is the part that faces strangers. Every route before sign-in can be called by anyone on the internet, and the plan gives those routes no bound and no SSRF guard. It also trusts two behaviours of the tunnels without a source: what `Host` they send, and whether a check from this machine really goes through the public path.

## Required changes

### 1. Everything reachable before sign-in is bounded

Anyone can call `/oauth/register`, `/oauth/authorize`, `/oauth/authorize/status` and `/agents/check/*`. The listener sees every peer as loopback and trusts no forwarded header, so there is no per-caller limit. The bounds are therefore global, and each is held by a test:
- **pending requests:** at most 5 at once. A sixth is refused with 429 until one ends;
- **registered clients that were never approved:** at most 20. The oldest is dropped first;
- **pre-sign-in calls:** at most 60 a minute in total, as a bucket computed when touched (§5's model);
- **request bodies on these routes:** a fixed size limit.

### 2. The Client ID Metadata Document fetch is guarded against SSRF

The specification says to "consider Server-Side Request Forgery (SSRF) risks" ([authorization, CIMD security](https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization)). Here an unauthenticated stranger supplies the URL that Bagholder fetches. The fetch:
- is `https` only, with no redirects followed;
- resolves the name, and refuses loopback, private, link-local, CGNAT (100.64/10, which includes Tailscale's addresses) and unique-local addresses. It connects only to the address that was checked, so a second DNS answer can't switch it;
- has a size limit and a timeout.

*Tests:* a `client_id` naming each refused range, and one that redirects, are each refused with nothing fetched.

### 3. Loopback redirect URIs are refused

Cloud agents never redirect to `localhost`; local agents use stdio. The specification warns that a `localhost` redirect lets anyone impersonate a client whose metadata lists one ("Localhost Redirect URI Risks", same page). So a redirect URI on `localhost`, `127.0.0.0/8` or `[::1]` is refused, in registration and in authorization alike.

*Test:* each form is refused.

### 4. The approval screens say who is really asking

- For a Client ID Metadata Document client, the identity shown is the **`client_id`'s host** ("claude.ai"), with its name beside it.
- For a client registered dynamically, the name is marked **"name given by the agent, not checked"**, with the redirect host shown as the identity.
- The local Allow screen says: "Allow only if you just started connecting this agent, and its page shows code ABCDEF."
- A request the person didn't start is the one attack left, and these words are the defence.

*Test (e2e):* a dynamically registered client's name is shown marked as unchecked.

### 5. The request id is not the code

The public page polls `/oauth/authorize/status` by request id, and an approved status hands back the redirect with the authorization code. So:
- the id is 128 random bits, separate from the six-letter code;
- the status route answers only for that id;
- the code is never accepted in its place.

PKCE already stops a stolen authorization code from being used. This change also keeps strangers from probing which requests exist.

### 6. Each tunnel's `Host`, and its lifetime, are sourced, not assumed

The gate refuses any `Host` but the public name. Whether each tunnel passes the visitor's `Host` or rewrites it to the target isn't stated in the plan. Tailscale's Funnel page doesn't say either ([Funnel CLI](https://tailscale.com/kb/1311/tailscale-funnel)).

For each provider the plan states, with a source (its docs or its code at a pinned version):
- the `Host` the listener receives;
- what the gate checks for that provider. If it's the target's host, the gate checks that instead, never a forwarded header;
- what happens to the public route when the tunnel process is killed, not stopped cleanly. For Tailscale's foreground mode in particular: the docs only say a foreground Funnel isn't restored after a restart.

### 7. The reachability check has to go through the public path

From this machine, a Funnel hostname resolves over the tailnet, not the public route. So a check from here can pass while the public route is broken. Tailscale's forum advice is to test from outside the tailnet ([forum](https://forum.tailscale.com/t/funnel-not-available-not-set/4092)). The same kind of shortcut can apply to a person's own proxy on their LAN.

The plan says how the check reaches the public route for each provider, for example by resolving through public DNS and connecting to the public answer. Where it can't, the panel says "reachable from this computer; confirm from your agent". It never shows "Ready" for a check that didn't leave the machine.

## Also

- **Conformance.** "The MCP inspector's checks" isn't a CI tool. Use `@modelcontextprotocol/conformance` in server mode, pinned to an exact version that covers 2026-07-28. That's an alpha today, so it's run as reported-only until it's stable ([conformance](https://github.com/modelcontextprotocol/conformance)).
- **The decisions.md entries and SPEC agents-5 are right as written.**

## Not changed

These parts of the plan stand:
- the route table;
- read-only held by type, the table and the scan, not by `GET`;
- `tunnel-guard`;
- the Agents submenu;
- the one migration and the Clear kind;
- SHA-256 over 32 random bytes;
- the rmcp-or-axum choice measured in the build PR;
- the owner's check in §7.

Order as the plan gives it: the plan amended, then one build PR, then the owner's check.
