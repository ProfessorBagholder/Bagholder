# Plan: Bagholder's notifications are the browser's, and no copy outlives the work that started it

## For the owner to decide

Nothing open. The owner decided both (2026-10-19, `docs/decisions.md`). Bagholder is a web application, so its notifications come through the browser. And no copy of the app is left running after the work that started it. Brief 65 sets the design, and brief 64's change 3 the second part.

## Scope

Today the server posts notifications itself through the computer's own tools: an AppleScript applet on a Mac, a PowerShell toast on Windows, `notify-send` on Linux. The page shows them only where the server has no desktop. This plan removes every native channel, and shows every notification through the browser. With a tab open, the page shows it through its service worker. With no tab open, the server sends it by Web Push to the browsers that subscribed, each of which shows it through the same service worker.

It also makes every copy the repository starts for some work end with that work (`BAGHOLDER_EXIT_WITH`, brief 64, change 3).

Touched:
- SPEC notifications-1 to notifications-6 and notifications-8;
- `docs/architecture.md` §11;
- the status payload's `notify` (a wire change, gated by brief 65);
- `CLAUDE.md`'s scratch command and its *Do not*;
- the book (a new migration);
- the page's notification switches.

Out of scope:
- a copy reached over the network (`BAGHOLDER_BIND`), which is not a secure context over plain HTTP and so gets no service worker (brief 65, out of scope);
- the bell's history (notifications-9 to notifications-13) and the kinds (notifications-15 onward), which are unchanged.

## The old app here

The old app posted notifications through the operating system, and the Rust app carried that over: `notify.rs`, `mac_build`, `windows_script`, `linux_command`. It was carried because it existed, which `CLAUDE.md` says is never a reason.

What it costs:
- a helper app built on the person's computer, which flashed a Dock tile on every notification;
- a registry entry on Windows;
- a per-OS code path for each system;
- any copy started on the same computer posting to the person's desktop: two leftover copies posted every release twice (brief 64).

None of it is carried over. The new entry in `docs/old-app-mistakes.md`: *notifications posted by the server through the operating system, for a web application*.

## How the leading products do it

