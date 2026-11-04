# Plan: the page behind Open from anywhere renews its sign-in by itself, and a lost connection comes back by itself

## For the owner to decide

Nothing open: the owner's decision is in `docs/decisions.md` (2026-11-04).

## Scope

**In it:**
- **The sign-in renews itself.** When the Cloudflare Access sign-in in front of the page has expired, the page reloads itself once. The browser passes through Access and the person's login method without a stop, and the page reopens on what it showed, drawn from what it kept.
- **No Access page to press through.** The Access application Bagholder makes uses Cloudflare's instant authentication (`auto_redirect_to_identity`): it has one login method, Bagholder's own GitHub sign-in, so Access sends the person straight to it. Applied to the application Bagholder made before as well, once, at start, with the token Bagholder already keeps.
- **A lost connection comes back by itself.** When the stream drops and the browser has stopped retrying, the page opens it again: at once when the network returns or the page is shown again (`online`, `visibilitychange`, `pageshow`), and while the page is visible and online, after 1 second, then doubling to 30 seconds between attempts, until it opens. Each attempt is preceded by the status probe below, so an expired sign-in renews instead of retrying. An EventSource fails for good on any answer but a stream: a proxy's 502 while the container is replaced (the Update button's own restart), Cloudflare's 530 while a tunnel is down, Access's redirect. This is a connection being reopened, not data polled: architecture §13 holds.
- **A lost connection is said on every screen.** While the page is not connected, the header's status line says so, with Retry, whatever the page shows. Today it is said only on a screen with nothing loaded yet (`App.svelte`, the `#page` empty state).

**Out of it:**
- The session's length: it stays 24 hours (`remote::SESSION`). With the renewal invisible, a longer one buys nothing and holds a stolen device's cookie longer.
- The agents' address: it has no Access in front of it (agents sign in with their own keys) and is unchanged.
- A sign-in the person's login method itself asks for (GitHub signed out, a new device): that page is shown, as it must be.

## What happens now

Read from the code, 2026-11-04, at f8806821:
- **The sign-in:** `remote::application` makes the Access application with `session_duration` `SESSION` (24h) and one login method, and leaves `auto_redirect_to_identity` as Cloudflare's own default, off (`the_access_application_lets_in_the_email_alone_through_bagholder_s_own_login` asserts it is absent). So an expired sign-in shows Access's own login page with a button for the one method.
- **The stream:** `live.svelte.ts` `connect` opens one `EventSource`. When Access answers its reconnect with a redirect to its login page (another origin), the browser fails the connection: `readyState` goes `CLOSED` and the browser does not retry. `onerror` sets `conn.error = 'Could not reach Bagholder.'`.
- **Where that is said:** only in `App.svelte`'s `#page` branch for a page with no book yet (`!book.data || !(showingEmpty || tabReady)`). With a book on screen nothing says it: the figures stand still with no word. Since 2026-09-26 (cfef28e8).
- **Bagholder itself never redirects** an `/api/` request: a redirect answering one is from something in front of it.

## How others do it

- **Cloudflare** ("Session management", Cloudflare One docs, read 2026-11-04): "Cloudflare will automatically issue a new application token if the global token is still valid", otherwise the person re-authenticates with the login method. Pages that rely on sub-requests "can block sub-requests due to an expired Access token without prompting the user to re-authenticate"; Access can "provide a `401` response on sub-requests with an expired session token" when they carry `X-Requested-With: XMLHttpRequest`, and Cloudflare recommends using it "to either force a page refresh" or to show that the session has expired.
- **Instant authentication** (Cloudflare One docs, device enrollment, read 2026-11-04): "If you plan to only allow access via a single IdP, turn on Apply instant authentication. End users will not be shown the Cloudflare Access login page. Instead, Cloudflare will redirect users directly to your SSO login event." The application's field is `auto_redirect_to_identity`.
- **GitHub** returns at once from an authorization the person has already granted while they are signed in to GitHub.

## Approach

1. **Telling an expired sign-in from a lost connection.** When the stream fails and the browser has stopped retrying, the page asks `GET /api/status` with `redirect: 'manual'` and `X-Requested-With: XMLHttpRequest`. A redirect (`opaqueredirect`) or Access's `401` is the sign-in in front of Bagholder having expired. No answer at all is a lost connection, and Bagholder's own error is that error.
2. **Renewing.** An expired sign-in reloads the page (`location.reload()`), a top-level navigation that Access and the login method answer with redirects only. It waits while the person is typing into a field or a dialog is open, and reloads when that ends; drafts are kept as now. One renewal per minute at most (kept in `sessionStorage`), so a sign-in that cannot complete shows its own page instead of reloading again.
3. **Reconnecting.** On `online`, on `visibilitychange` to visible and on `pageshow`, a closed stream is opened again (`connect`) at once. While the page is visible and online, a closed stream is also tried again 1 s after it closed, then 2, 4, 8, 16 and 30 s apart, until it opens; each try asks the probe of step 1 first. The browser's own retry is left as it is while it still retries. The reload after an update (`onRestart`, fired by the stream's `hello`) then comes as soon as the new build's stream opens.
4. **Saying it.** While the stream is not open, the header's status line says so on every screen, with Retry, in the line's existing error style: `Reconnecting to Bagholder…` while it is being tried again, `Could not reach Bagholder.` once a try has found it unreachable, and `Signing in again…` while the sign-in renews. The `#page` empty state keeps its own as now.
5. **Instant authentication.** `remote::application` sets `auto_redirect_to_identity: true`. At start, with Open from anywhere on and a kept token, Bagholder reads its own application (`GET /accounts/{account}/access/apps/{Kept.app_id}`) and, only when the field is off, sends the whole application `remote::application` makes, with the field, to it (`PUT`, which Cloudflare's reference does not promise merges, so never a partial body), keeping its id and audience. An application the person made is never touched. A refused update is said where Open from anywhere's other failures are, and the renewal still works through Access's page.

## Acceptance criteria

- [ ] With a book on screen, a stream that fails is said in the header's status line with Retry. A browser test with the stream refused after the book is drawn. Plant: the line shown only on an empty page fails it.
- [ ] A stream whose reopening is answered with a redirect to another origin reloads the page once, and not again within a minute. A browser test with the stand-in answering `/api/events` and `/api/status` with a redirect.
- [ ] While a field is being typed into, the reload waits until it ends. The same test.
- [ ] A closed stream is opened again on `online` and on the page being shown again. A browser test firing each event.
- [ ] A stream refused three times (the stand-in answering `/api/events` 502) is connected with no click, the line saying it is reconnecting meanwhile. A browser test. Plant: the backoff removed fails it.
- [ ] After Update, the page comes back on the new build without Retry: the update spec, with the stream refused while the server restarts.
- [ ] `remote::application` sets `auto_redirect_to_identity`; the existing test is changed to assert it. A start with Open from anywhere on updates an application Bagholder made without it, once, with the whole application as its body, and never one the person made, nor one that has it already. A test with the stand-in Cloudflare asserting the body sent.
- [ ] SPEC's Open from anywhere section says what the page does when its sign-in expires and when its connection drops.
