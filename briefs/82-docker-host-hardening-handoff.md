# Brief 82: Hardening Bagholder's container on the home server (a handoff)

A handoff for the session that edits the home server's compose files. Written 2026-10-31 from the owner's plan, corrected against Bagholder as it is at 2.16.1 (`rust/Dockerfile`, `docker-compose.yml`, `http/admit.rs`, `market/src/localmodel.rs`). Nothing here changes Bagholder's code; what needs a code change is listed at the end as the Architect's to brief.

## Rules for the session

- Edit compose and env files only. Touch no port 22, 80 or 443, no media-server compose file, nothing of Bagholder's code or image.
- Show each diff before applying it. Apply one step at a time. After each step, run the check in §8; if anything fails, roll that step back before going on.
- Run nothing that restarts another container. `docker compose up -d bagholder` recreates Bagholder alone; Traefik is recreated only in step 2, once, and the owner says when.
- Print no hostname, address, token or key in any report. Name files, not their contents.
- Never ask the owner to run, approve or delete anything; a refused step is reported with its exact text, and the session stops.
- Scratch, if any, goes in `$TMPDIR/bh-*` and is removed by the same command that made it.

## What is true about Bagholder, and decides the steps

- **The image runs as root on purpose** (`rust/Dockerfile`: "root inside the container, so a data folder mounted from any host user is writable as it is"). The process touches nothing but `/data`. A non-root user is a change to the image, not to compose (§9).
- **It writes under `/data` only:** the book, the Wealthsimple session, the installed cloudflared and ngrok (`agents-cloudflare/`, `agents-ngrok/`), tunnel credentials, the page-from-anywhere files, and the summarizer's llamafile. Chromium (for the sign-in, on Xvfb) writes a profile under `/tmp`.
- **It has no login of its own.** The page's gate (`http/admit.rs`, 2.16.0) answers the local network (private ranges, 100.64/10 for Tailscale) and refuses the internet, however reached. An internet visit through Traefik with the local name gets 403 from Bagholder: that is the gate working, not a hole. `BAGHOLDER_HOSTS` means "a proxy with its own login in front, let its visitors in from anywhere": never set it for the local route.
- **Its own doors from outside:** the agents' tunnel (sign-in codes and keys) and the page's tunnel (Cloudflare Access), both run inside the container, neither through Traefik.
- **The local summarizer** is Gemma 2 2B, a 1.95 GB llamafile downloaded from huggingface.co on first use and run inside the container, on the CPU. It takes about 2 GB of memory loaded, plus working memory.
- **Outbound is open-ended by design:** Wealthsimple, TMX, Yahoo, Nasdaq, the Bank of Canada, Coinbase, SEDAR+, CIRO, each fund company's own site (Vanguard, BlackRock, Ninepoint, …), news sites, GitHub, Cloudflare's API, Cloudflare DNS over HTTPS, the push services, huggingface.co, bin.equinox.io. An outbound allow-list by host is not practical; the rule is "outbound to the internet, nothing to the LAN or host".

## 1. Secrets first

1. The Traefik compose file's Cloudflare DNS API token moves to a `.env` beside it, mode 600, owned by the compose owner, referenced as `${CF_DNS_API_TOKEN}`. Check `git status` there first: if that folder is a repository, add `.env` to its ignore file in the same change.
2. Traefik's dynamic config (the dashboard's password hash) and the system cloudflared tunnel's token (on its service command line or in its config): each file mode 600, owner-only. Report where each lives, by path.
3. Bagholder's `./data` is root-only already (the finding); leave it so. Its secrets live there (Cloudflare sign-in, ngrok token, tunnel credentials, Wealthsimple session, Access files); Bagholder itself keeps them 0600 in 0700 folders.
4. Then tell the owner: rotate the Cloudflare DNS token (it was readable by every account on the server). Say which Cloudflare page, not the value.

## 2. A network of Bagholder's own

Bagholder today sits on the shared `traefik` network with eleven containers, three of which handle untrusted content (Transmission, Flaresolverr, ArchiveBox); each can reach port 8765 directly.

