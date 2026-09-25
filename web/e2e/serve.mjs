// Builds the server and the made-up book, then runs the server on it until told
// to stop. Started by Playwright (`webServer` in playwright.config.ts).
import { execFileSync, spawn } from 'node:child_process'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const web = join(dirname(fileURLToPath(import.meta.url)), '..')
const rust = join(web, '..', 'rust')
const exe = process.platform === 'win32' ? '.exe' : ''

execFileSync('cargo', ['build', '--quiet', '--bin', 'bagholder', '--bin', 'demo-book'], { cwd: rust, stdio: 'inherit' })
const home = mkdtempSync(join(tmpdir(), 'bagholder-e2e-'))
execFileSync(join(rust, 'target', 'debug', 'demo-book' + exe), ['--home', home, '--bars'], { stdio: 'inherit' })
// the book the engine reads, carried from it, with the made-up facts and prices beside it
execFileSync(join(rust, 'target', 'debug', 'bagholder' + exe), ['demo-facts', home], { stdio: 'inherit' })

const server = spawn(join(rust, 'target', 'debug', 'bagholder' + exe), [], {
  stdio: 'inherit',
  env: {
    ...process.env,
    BAGHOLDER_HOME: home,
    BAGHOLDER_PORT: process.env.E2E_PORT || '8791',
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
