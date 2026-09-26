import { defineConfig } from '@playwright/test'

// The browser tests (docs/architecture.md, rule 8): the real server, on a made-up
// book, driven in a real browser. `e2e/serve.mjs` makes the book and starts the
// server; nothing leaves the machine (BAGHOLDER_OFFLINE) and no order can be
// placed (BAGHOLDER_DRY_ORDERS).
// (E2E_PORT: a second run beside the first, each with its own server and book)
export const PORT = Number(process.env.E2E_PORT) || 8791

export default defineConfig({
  testDir: 'e2e',
  fullyParallel: false, // one server, one book: tests that write would cross
  workers: 1,
  reporter: [['list']],
  outputDir: process.env.E2E_PORT ? `test-results-${process.env.E2E_PORT}` : 'test-results',
  timeout: 30_000,
  use: {
    baseURL: `http://127.0.0.1:${PORT}`,
    viewport: { width: 1440, height: 900 },
    colorScheme: 'dark',
    // the viewer's zone, pinned: a machine's own zone never changes an expected time
    timezoneId: 'America/Toronto',
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
