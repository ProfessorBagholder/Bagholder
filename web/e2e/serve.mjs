// Builds the server and the made-up book, then runs the server on it until told
// to stop. Started by Playwright (`webServer` in playwright.config.ts).
import { execFileSync, spawn } from 'node:child_process'
import { mkdtempSync, readdirSync, rmSync, writeFileSync } from 'node:fs'
import { createServer } from 'node:http'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { standIn } from './wealthsimple.mjs'

const web = join(dirname(fileURLToPath(import.meta.url)), '..')
const rust = join(web, '..', 'rust')
const exe = process.platform === 'win32' ? '.exe' : ''

execFileSync('cargo', ['build', '--quiet', '--bin', 'bagholder', '--bin', 'demo-book'], { cwd: rust, stdio: 'inherit' })
// a run stopped before it could clean up leaves its home behind (a saved sign-in in
// the signed-in one): each home is named for the server process that owns it, and
// one whose owner is gone is removed now
for (const name of readdirSync(tmpdir())) {
  // one named before homes named their owner has none left
  if (/^bagholder-e2e-[^-]+$/.test(name)) {
    rmSync(join(tmpdir(), name), { recursive: true, force: true })
    continue
  }
  const owner = Number(/^bagholder-e2e-(\d+)-/.exec(name)?.[1] ?? 0)
  if (!owner) continue
  let alive = false
  try { process.kill(owner, 0); alive = true } catch { alive = false }
  if (!alive) rmSync(join(tmpdir(), name), { recursive: true, force: true })
}
const home = mkdtempSync(join(tmpdir(), `bagholder-e2e-${process.pid}-`))
execFileSync(join(rust, 'target', 'debug', 'demo-book' + exe), ['--home', home, '--bars'], { stdio: 'inherit' })
// the book the engine reads, carried from it, with the made-up facts and prices beside it
execFileSync(join(rust, 'target', 'debug', 'bagholder' + exe), ['demo-facts', home], { stdio: 'inherit' })

// `--signed-in`: a saved Wealthsimple sign-in, and Wealthsimple stood in on loopback
// answering its search alone (`wealthsimple.mjs`): what the order path asks of it,
// which offline the app cannot reach. Every other call is answered as an error.
const signedIn = process.argv.includes('--signed-in')
const port = process.argv.find((a) => a.startsWith('--port='))?.slice('--port='.length) || process.env.E2E_PORT || '8791'
let wsBase = ''
if (signedIn) {
  const ws = createServer(standIn)
  await new Promise((ok) => ws.listen(0, '127.0.0.1', ok))
  wsBase = `http://127.0.0.1:${ws.address().port}`
  writeFileSync(join(home, 'session.json'), JSON.stringify({ access_token: 'e2e-access', refresh_token: 'e2e-refresh', client_id: 'e2e-client', identity_canonical_id: 'identity-e2e' }))
}

const server = spawn(join(rust, 'target', 'debug', 'bagholder' + exe), [], {
  stdio: 'inherit',
  env: {
    ...process.env,
    ...(signedIn ? { BAGHOLDER_WS_BASE: wsBase } : {}),
    BAGHOLDER_HOME: home,
    BAGHOLDER_PORT: port,
    BAGHOLDER_CHILD: '1', // no supervisor: this script is what starts and stops it
    BAGHOLDER_NO_BROWSER: '1',
    BAGHOLDER_DRY_ORDERS: '1',
    BAGHOLDER_OFFLINE: '1',
    BAGHOLDER_NOTIFY: 'browser',
  },
})
const stop = () => {
  server.kill('SIGTERM')
  rmSync(home, { recursive: true, force: true })
}
process.on('SIGTERM', () => { stop(); process.exit(0) })
process.on('SIGINT', () => { stop(); process.exit(0) })
server.on('exit', (code) => { rmSync(home, { recursive: true, force: true }); process.exit(code ?? 0) })
