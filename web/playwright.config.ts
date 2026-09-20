import { defineConfig } from '@playwright/test'

// The browser tests (docs/architecture.md, rule 8): the real server, on a made-up
// book, driven in a real browser. `e2e/serve.mjs` makes the book and starts the
// server; nothing leaves the machine (BAGHOLDER_OFFLINE) and no order can be
// placed (BAGHOLDER_DRY_ORDERS).
export const PORT = 8791

export default defineConfig({
  testDir: 'e2e',
  fullyParallel: false, // one server, one book: tests that write would cross
  workers: 1,
  reporter: [['list']],
  timeout: 30_000,
  use: {
    baseURL: `http://127.0.0.1:${PORT}`,
    viewport: { width: 1440, height: 900 },
    colorScheme: 'dark',
  },
  webServer: {
    command: 'node e2e/serve.mjs',
    url: `http://127.0.0.1:${PORT}/api/status`,
    reuseExistingServer: false,
    timeout: 600_000, // a cold `cargo build` on a fresh machine
    stdout: 'ignore',
    stderr: 'ignore',
  },
})