1. Create `bagholder`, a bridge network, **not** `internal` (Bagholder needs outbound).
2. Bagholder joins `bagholder` only. Traefik joins `bagholder` beside `traefik`. Label Bagholder `traefik.docker.network=bagholder`.
3. Recreate Bagholder, then Traefik (the one Traefik restart, with the owner's go).
4. Verify from inside a media container (`docker exec … sh -c 'nc -zw2 bagholder 8765 || echo refused'` or the curl it has) that 8765 is unreachable; from Bagholder's container that Transmission's control port is unreachable; from Traefik that 8765 is reachable.

Nothing of Bagholder's needs any neighbour but Traefik. Authelia in front of Bagholder would be the `BAGHOLDER_HOSTS` path and still needs only Traefik.

## 3. Traefik allow-list on the local route

Bagholder's README already recommends it; the two layers then agree.

- Middleware `ipallowlist` with `sourcerange` = the home range(s) and `100.64.0.0/10` (Tailscale), attached to Bagholder's router. Ask the owner for the home range; never guess it.
- Test from an allowed address (200 or the page) and a disallowed one (Traefik's 403 before Bagholder's).
- Do not set `BAGHOLDER_HOSTS`.

## 4. Hardening Bagholder's compose

In this order, each tested with §8 before the next:

1. `security_opt: [no-new-privileges:true]`: safe.
2. `pids_limit: 512`: Chromium, Xvfb, the llamafile and its threads, a tunnel and its guard all count. Never below 256.
3. `mem_limit`: **6g** when the summarizer runs inside (the model is 1.95 GB loaded, plus working memory, plus Chromium). If the server can't spare it, set `BAGHOLDER_LLM_URL` to an Ollama on the host instead and the container needs about 2g. Set `memswap_limit` equal to `mem_limit` so it doesn't swap past the limit. Test a Wealthsimple sign-in and a summary after.
4. `cap_drop: [ALL]`: expected to work. Bagholder's tunnel guard uses `prctl(PR_SET_PDEATHSIG)`, no capability. Test: the page, a Wealthsimple sign-in (Chromium under Xvfb), a tunnel start and stop, a summary. Keep it only if all pass; else `cap_drop: [ALL]` plus the one capability the failure names, reported.
5. **Not on the live container, and expect it to break:** `read_only: true` with `tmpfs: [/tmp]`, and a non-root `user:`. On a copy with its own `./data` copy only. Bagholder writes `/data` (fine, it's a mount) and Chromium writes `/tmp` (tmpfs covers it), but `/data` is root-owned, so a non-root user can't open the book. Report what breaks; the fix is in the image (§9), not compose.
6. `logging: {driver: json-file, options: {max-size: 10m, max-file: "3"}}` so the log can't fill the disk.
7. A healthcheck: `GET http://127.0.0.1:8765/api/status` from inside the container, interval 60s, so `restart: unless-stopped` has something to act on. Bagholder's own gate admits its own loopback.

## 5. Firewall (the server's DOCKER-USER chain), with the owner's rules in hand

Only after the owner pastes `ufw status verbose`, `iptables -S DOCKER-USER` and `iptables -S FORWARD`. Propose, never apply without their yes:

- From the `bagholder` network: drop to the host's LAN ranges and to the host's own addresses, except established/related replies; allow everything else out (the internet). Never block outbound 443/80 or DNS, or every data source and both tunnels die.
- Nothing inbound to the `bagholder` network from the LAN except Traefik (it's on the network itself, not forwarded).
- Keep the media stack's existing rules untouched.

The DMZ the owner wants is router hardware for another day (the RT-AC3100 can't do tagged VLANs in a supported way); when it exists, these rules move to the router and this step is removed.

## 6. Pin the image

Replace `:latest` with the release tag (`:v2.16.1` today). Each release's notes name the tag; the owner updates by changing the tag and `docker compose pull && up -d bagholder`. Trade stated: fixes no longer arrive by pull alone.

## 7. More low-hanging fruit, each non-impacting

- **Traefik's Docker socket:** mount it read-only (`/var/run/docker.sock:/var/run/docker.sock:ro`) and set `--providers.docker.exposedByDefault=false`, then `traefik.enable=true` only on the containers meant to be routed. Today any container on the network can be exposed by a label. A socket proxy (tecnativa/docker-socket-proxy, `CONTAINERS=1` only) is the next step up; propose it, don't apply it in this pass.
- **Traefik's dashboard:** confirm it's behind the password and not on a public entrypoint; if it's on the internet, put the same `ipallowlist` on it.
- **Transmission's control port with no password:** out of scope to fix (media compose), but report it plainly: anything on the `traefik` network can drive it. Suggest moving it to its own network the same way, as a follow-up.
- **Image provenance:** `docker compose pull` with the pinned tag, and once a release, `docker image inspect` the digest against the one in the release notes. Ask the CTO to print the image digest in each release's notes (§9).
- **`./data` backups:** the book and every tunnel credential are in one folder. If it isn't in the server's backup set, say so; if it is, the backup must be as private as the folder (it holds the Wealthsimple session).
- **Docker daemon defaults** (`/etc/docker/daemon.json`): `"no-new-privileges": true` and `"live-restore": true` as defaults for every container, and `"log-driver"` limits daemon-wide. Propose; apply only with the owner's yes, since it touches every container on restart.
- **Unattended security updates on the host** (`unattended-upgrades` for the OS, not Docker images): check it's on; report.
- **Time:** the host's clock in sync (`timedatectl`), since Access passes and the agents' codes expire by time.

## 8. The check after every step

1. `docker compose ps bagholder` healthy (after step 4.7, the healthcheck says so).
2. The local route: the page opens from a device on the home network at its local name, figures shown.
3. The public MCP route: from outside the network (a phone off Wi‑Fi), the agents' address answers its `/.well-known/oauth-protected-resource` with 401 and the metadata header, and a connected agent still reads. Bagholder's page at the "Open from anywhere" address opens after the Cloudflare sign-in.
4. A Wealthsimple sync completes (Sync now), and a summary renders where one did before.
5. Every media container still up and reachable as before (`docker compose ps` in their folder; Emby's page, Transmission's UI).
6. No new file under `/tmp` on the host from this session; the `bh-*` scratch gone.

## 9. Not compose: what needs a change to Bagholder (the Architect's to brief)

- A non-root user in the image, with `/data` chowned at start by an entrypoint that drops privileges (so any host user's `./data` still works): the owner's `read_only` and `user:` tests then become possible.
- The image digest and the tag printed in each release's notes.
- A documented `GET /api/status` healthcheck line in the shipped `docker-compose.yml`, and the `logging` limits, so every install gets them.

These three go to the CTO as one small PR after this handoff runs.
