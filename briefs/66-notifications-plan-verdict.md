# Brief 66: Verdict on plan #555 (notifications through the browser): Go with two changes

## Verdict

**Go.** The plan builds brief 65 and brief 64's change 3:
- openssl, already linked, for P-256, HKDF, AES-128-GCM and ES256, with RFC 8291's Appendix A as the correctness test, so no new crate;
- the service worker as a second Vite entry, its handlers exported for Vitest;
- an open page shows a row through the service worker, and push is used only with no page open;
- subscriptions kept by a new migration, and VAPID in the book's settings;
- the old helper and registry id deleted at start;
- `notify.push` in place of `notify.native`, a wire change gated here.

## Required changes

1. **`sub` is the project's public address, not a mailto to no one.** RFC 8292 gives `sub` for the push service's operator to reach the sender. Apple's push service rejects a token whose "subject claim isn't a URL or `mailto:`" (`BadJwtToken`, Apple, "Sending web push notifications in web apps and browsers"). A mailto to no one passes that check and tells the operator nothing.
   - Use the repository's own URL, `https://github.com/ProfessorBagholder/Bagholder`, whose issues are where a sender running this software is reached.
   - It names the software, never the person, and the owner's email never goes in any request.
2. **Meet each push service's stated limits, and test each against the stand-in.** From Apple's page, which matches RFC 8030 and 8292:
   - `TTL` on every request;
   - `Urgency: high` for a fill or an order problem, and `normal` otherwise;
   - a payload of at most 4 KB, so a row that doesn't fit carries its id, and the service worker reads the row from the server before it shows it;
   - the VAPID token's `exp` at most 24 hours ahead, and the token kept per push-service origin and renewed no more often than hourly;
   - `aud` set to the endpoint's origin;
   - a 404 or 410 deletes the subscription, and a 429 or 5xx is retried once a push service says so (`Retry-After`) and said in the header once confirmed.

   *Test:* the loopback stand-in checks every header and the token's claims on generated rows, including one over 4 KB.

The one item left open is Safari on `http://127.0.0.1`, checked by the Mac session, as the plan says. Build it in one PR.