Each was read on 2026-10-19 from the source linked.
- **Web applications:** Gmail, Google Calendar, Slack and Discord on the web all ask for the browser's permission from a click, show notifications through the browser, and deliver with no tab open through Web Push. None installs a helper on the computer.
- **The Push API** (W3C, as [MDN, Push API](https://developer.mozilla.org/en-US/docs/Web/API/Push_API) describes it): a service worker "will be started as necessary to handle incoming push messages… for example, by displaying a notification", and a subscription's endpoint "is a unique capability URL".
- **Safari:** "Web Push is supported in Safari 16 on macOS Ventura" ([WebKit, Meet Web Push](https://webkit.org/blog/12945/meet-web-push/)). It asks for permission only from a user gesture, and allows no push that shows nothing (WWDC22, "Meet Web Push for Safari").
- **Loopback is a secure context:** 127.0.0.0/8 and ::1/128 are "potentially trustworthy" ([W3C Secure Contexts §3.1](https://www.w3.org/TR/secure-contexts/)). So `http://127.0.0.1:<port>` may register a service worker and subscribe.
- **The protocol:**
  - RFC 8030, delivery: an endpoint answering 404 or 410 is gone (§7.3);
  - RFC 8291, payload encryption: `aes128gcm`, unreadable by the push service;
  - RFC 8292, VAPID: the server's own P-256 key, with no account anywhere.

## Open questions

1. **Which crypto for RFC 8291 and RFC 8292.**
   - *Objective:* exact encryption and signing, at the least cost to the archive.
   - *Known:* the server already links `openssl` (`notify.rs` hashes with `openssl::sha`, and the market client links the system OpenSSL). It provides everything the two RFCs need: P-256 key generation and ECDH (`EcKey`, `Deriver`), HKDF from HMAC-SHA-256 (RFC 5869 is two HMACs), AES-128-GCM (`symm::encrypt_aead`), and ES256 signatures (`EcdsaSig`). RustCrypto's `p256`, `hkdf` and `aes-gcm` are not in `Cargo.lock`. The `web-push` crate brings its own HTTP client.
   - *Settled by:* building on `openssl`, which adds no crate, after measuring the archive with the alternatives. The PR gives the crate counts and sizes of `web-push`, and of `p256` + `hkdf` + `aes-gcm`, beside zero for `openssl`. RFC 8291's worked example (Appendix A), reproduced byte for byte, settles correctness whichever is used.

2. **Whether Safari delivers push to `http://127.0.0.1`.**
   - *Objective:* SPEC says exactly where notifications arrive with no tab open.
   - *Known:* Secure Contexts makes loopback trustworthy, but WebKit's documentation of push speaks of HTTPS sites.
   - *Settled by:* the real-browser check by a session on the owner's Mac. If Safari refuses, SPEC says that in Safari notifications arrive with a tab open, and fully in Chrome and Firefox.

## Approach

### Removed

- `notify.rs`: the Mac applet (`mac_app`, `mac_build`, `mac_stamp`, `mac_icon`, `mac_open_command`, `mac_plain_command`, `mac_script`), the Windows toast (`windows_script`, `windows_command`, the registry app id), `linux_command`, `deliver`'s native arms, `native_channel`, `channel_for`, `MODE_ENV` and the `notifications-system` header entry.
- `BAGHOLDER_NOTIFY` everywhere it is set: `tests_common.rs`, `serve.mjs` and `CLAUDE.md`.
- **At start**, the server deletes what earlier versions made: `Bagholder.app` in the data folder, and, on Windows, the app id's key under `HKEY_CURRENT_USER\Software\Classes\AppUserModelId` (through `windows-sys`'s registry calls, no shell). It logs each removal once; one that is already gone is no line.

### The service worker (`web/src/sw.ts`, served as `/sw.js`, scope `/`)

- It is built with the page as a second entry of `vite build`, its output named `sw.js` at the root of `web/dist`, with no hash, so the browser finds it at a fixed URL. The server serves it from the built page as any asset, with `Cache-Control: no-cache` so an update is seen at once.
- **`push`**: it parses the row (`{ id, title, body, kind, target }`) and always calls `showNotification(title, { body, tag: 'bh-' + id, data })`. A push it cannot read still shows a generic "Bagholder" notification, so no push is ever silent.
- **`notificationclick`**: it closes the notification, then focuses an open Bagholder tab and sends it the row to open its target (`openNoteTarget`, unchanged: SPEC notifications-7 and notifications-12). With no tab open, it opens one at the row's target URL.
- **`pushsubscriptionchange`**: it subscribes again with the same key and posts the new subscription.
- Its handlers are plain functions over a stand-in `self`, exported for Vitest; the file's last lines bind them to the worker's events.

### The page

- `channel.svelte.ts` registers `/sw.js` once, at start.
- **"Turn on notifications"**, beside the kinds in the menu, is the one place permission is asked. In its click handler it calls `Notification.requestPermission()`, then `registration.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey })` with the server's public key, then `POST /api/push/subscribe`. It never asks on its own.
- **Its state** is on, off, or blocked in the browser's settings, with how to change them for this site.
- **A row arriving with the page open** is shown through `registration.showNotification` with the same tag, never `new Notification`. A push that also arrives for it collapses into the one banner.

### The server

- **VAPID:** a P-256 key pair made once per data folder, kept in the book's settings. Its public key, in base64url, is sent in the status payload's `notify.push`.
- **Subscriptions:** a new book migration creates `push_subscriptions(endpoint TEXT PRIMARY KEY, p256dh TEXT, auth TEXT, made_at TEXT)`, with its schema snapshot, and is named in the clear-all classification. Routes:
  - `POST /api/push/subscribe`: endpoint and keys, validated (the endpoint an `https` URL, the keys base64url of the right lengths);
  - `POST /api/push/unsubscribe`: the endpoint.
- **Delivery** (`announce`):
  - with any page's stream open, the row goes to the pages only, as today, and the page shows it;
  - with none open, the existing notify worker thread sends it to each subscription. The row's JSON is encrypted per RFC 8291 (`aes128gcm`, record size 4096, one record) and POSTed per RFC 8030, with `TTL`, `Urgency: normal`, `Content-Encoding: aes128gcm` and `Authorization: vapid t=<JWT>, k=<public key>` (RFC 8292). The JWT is ES256 with `aud` the endpoint's origin, `exp` 12 hours on, and `sub` a `mailto:` of no one: the RFC allows any contact URI, and this app has no operator;
  - a row a push reached is marked seen;
  - a 404 or a 410 deletes the subscription;
  - a failed send is said in the header under the existing `notifications` entry, by the 2026-10-09 rule: said once confirmed, and standing until a send succeeds.
- **The status payload's `notify`:** `native` goes, and `push` comes: the public key and the number of subscriptions. `wire.ts` is regenerated, and SPEC notifications-8 says what it carries.
- **No other copy can notify the person, by construction.** A browser holds a subscription only to the copy it subscribed to, on that copy's data folder. A scratch copy has none.

### `BAGHOLDER_EXIT_WITH` (brief 64, change 3, built and held back for this PR)

- `exit_with.rs` watches the named process's exit: a pidfd on Linux, a kqueue `NOTE_EXIT` on macOS, and `OpenProcess` with `WaitForSingleObject` on Windows. The server stops through `request_stop`, and a command exits. A process already gone refuses the start. There is no timer.
- The scratch command in `CLAUDE.md` (`BAGHOLDER_EXIT_WITH=$PPID`), `serve.mjs` (its own pid) and the capacity probes (the runner's pid) set it. A source scan holds every start in `CLAUDE.md`, `web/e2e/*.mjs`, `.github/workflows/*.yml` and `capacity.rs` to it.
- `CLAUDE.md`, *Do not*: start the old Python app.
- `libc` (unix) and `windows-sys` (windows) are listed with the reason, beside each other.

### SPEC and architecture

- notifications-1 to notifications-6 and notifications-8 are rewritten to the above, citing the standards. "Under its own name and icon" goes.
- `docs/architecture.md` §11's notifications line says they come through the browser.

What stays: the kinds and their switches, the store's `notifications` table, the bell and its history, what each card opens, and one row per event.

## Acceptance criteria

- [ ] `cargo test --workspace --locked` green, clippy clean, no warnings. `npm run check`, `npm test` and the e2e suite green in `web/`.
- [ ] **Encryption:** RFC 8291 Appendix A reproduced byte for byte, from its keys, salt and plaintext to its body. The VAPID JWT verifies against the public key and has RFC 8292's header and claims.
- [ ] **Delivery:** a stand-in push service on loopback receives a generated row, decrypts it with the test subscription's private key and auth secret, and gets the row back field for field. A 410 deletes the subscription, and so does a 404. A refused send is said in the header on the second failure and cleared by the next success. With a page's stream open, nothing reaches the stand-in.
- [ ] **The service worker** (Vitest, with a stand-in `self`): every push shows a notification, one it cannot read included. The tag is the row's. A click focuses an open tab and sends it the row, or opens a tab at each kind's target URL. A subscription change subscribes again and posts.
- [ ] **The page** (e2e, Chromium, permission granted by the test's context): the control subscribes, and the server stores the subscription. A new row with the page open shows one notification with its tag, and nothing goes to the push service. The control's three states render.
- [ ] **Removal:** a source scan finds no `osacompile`, `open -n`, `notify-send` or `powershell` in the server's sources. A start over a data folder holding an old `Bagholder.app` removes it and logs it once.
- [ ] **No copy outlives its starter:** a server started with a child's pid stops through its own stop when the child is killed. A pid already gone refuses the start. The source scan passes, and fails on a planted start.
- [ ] **Real browsers,** by a session on the owner's Mac, not the owner, with no Bagholder tab open: in Chrome and in Safari a real push arrives, a click opens the right page, and `log show` shows nothing new in the Dock. Recorded in the PR, or Safari's refusal on `127.0.0.1` stated, with SPEC saying so.
- [ ] **The container:** the image built from the branch, its page subscribing in a browser on the host, and a row shown.
- [ ] **Cost:** archive size before and after, the crate comparison (open question 1), and the paired idle hour (nothing new runs with no subscription).

## Surfaces to check beyond the diff

- `web/src/lib/generated/wire.ts` (`notify`), and `tests_types` blessed.
- The book migration and its schema snapshot; the clear-all classification; the authorizer test of #507, if it reads the new table.
- `TIMED_WAITS`: nothing added.
- The `TERMINAL` list.
- `docs/old-app-mistakes.md` (the new entry).
- `vite.config.ts` (the second entry) and the server's asset route (`/sw.js`).
- `rust/Dockerfile`: unchanged, since the page carries `sw.js`.

## Right to refuse

- If RFC 8291's example cannot be reproduced with `openssl`, stop and report before choosing another crate.
- If Chromium in the e2e cannot subscribe on `127.0.0.1`, which would contradict Secure Contexts, stop and report with its error.

## Anti-stub self-check

*Initialled when built.*

## Verification

*Filled when built.*

## Handoff

*Filled when built.*

**Nothing left running.** *Filled when built.*
