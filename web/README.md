# The page

The Svelte page the Rust server serves at `/`. Svelte 5, TypeScript, built by Vite. `docs/architecture.md` is its design; `docs/parity.md` is what it still owes `SPEC.md`.

Develop against a running server (hot reload, `/api` proxied to it):

    BAGHOLDER_PORT=8765 npm run dev

Check, test, build:

    npm run check
    npm test
    npm run build

`dist/` is built here, in CI, at release and in the Docker image; it is never committed. The server reads `web/dist` from beside the repository root, so after `npm run build` a `cargo run` serves the new page.
