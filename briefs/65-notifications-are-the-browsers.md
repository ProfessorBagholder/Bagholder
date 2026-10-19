# Brief 65: Bagholder's notifications are the browser's (Notifications API, Service Worker, Web Push). The native channels are removed. Replaces brief 64, changes 1 and 2

## Verdict

Bagholder is a web application. A web application tells the person things through the browser: Gmail, Google Calendar, Slack and Discord on the web all do it this way.
- The page asks for permission once, from a click.
- A service worker shows each notification under the site's name and icon.
- The server delivers events while no tab is open through the browser's own push channel (Web Push).

There is no helper app, nothing in the Dock, and nothing installed on the computer. Brief 64's changes 1 and 2 patched a native helper that should never have been built. That was my error: I fixed the helper instead of asking whether there should be one. The owner: "there IS no app. this is literally a web application… show me where any top human engineer would be designing notifications for a webapp this way."

Brief 64's change 3 (`BAGHOLDER_EXIT_WITH`, the source scan, "Do not: start the Python app") and its `decisions.md` entry stand.

## The standard, checked 2026-10-19

- **Push API (W3C):** "the ability to receive messages pushed to them from a server, whether or not the web app is in the foreground, or even currently loaded". The service worker "will be started as necessary to handle incoming push messages… for example, by displaying a notification (using `ServiceWorkerRegistration.showNotification()`)". A subscription's endpoint "is a unique capability URL" ([MDN, Push API](https://developer.mozilla.org/en-US/docs/Web/API/Push_API)).
- **Safari:** "Web Push is supported in Safari 16 on macOS Ventura", on the W3C Push API, Notifications API and Service Workers ([WebKit, Meet Web Push](https://webkit.org/blog/12945/meet-web-push/)). Safari asks for permission only from a click or a keystroke, and allows no push that shows nothing (Apple, WWDC22 "Meet Web Push for Safari").
- **Loopback is a secure context:** an origin on 127.0.0.0/8 or ::1/128 is "potentially trustworthy" ([W3C Secure Contexts, §3.1](https://www.w3.org/TR/secure-contexts/)). So the page at `http://127.0.0.1:<port>` may register a service worker and subscribe to push.
- **Message encryption and sender identity:** RFC 8030 (the push protocol), RFC 8291 (payload encryption, `aes128gcm`: the push service cannot read the message) and RFC 8292 (VAPID: the server's own key pair, with no account at any vendor).

Delivery goes through the push service of the person's own browser: Google's for Chrome, Mozilla's for Firefox, Apple's for Safari. That is the browser's channel, not a service Bagholder signs up for. The message is encrypted end to end (RFC 8291). With a tab open, nothing goes through it at all.

## The design

1. **Remove every native channel:**
   - the Mac applet: `mac_build`, `mac_open_command`, `osacompile` and `plutil`;
   - the Windows toast and its registry app id;
   - `notify-send`;
   - `native_channel` and `channel_for`, and the native modes of `BAGHOLDER_NOTIFY`.

   At its next start, the server deletes what it made before: `Bagholder.app` in the data folder, and the Windows app id under the person's own registry hive. It logs each removal once.
2. **A web app manifest:** the name Bagholder, its icons, `start_url` and `display: standalone`. A person who installs the page from the browser's own install button gets a window and notifications under Bagholder's name and icon. This is the web's mechanism for that, and SPEC's "under its own name and icon" is met this way.
3. **A service worker** (`/sw.js`, built with the page, scope `/`):
   - `push`: always shows the notification (`showNotification`, with the tag set to the row's id), never a silent push;
   - `notificationclick`: focuses an open Bagholder tab, or opens one, at the row's own target (SPEC notifications-7 and notifications-12);
   - `pushsubscriptionchange`: subscribes again and tells the server.
4. **Permission and subscription, from a click only:** a "Turn on notifications" control where the notification kinds are set. It calls `Notification.requestPermission()` and `pushManager.subscribe({ userVisibleOnly: true, applicationServerKey })` in the click's handler. It shows the state (on, off, or blocked in the browser's settings, with how to change it) and never asks on its own.
5. **The server sends Web Push itself:**
   - a VAPID key pair (P-256) made once per data folder and kept in the book's settings;
   - subscriptions stored in the book. This is a migration, so a new one, never an edited one;
   - each row encrypted per RFC 8291 and sent per RFC 8030, with a VAPID token (RFC 8292);
   - a subscription the push service answers 404 or 410 is deleted (RFC 8030 §7.3);
   - a failure to deliver is said in the header by the rule of 2026-10-09: said once confirmed, and standing until a send succeeds.

   **Dependencies:** weigh a maintained Web Push crate against RustCrypto's `p256`, `hkdf` and `aes-gcm`, which the workspace may already carry. Put the sizes in the PR.
6. **One row, one banner:**
   - a row sent by push is marked sent;
   - a page that is open shows the row through the same service worker, so the tag collapses it into one banner, and nothing goes through the push service;
   - the bell's history is unchanged (notifications-9 to notifications-13).
7. **No other copy can notify the person, by construction.** Only a browser that subscribed to a copy, and only that copy's data folder, holds a subscription. A scratch copy has none, so `BAGHOLDER_NOTIFY` goes entirely.
8. **SPEC and the wire:**
   - SPEC notifications-1 to notifications-6 and notifications-8 are rewritten to this, citing the standards above. `architecture.md` §11's notifications line says "through the browser";
   - the status's `notify.native` field goes. That is a wire change, and this brief is its gate. `wire.ts` is regenerated.

## Tests

- **Encryption:** RFC 8291's own worked example (Appendix A) is reproduced byte for byte, and the VAPID token is checked against RFC 8292's form.
- **Delivery:** a stand-in push service on loopback receives a generated row, decrypts it with the test subscription's keys, and gets the row back. A 410 deletes the subscription. A refused send is said once confirmed, and cleared by the next success.
- **Service worker:** its `push` and `notificationclick` handlers are tested in Vitest, with a stand-in `self`. Every push shows a notification, and a click goes to each kind's target.
- **Browser (e2e):** with permission granted, the control subscribes and the server stores the subscription. With an open page, a new row shows one notification with its tag and sends nothing to the push service.
- **Removal:** a source scan finds no `osacompile`, `open -n`, `notify-send` or `powershell` in the server. A start over a data folder holding an old `Bagholder.app` removes it.
- **Verified once in real browsers,** by a session on the owner's Mac, not by the owner:
  - Chrome and Safari, with no Bagholder tab open: a real push arrives, under the site's name, or under Bagholder's when installed;
  - a click opens the right page;
  - nothing appears in the Dock (`log show`, as brief 64 described).

  If Safari refuses push on `http://127.0.0.1`, say so in the PR. Notifications then still work with a tab open in Safari, and fully in Chrome and Firefox, and SPEC says which.

## Out of scope

A copy reached over the network (`BAGHOLDER_BIND`) isn't a secure context over plain HTTP, so it gets no service worker. That comes with remote access, when the owner takes it up.

## Process

This is a feature redesign and a wire change. Write it as a plan from this brief (`PLAN.template.md`, sources as above), and I'll gate it at once. Build it in one PR together with brief 64's change 3, and nothing else.
