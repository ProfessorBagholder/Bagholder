import { defineConfig } from '@playwright/test'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

// The browser tests (docs/architecture.md, rule 8): the real server, on a made-up
// book, driven in a real browser. `e2e/serve.mjs` makes the book and starts the
// server; nothing leaves the machine (BAGHOLDER_OFFLINE) and no order can be
// placed (BAGHOLDER_DRY_ORDERS).
// (E2E_PORT: a second run beside the first, each with its own server and book)
export const PORT = Number(process.env.E2E_PORT) || 8791
// where the signed-in server names every request it makes, for the test that nothing
// it sends goes to a host but the stand-in
export const REQUEST_LOG = join(tmpdir(), `bagholder-e2e-requests-${PORT + 1}.log`)
process.env.E2E_REQUEST_LOG = REQUEST_LOG
// (E2E_URL: a server already running, the image in CI; only the order test, which
// stands nothing in, is run against it, and nothing is started here)
const URL = process.env.E2E_URL

export default defineConfig({
  testDir: 'e2e',
  fullyParallel: false, // one server, one book: tests that write would cross
  workers: 1,
  reporter: [['list']],
  outputDir: process.env.E2E_PORT ? `test-results-${process.env.E2E_PORT}` : 'test-results',
  timeout: 30_000,
  use: {
    baseURL: URL ?? `http://127.0.0.1:${PORT}`,
    viewport: { width: 1440, height: 900 },
    colorScheme: 'dark',
    // the viewer's zone, pinned: a machine's own zone never changes an expected time
    timezoneId: 'America/Toronto',
  },
  // the signed-in tests run against a second server, the same made-up book with a
  // saved Wealthsimple sign-in and Wealthsimple's search stood in (`serve.mjs --signed-in`)
  projects: URL
    ? [{ name: 'image', testMatch: 'dryorder.spec.ts' }]
    : [
        { name: 'app', testIgnore: 'signedin*.spec.ts' },
        { name: 'signed-in', testMatch: 'signedin*.spec.ts', use: { baseURL: `http://127.0.0.1:${PORT + 1}` } },
      ],
  webServer: URL ? undefined : [
    {
      command: 'node e2e/serve.mjs',
      url: `http://127.0.0.1:${PORT}/api/status`,
      reuseExistingServer: false,
      timeout: 600_000, // a cold `cargo build` on a fresh machine
      stdout: 'ignore',
      stderr: 'ignore',
      // stopped as a person stops it, so it removes its home (a saved sign-in in the
      // signed-in one) once its server has exited, never left for the next run
      gracefulShutdown: { signal: 'SIGTERM', timeout: 10_000 },
    },
    {
      command: `node e2e/serve.mjs --signed-in --port=${PORT + 1} --requests=${REQUEST_LOG}`,
      url: `http://127.0.0.1:${PORT + 1}/api/status`,
      reuseExistingServer: false,
      timeout: 600_000,
      stdout: 'ignore',
      stderr: 'ignore',
      // stopped as a person stops it, so it removes its home (a saved sign-in in the
      // signed-in one) once its server has exited, never left for the next run
      gracefulShutdown: { signal: 'SIGTERM', timeout: 10_000 },
    },
  ],
})
