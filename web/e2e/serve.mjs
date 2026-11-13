// Builds the server and the made-up book, then runs the server on it until told
// to stop. Started by Playwright (`webServer` in playwright.config.ts).
import { execFileSync, spawn } from 'node:child_process'
import { appendFileSync, mkdtempSync, openSync, readdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { createServer } from 'node:http'
import { tmpdir } from 'node:os'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { onAnswered, standIn } from './wealthsimple.mjs'

const web = join(dirname(fileURLToPath(import.meta.url)), '..')
const rust = join(web, '..', 'rust')
const exe = process.platform === 'win32' ? '.exe' : ''
// where cargo builds: its own CARGO_TARGET_DIR where one is set, else the workspace's
const target = process.env.CARGO_TARGET_DIR ? resolve(rust, process.env.CARGO_TARGET_DIR) : join(rust, 'target')

// with the made-up book (`bagholder demo`), a developer's subcommand no release build carries
execFileSync('cargo', ['build', '--quiet', '--bin', 'bagholder', '--features', 'demo'], { cwd: rust, stdio: 'inherit' })
const signedIn = process.argv.includes('--signed-in')
const port = process.argv.find((a) => a.startsWith('--port='))?.slice('--port='.length) || process.env.E2E_PORT || '8791'
// where a signed-in server names every request it makes (`BAGHOLDER_LOG_REQUESTS`)
const requestLog = process.argv.find((a) => a.startsWith('--requests='))?.slice('--requests='.length) || ''

// a run stopped before it could clean up leaves its home behind (a saved sign-in in
// the signed-in one): each home is named for the server process that owns it, and
// one whose owner is gone is removed now
for (const name of readdirSync(tmpdir())) {
  // one named before homes named their owner has none left
  if (/^bagholder-e2e-[^-]+$/.test(name) || /^bagholder-e2e-\d+-[^-]+$/.test(name)) {
    rmSync(join(tmpdir(), name), { recursive: true, force: true })
    continue
  }
  const owner = Number(/^bagholder-e2e-\d+-(\d+)-/.exec(name)?.[1] ?? 0)
  if (!owner) continue
  let alive = false
  try { process.kill(owner, 0); alive = true } catch { alive = false }
  if (!alive) rmSync(join(tmpdir(), name), { recursive: true, force: true })
}
// named for the port and the process that owns it
const home = mkdtempSync(join(tmpdir(), `bagholder-e2e-${port}-${process.pid}-`))
// the made-up book, pulled from a made-up Wealthsimple, with its facts and prices beside it
execFileSync(join(target, 'debug', 'bagholder' + exe), ['demo', home], { stdio: 'inherit' })

// `--signed-in`: a saved Wealthsimple sign-in, and Wealthsimple stood in on loopback
// answering its search alone (`wealthsimple.mjs`): what the order path asks of it,
// which offline the app cannot reach. Every other call is answered as an error.
let wsBase = ''
if (signedIn) {
  // each call the stand-in answers, beside the server's own log of what it asked
  if (requestLog) {
    writeFileSync(`${requestLog}.answered`, '')
    onAnswered((call) => appendFileSync(`${requestLog}.answered`, call + '\n'))
  }
  const ws = createServer(standIn)
  await new Promise((ok) => ws.listen(0, '127.0.0.1', ok))
  wsBase = `http://127.0.0.1:${ws.address().port}`
  writeFileSync(join(home, 'session.json'), JSON.stringify({ access_token: 'e2e-access', refresh_token: 'e2e-refresh', client_id: 'e2e-client', identity_canonical_id: 'identity-e2e' }))
}

// the Options card's sources stood in on loopback (`BAGHOLDER_OPTIONS_BASE`), for the
// server the app's tests drive: Cboe's delayed chain of KO, and of AAPL (KO's with
// each strike six times over, about the made-up book's AAPL), and Yahoo's daily
// chart of one KO put, from the server's own made-up captures; Cboe's 403 for any
// other symbol (it lists no options of it) and Yahoo's 404 for any other contract
let optionsBase = ''
if (!signedIn) {
  const replies = join(rust, 'crates', 'server', 'tests', 'replies', 'options-card')
  const chain = readFileSync(join(replies, 'chain-KO.json'), 'utf8')
  const aapl = chain
    .replace(/"option": "KO(\d?)(\d{6})([CP])(\d{8})"/g, (_, adjusted, day, right, strike) => `"option": "AAPL${adjusted}${day}${right}${String(Number(strike) * 6).padStart(8, '0')}"`)
    .replaceAll('"symbol": "KO"', '"symbol": "AAPL"')
  const chains = { KO: chain, AAPL: aapl }
  const charts = { KO261211P00066000: readFileSync(join(replies, 'chart-KO261211P00066000.json'), 'utf8') }
  const options = createServer((req, res) => {
    const path = new URL(req.url ?? '/', 'http://stand-in').pathname
    const chainOf = /^\/api\/global\/delayed_quotes\/options\/([A-Z.]+)\.json$/.exec(path)?.[1]
    const chartOf = /^\/v8\/finance\/chart\/([A-Z0-9]+)$/.exec(path)?.[1]
    const body = chainOf ? chains[chainOf] : chartOf ? charts[chartOf] : undefined
    res.writeHead(body ? 200 : chainOf ? 403 : 404, { 'content-type': 'application/json' })
    res.end(body ?? '')
  })
  await new Promise((ok) => options.listen(0, '127.0.0.1', ok))
  optionsBase = `http://127.0.0.1:${options.address().port}`
}
const logFd = requestLog ? openSync(requestLog, 'w') : null
const server = spawn(join(target, 'debug', 'bagholder' + exe), [], {
  stdio: ['inherit', 'inherit', logFd ?? 'inherit'],
  env: {
    ...process.env,
    ...(signedIn ? { BAGHOLDER_WS_BASE: wsBase } : {}),
    ...(optionsBase ? { BAGHOLDER_OPTIONS_BASE: optionsBase } : {}),
    ...(requestLog ? { BAGHOLDER_LOG_REQUESTS: '1' } : {}),
    BAGHOLDER_HOME: home,
    BAGHOLDER_EXIT_WITH: String(process.pid), // it ends when this script does, however this script ends
    BAGHOLDER_PORT: port,
    BAGHOLDER_CHILD: '1', // no supervisor: this script is what starts and stops it
    BAGHOLDER_NO_BROWSER: '1',
    BAGHOLDER_DRY_ORDERS: '1',
    BAGHOLDER_OFFLINE: '1',
  },
})
// stopped: the server first, and its home once it has exited (it writes there until then)
const stop = () => server.kill('SIGTERM')
process.on('SIGTERM', stop)
process.on('SIGINT', stop)
server.on('exit', (code) => {
  rmSync(home, { recursive: true, force: true })
  if (requestLog) {
    rmSync(requestLog, { force: true })
    rmSync(`${requestLog}.answered`, { force: true })
  }
  process.exit(code ?? 0)
})
