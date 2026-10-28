# Brief 81: Agents, 2.14.0: native clients, and sign-in by a typed code

The owner agreed the direction on 2026-10-28 after 2.13.0's panel ("it's not clear what is going on, what the user should do, why a URL with /mcp/key is created, what exactly to do on the agent's end"), from the proposal they reviewed (https://claude.ai/artifact/4jnHn2RN6wxkoCqiVey4uU). One release, 2.14.0. Line numbers are from master at 2.13.0.

## 1. Loopback redirects are accepted: brief 73 change 3 withdrawn

Brief 73 change 3 refused every loopback redirect on the premise "cloud agents never redirect to localhost; local agents use stdio". Grok Bot is a desktop MCP client whose Connect starts the standard OAuth flow with `http://localhost/callback`, and the MCP specification allows it: "All redirect URIs MUST be either localhost or use HTTPS" (Communication Security). The premise was false; the refusal goes.

- `redirect_ok` accepts `http://localhost`, `http://127.0.0.1` and `http://[::1]` with any port, as RFC 8252 §7.3 has them for native apps, and https to a public host as now. Anything else stays refused (the number forms of brief 80's re-review stay refused: a loopback client names `localhost` or the canonical literals, nothing else).
- The redirect is still matched exactly against the registered or document-listed URI, except the port of a loopback URI, which RFC 8252 §7.3 lets vary. PKCE S256 stays required.
- **Who is asking:** a client whose redirect is loopback is an app on the person's own computer, and any app can claim any name. Its name is shown marked as given by the app, never checked, as a dynamically registered client's is (brief 73 change 4; the specification's "Localhost Redirect URI Risks").
- **The whole flow reviewed against the specification's current authorization section, section by section**, in the PR: each requirement, where the code meets it, and its test.
- **Test:** an end-to-end run with a stand-in native client: register with a loopback redirect, authorize, the typed code, the redirect to the loopback URI with the code, the token, an MCP read. Plant: loopback refused again.

## 2. Sign-in by a code typed on the public page

The person's proof that they started a sign-in moves from the panel's Allow to typing, on the public sign-in page their agent opened, a code Bagholder shows only on their own computer (the page's listener, which the tunnel cannot reach). The code can be guessed on the public page by anyone who reaches the address, so its bounds are what make it safe:

1. **One code per pending request**, shown only in the person's own panel beside who is asking, never sent through the agents' listener. Six digits are acceptable only with the bounds below.
2. **A code ends** when it is entered right, after 10 minutes, or after 3 wrong entries for that request; the request ends with it.
3. **Wrong codes pause sign-in for everyone.** After 5 wrong entries in all, across every request, no code is accepted until the person resumes sign-in in their panel, which says in a plain sentence that codes were entered wrong. A stranger therefore gets at most 5 guesses before the person sees it: 5 in a million for six digits. State this bound in the PR.
4. Entries count against the pre-sign-in allowance as every call there does.
5. Tests: a right code completes the grant; a wrong one is refused and counted; the fourth entry on one request ends it; the sixth wrong entry overall is refused while paused, and a resume in the panel lets the next right code through; an expired code is refused. Plants: the per-request limit removed; the pause removed; expiry removed.

## 3. The panel

Built to the reviewed proposal: the agent first; the tunnel's checklist showing only what is missing; one status line while the address gets ready, with the bounded retry and its limit (brief 80's 2.13.1 conditions); that agent's own steps with Copy; the key case for an agent that takes a key; the code to type; your agents; **Remove** in place of Cut off, a removed agent leaving the list (its grant or key withdrawn at once, as Cut off did); plain words only, held by the scan; every state screenshotted in light and dark, judged by the Architect and shown to the owner before release.

## 4. Not included

Writes by agents; remote access to the page; a relay; an open port; changes to the read-only tools.
