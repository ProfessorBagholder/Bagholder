# Plan: in Docker, the update notice is a button that updates Bagholder

## For the owner to decide

Nothing open: the owner chose the button (`docs/decisions.md`, 2026-11-02).

## Scope

**In it:**
- A Bagholder running in Docker with an updater set up shows `Update to vX.Y.Z` where it now shows `vX.Y.Z image available` (a link to GitHub). Pressing it updates the container and the page comes back on the new version.
- Without an updater, the notice no longer links to GitHub: it says the one command that updates the container (`docker compose pull && docker compose up -d`), with a copy button.
- The shipped `docker-compose.yml` and the README's Docker section carry the updater as an optional service.

**Out of it:** the app's own self-update outside Docker, which is unchanged.

## Why the updater can't be part of Bagholder

The owner's rule is that nothing is built outside Bagholder that could be a feature of it, so this has to be shown.
- **Replacing a container takes Docker's own controls.** The Docker Engine API (its socket) can pull an image and recreate a container. Whoever holds that socket holds the host, because a container with it can start a privileged container: Docker's own documentation says giving access to the socket is equivalent to root on the host (docs.docker.com, "Docker daemon attack surface"). Giving it to Bagholder would undo 2.16.3's hardening: no capabilities, a user of its own, no new privileges.
- **A container can't recreate itself.** Stopping the old container ends the process doing the work, so the replacement has to be done by something outside it.
- So the work belongs to a small, separate updater that holds the socket and does nothing else. Bagholder only asks it, over its token-protected HTTP API, to update Bagholder's own container.

## How it's done in the category

Read 2026-11-02.
- **Watchtower** is the common updater for self-hosted Docker apps, but the original project (`containrrr/watchtower`) is archived on GitHub, with its last release v1.7.1 in 2023-11.
- **Its maintained fork, `nicholas-fedor/watchtower`** (image `nickfedor/watchtower`, release v1.23.2 on 2026-10-22, about 4,800 stars), documents an HTTP API: `POST /v1/update` with an API token, taking `container` and `image` filters to update one container, and `async` (its `docs/http-api/overview`). Only containers labelled `com.centurylinklabs.watchtower.enable=true` are touched when `--label-enable` is set.
- Diun (`crazy-max/diun`) only notifies; it doesn't update.

## Approach

- **Settings:** `BAGHOLDER_UPDATER_URL`, the updater's address on the compose network (e.g. `http://bagholder-updater:8080`), and `BAGHOLDER_UPDATER_TOKEN_FILE`, a file holding the token, read at start. A bad value refuses the start and names it, as `BAGHOLDER_HOSTS` does.
- **The button:** `POST /api/update/container`, the page's write, from the page only (the gate's write rule). The server asks the updater `POST /v1/update?container=<this container's name>` with the token, and says what came back. A failure is said in plain words.
  - **The container's name:** `BAGHOLDER_CONTAINER_NAME`, required whenever `BAGHOLDER_UPDATER_URL` is set and refused at start by name if missing. It's never inferred: Docker's default hostname is the container's id, not its name, and the updater would match nothing.
- **During the update:** the page says `Updating to vX.Y.Z…`. The stream drops as the container stops, the page reconnects when it answers again, and the new version's status clears the notice.
- **The shipped `docker-compose.yml` gains:**
  - an optional `bagholder-updater` service, `nickfedor/watchtower` **pinned by digest**, since it's the one container on the host that holds the socket. The README says how to move the pin. It runs on the same network with `--http-api-update` and **without** `--http-api-periodic-polls`, so it never updates on a schedule and the button is the only trigger (said in the compose comment too). It also takes `--label-enable`, `--cleanup`, the token from a file (never an environment variable), and the Docker socket;
  - Bagholder with `container_name: bagholder`, labelled `com.centurylinklabs.watchtower.enable=true`, and given the three settings.

  The updater publishes no port.

## Acceptance criteria

- [ ] With the two settings, the status says the updater is there, and the notice reads `Update to vX.Y.Z`. A page unit test.
- [ ] Pressing it makes exactly one `POST /v1/update?container=<name>` with the token to the updater. A server test against a stand-in updater.
- [ ] An updater that refuses or can't be reached is said in words, and nothing else changes. Server tests.
- [ ] Without an updater, the notice says the compose command with a copy button and links nowhere. A page unit test.
- [ ] The write is refused from anywhere but the page (the gate's write test lists it).
- [ ] The shipped compose starts with the updater (the container job), and the updater sees only labelled containers. Its startup log says it runs in HTTP API update mode with no periodic polls; the container job asserts the line the fork logs, or, if it logs none, this is held by review and said here.
- [ ] `BAGHOLDER_UPDATER_URL` set without `BAGHOLDER_CONTAINER_NAME` refuses the start, naming it. A server test.
- [ ] An update pressed in the page, on a copy running the shipped compose, comes back on the new version: the container job, from one pinned image to the next.
