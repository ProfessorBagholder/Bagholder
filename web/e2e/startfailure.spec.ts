import { execFileSync, spawn, type ChildProcess } from 'node:child_process'
import { mkdirSync, mkdtempSync, renameSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { expect, test } from '@playwright/test'
import { PORT } from '../playwright.config'

// #516, brief 50: a store this build cannot open never takes the header with it.
// The made-up book's market cache made unreadable (a folder where its file was),
// on a server of this test's own: the page still shows the version beside the
// update on offer, and its error line says the cache failed.

const rust = join(dirname(fileURLToPath(import.meta.url)), '..', '..', 'rust')
const exe = process.platform === 'win32' ? '.exe' : ''
// beside the two servers Playwright starts (`PORT`, `PORT + 1`)
const port = PORT + 5

let home = ''
let server: ChildProcess | null = null

test.beforeAll(async () => {
  home = mkdtempSync(join(tmpdir(), `bagholder-e2e-${port}-${process.pid}-`))
  execFileSync(join(rust, 'target', 'debug', 'demo-book' + exe), ['--home', home], { stdio: 'ignore' })
  execFileSync(join(rust, 'target', 'debug', 'bagholder' + exe), ['demo-facts', home], { stdio: 'ignore' })
  renameSync(join(home, 'market.db'), join(home, 'market.db.kept'))
  mkdirSync(join(home, 'market.db'))
  server = spawn(join(rust, 'target', 'debug', 'bagholder' + exe), [], {
    stdio: 'ignore',
    env: { ...process.env, BAGHOLDER_HOME: home, BAGHOLDER_PORT: String(port), BAGHOLDER_CHILD: '1', BAGHOLDER_NO_BROWSER: '1', BAGHOLDER_DRY_ORDERS: '1', BAGHOLDER_OFFLINE: '1' },
  })
  // until it serves the page: it listens once its start is done
  await expect.poll(async () => fetch(`http://127.0.0.1:${port}/`).then((r) => r.status, () => 0), { timeout: 30_000 }).toBe(200)
})

test.afterAll(async () => {
  if (server && server.exitCode == null) {
    const gone = new Promise((r) => server!.once('exit', r))
    server.kill('SIGTERM')
    await gone
  }
  rmSync(home, { recursive: true, force: true })
})

test('a market cache that cannot be opened leaves the header whole: the version shows, and the error line says why', async ({ page }) => {
  await page.goto(`http://127.0.0.1:${port}/#dashboard`)
  await expect(page.locator('header, #page').first()).toBeVisible()
  await expect(page.getByText(/^v\d+\.\d+\.\d+/)).toBeVisible()
  await expect(page.locator('.status-err')).toContainText('The market cache could not be opened')
})
