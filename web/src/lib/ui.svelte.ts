// Small shared UI state for the header menu, modals and the confirm dialog —
// the pieces the legacy `state` object tracked (menuOpen, modal, confirmOpen).
import { status } from './subs.svelte'
import { applied } from './subs.svelte'
import { sort } from './sort.svelte'
import { call } from './api'
import { held, read } from './reads.svelte'
import { localDay, qty, waiting } from './fmt'
import { waits } from './dec'

// the server's own types, generated
import type { ImportReport as ImportedFileReport, WatchStatus } from './generated/model_api'
import type { Imported } from './generated/status'
import type { LoginInput } from './generated/session'
import type { EntryRequest, Kind as DataKind } from './generated/model_api'

export interface TradeForm {
  /** A trade, or an opening balance: what units that arrived without a cost cost. */
  mode: 'trade' | 'opening'
  date: string
  account: string
  symbol: string
  side: 'BUY' | 'SELL'
  qty: string
  price: string
  currency: 'CAD' | 'USD'
  fees: string
  /** The opening balance: the arrival it prices, its total cost and the day acquired. */
  arrival: string
  cost: string
  acquired: string
  error: string
}
/** One file's import: what it did, or why it was refused. */
export type ImportFileReport = { file: string; report: ImportedFileReport } | { file: string; error: string }
export interface ImportReport {
  files: ImportFileReport[]
}

function today(): string {
  return localDay()
}
function freshTradeForm(): TradeForm {
  return { mode: 'trade', date: today(), account: '', symbol: '', side: 'BUY', qty: '', price: '', currency: 'CAD', fees: '', arrival: '', cost: '', acquired: '', error: '' }
}

export const ui = $state<{
  menuOpen: boolean
  modal: '' | 'trade' | 'import' | 'folder'
  // '' | 'clear' | 'disconnect' | `cancel:<orderId>` | `bracket:<bracketId>`
  confirm: string
  /** what the header says of the person's own actions, oldest first, the newest showing: each until they close it */
  notices: Notice[]
  busy: '' | 'trade' | 'import' | 'folder' | 'clearing' | 'refresh'
  tradeForm: TradeForm
  importReport: ImportReport | null
  /** the account files are imported into, or a folder's go to; '' the Manual account */
  importAccount: string
  /** the kinds ticked in Clear data, and why the server refused them */
  clearKinds: DataKind[]
  clearError: string
  folderPath: string
  folderError: string
  watch: WatchStatus | null
  connecting: boolean
  loginView: boolean
  /** the side panels: a notification's card opens the Orders panel, so they are not the app shell's alone */
  ordersOpen: boolean
  notesOpen: boolean
}>({
  menuOpen: false,
  modal: '',
  confirm: '',
  notices: [],
  busy: '',
  tradeForm: freshTradeForm(),
  importReport: null,
  importAccount: '',
  clearKinds: [],
  clearError: '',
  folderPath: '',
  folderError: '',
  watch: null,
  ordersOpen: false,
  notesOpen: false,
  connecting: false,
  loginView: false,
})


/** A notice of an action's outcome: what it says, and whether it reports something not done. */
export type Notice = { msg: string; kind: '' | 'ok' | 'err' }

/** Say the outcome of something the person did in the header's status line, until
 * they close it (SPEC.md §4, the header): never on a timer, so nothing is gone
 * before it is read, copied or acted on. The newest shows, since it says what was
 * just done; one it covers shows again once it is closed, so none is gone unread.
 * The same words twice in a row are said once. */
export function tell(msg: string, kind: '' | 'ok' | 'err' = 'ok'): void {
  const last = ui.notices[ui.notices.length - 1]
  if (last && last.msg === msg && last.kind === kind) return
  ui.notices.push({ msg, kind })
}

/** The person closed the notice showing: the one it covered, if any, shows. */
export function closeNotice(): void {
  ui.notices.pop()
}

/** The notice showing: the newest not yet closed. */
export function showing(): Notice | undefined {
  return ui.notices[ui.notices.length - 1]
}

/** A failure of the page's own background work, which it recovers from by itself
 * (the next reading or asking answers it) and which leaves nothing the person
 * sees wrong: logged, never shown (SPEC.md §4, the header). */
export function quiet(what: string, why: string): void {
  console.warn(`Bagholder: ${what}: ${why}`)
}

// Sync: ask for it. Each step, and the end, reach the header as changes to the
// status (the server's state is written, the page is told) -- nothing here asks how
// it is going. The rows a sync brings arrive the same way, as they are committed.
export function syncNow(): void {
  ui.menuOpen = false
  const s = status.data
  if (s) {
    s.error = ''
    s.syncing = true
    s.syncStep = 'Syncing…'
  }
  call('POST /api/sync').then((r) => {
    if (r.ok) return
    const cur = status.data
    if (cur) {
      cur.syncing = false
      cur.error = r.error
    }
  })
}

export function refreshSession(): void {
  ui.menuOpen = false
  ui.busy = 'refresh'
  call('POST /api/refresh').then((r) => {
    ui.busy = ''
    if (r.ok) tell('Session refreshed')
    else tell(r.error, 'err')
  })
}

/** Install the release on offer. Its progress reaches the header as the status changes. */
export function updateNow(): void {
  call('POST /api/update').then((r) => {
    if (!r.ok) tell(r.error, 'err')
  })
}

// Connect to Wealthsimple: start the login browser and show the in-app sign-in
// window if the server streams one. How it goes is read off the status as it
// changes: the session landing (then sync), or the attempt ending without one. The
// deadline, three minutes to sign in, is the server's, and so is what the header
// then says: the page keeps no clock of its own to race it.
let sawCapturing = false
function endConnect(error: string): void {
  ui.connecting = false
  closeLoginView()
  const cur = status.data
  if (cur && error) cur.error = error
}
export function connect(): void {
  ui.menuOpen = false
  ui.connecting = true
  sawCapturing = false
  const s = status.data
  if (s) s.error = ''
  call('POST /api/login/start').then((res) => {
    if (!ui.connecting) return // cancelled meanwhile
    if (!res.ok) {
      endConnect(res.error)
      return
    }
    if (status.data?.loginView) ui.loginView = true
  })
}
/**
 * Follow a sign-in to its end from the status the server sends. Started by the page
 * when it mounts, not when this file is read: a file read first in a cycle of imports
 * would meet a store that is not there yet.
 */
export function followConnect(): () => void {
  return $effect.root(() => {
  $effect(() => {
    const st = status.data
    if (!ui.connecting || !st) return
    if (st.connected) {
      endConnect('')
      syncNow()
    } else if (st.capturing) {
      sawCapturing = true
    } else if (sawCapturing) {
      // the login window was there and is gone, with no session: the server says why
      endConnect((st.error as string) || '')
    }
  })
  })
}
function closeLoginView(): void {
  ui.loginView = false
}
export function cancelConnect(): void {
  endConnect('')
  const cur = status.data
  if (cur) cur.error = ''
  // the header goes back to its line at once; a cancel the server refused is said there
  call('POST /api/login/cancel').then((r) => {
    if (!r.ok) tell('Could not cancel the sign-in: ' + r.error, 'err')
  })
}
// One login input event (click/key/wheel/text), forwarded to the streamed browser.
export function loginInput(ev: LoginInput): void {
  call('POST /api/login/input', { body: ev }).then((r) => {
    if (!r.ok) tell('The sign-in window did not take that: ' + r.error, 'err')
  })
}

export function disconnect(): void {
  ui.menuOpen = false
  ui.confirm = 'disconnect'
}
export function disconnectNow(): void {
  ui.confirm = ''
  // the header says the session is gone when the server's status does; a refusal is said there meanwhile
  call('POST /api/disconnect').then((r) => {
    if (!r.ok) tell('Could not disconnect: ' + r.error, 'err')
  })
}

/** The kinds of data Clear data offers, in the order its dialog lists them. */
export const DATA_KINDS: { kind: DataKind; label: string }[] = [
  { kind: 'broker', label: 'Wealthsimple records' },
  { kind: 'entries', label: 'Your entries and imports' },
  { kind: 'journal', label: 'Journal' },
  { kind: 'market', label: 'Market data' },
  { kind: 'orders', label: 'Orders and brackets' },
  { kind: 'settings', label: 'Watchlist, tiles and notifications' },
  { kind: 'login', label: 'Wealthsimple login' },
]
export function openData(): void {
  ui.menuOpen = false
  ui.clearKinds = []
  ui.clearError = ''
  ui.confirm = 'clear'
}
/** Clear what is ticked: the dialog stays open, saying why, when the server refuses. */
export async function clearDataNow(): Promise<void> {
  if (!ui.clearKinds.length) return
  ui.busy = 'clearing'
  ui.clearError = ''
  const r = await call('POST /api/data/clear', { body: { kinds: ui.clearKinds } })
  ui.busy = ''
  if (!r.ok) {
    ui.clearError = r.error
    return
  }
  ui.confirm = ''
  tell('Data cleared')
}

export function openTradeModal(): void {
  ui.menuOpen = false
  ui.tradeForm = freshTradeForm()
  ui.modal = 'trade'
}
export function closeModal(): void {
  ui.modal = ''
}

// Save what the Add trade form holds: a trade, or an opening balance. Quantities and
// amounts go as the text typed (the server reads them as exact decimals and says what
// it cannot read); the figures move on the stream.
export async function saveTrade(): Promise<void> {
  const f = ui.tradeForm
  const text = (v: string) => v.trim().replace(/[$,]/g, '')
  let body: EntryRequest
  if (f.mode === 'opening') {
    if (!f.arrival || !text(f.cost) || !f.acquired) {
      f.error = 'The arrival, its cost and the day acquired are required.'
      return
    }
    body = { entry: 'cost-of-arrival', arrival: f.arrival, cost: text(f.cost), acquired: f.acquired }
  } else {
    if (!f.date || !f.symbol.trim() || !text(f.qty) || !text(f.price)) {
      f.error = 'Date, symbol, a quantity and a price are required.'
      return
    }
    body = { entry: 'trade', account: f.account, instrument: null, symbol: f.symbol.trim().toUpperCase(), currency: f.currency, day: f.date, side: f.side, quantity: text(f.qty), price: text(f.price), fee: text(f.fees) }
  }
  ui.busy = 'trade'
  f.error = ''
  const r = await call('POST /api/entries', { body })
  ui.busy = ''
  if (!r.ok) {
    f.error = r.error
    return
  }
  ui.modal = ''
  tell(f.mode === 'opening' ? 'Opening balance added' : 'Trade added')
}

// Import CSV: the account chosen, then files through the browser's picker, each
// sent to the server and its report shown.
export function importCsv(): void {
  ui.menuOpen = false
  ui.importReport = null
  ui.importAccount = ''
  ui.modal = 'import'
}
export function chooseFiles(): void {
  const input = document.createElement('input')
  input.type = 'file'
  input.accept = '.csv,text/csv'
  input.multiple = true
  input.hidden = true
  document.body.appendChild(input)
  input.addEventListener('change', () => {
    importFiles(input.files)
    input.remove()
  })
  input.click()
}
// An import runs on as a job once its file has arrived (issue #361): the upload is
// answered with its id, and what it did is the status's `imported`, whatever happens
// to the request or the page meanwhile.
const importWaits = new Map<string, (done: Imported) => void>()
/** The server keeps that a page has shown an import's report, for every page on every device. */
function told(id: string): void {
  call('POST /api/import/told', { body: { id } }).then((r) => {
    // not kept: the next page opened may say the report again, so this one says why
    if (!r.ok) tell(r.error, 'err')
  })
}
function fileReport(done: Imported): ImportFileReport {
  return done.report ? { file: done.file, report: done.report } : { file: done.file, error: done.error ?? '' }
}
// each import's end is handled once by this page, whatever else makes the effect run
const handled = new Set<string>()
$effect.root(() => {
  $effect(() => {
    const done = status.data?.imported
    if (!done || handled.has(done.id)) return
    const wait = importWaits.get(done.id)
    if (wait) {
      handled.add(done.id)
      importWaits.delete(done.id)
      told(done.id)
      wait(done)
      return
    }
    // while this page imports, an end it has not been answered for yet is its own
    if (ui.busy === 'import' || done.told) return
    // an import no page has shown (its page went first): said once, here
    handled.add(done.id)
    told(done.id)
    if (ui.modal === 'import') {
      ui.importReport = { files: [fileReport(done)] }
    } else if (done.report) {
      const n = done.report.added
      tell(`${done.file} · ${qty(n)} new ${n === 1 ? 'activity' : 'activities'} imported${done.report.stopped ? ' (stopped)' : ''}`)
    } else {
      tell(`${done.file}: ${done.error ?? ''}`, 'err')
    }
  })
})

async function importFiles(list: FileList | null): Promise<void> {
  const files = Array.from(list || []).filter((f) => /\.csv$/i.test(f.name) && !/^\._/.test(f.name))
  if (!files.length) return
  ui.busy = 'import'
  importStopped = false
  const report: ImportReport = { files: [] }
  for (const file of files) {
    // a file stopped is the last: the ones after it are not started
    if (importStopped) break
    const r = await call('POST /api/import', { query: { name: file.name, account: ui.importAccount }, body: file })
    if (!r.ok) {
      report.files.push({ file: file.name, error: r.error })
      continue
    }
    // its end, from the status: already there, or when it comes
    const now = status.data?.imported
    const done = now?.id === r.id ? now : await new Promise<Imported>((resolve) => importWaits.set(r.id, resolve))
    handled.add(r.id)
    told(r.id)
    report.files.push(fileReport(done))
  }
  ui.busy = ''
  ui.importReport = report
}

let importStopped = false
/** Stop the import running: its file ends at its next row (the rows kept stay), and no further file starts. */
export async function stopImport(): Promise<void> {
  importStopped = true
  const r = await call('POST /api/import/stop')
  if (!r.ok) tell(r.error, 'err')
}

export function openFolder(): void {
  ui.menuOpen = false
  ui.modal = 'folder'
  ui.folderError = ''
  ui.folderPath = ''
  ui.importAccount = ''
  // the folder last known is drawn at once; the server's answer replaces it
  const had = held('GET /api/watch')
  if (had?.ok) {
    ui.watch = had
    ui.folderPath = had.path
    ui.importAccount = had.account
  }
  read('GET /api/watch').then((w) => {
    if (!w.ok) ui.folderError = w.error
    else {
      // the folder watched fills the boxes, unless the person has typed or chosen already
      if (ui.folderPath === '' || ui.folderPath === ui.watch?.path) ui.folderPath = w.path
      if (ui.importAccount === '' || ui.importAccount === ui.watch?.account) ui.importAccount = w.account
      ui.watch = w
    }
  })
}
function watched(w: Awaited<ReturnType<typeof call<'GET /api/watch'>>>): void {
  ui.busy = ''
  if (!w.ok) ui.folderError = w.error
  else {
    ui.folderError = ''
    ui.watch = w
  }
}
/**
 * The header's notice after a scan of the watched folder (SPEC §4, the header): the
 * rows the scan's files added that the book did not hold, as the server counts them.
 * None when the scan failed: the folder's dialog says why.
 */
export function scanNotice(w: WatchStatus): string | null {
  if (w.scanError) return null
  const added = w.lastScanAdded
  return added ? `${added} new ${added === 1 ? 'activity' : 'activities'} imported` : 'Folder scanned · nothing new'
}
function scanned(w: Awaited<ReturnType<typeof call<'POST /api/watch/scan'>>>): void {
  watched(w)
  const notice = w.ok ? scanNotice(w) : null
  if (notice) tell(notice)
}
export function watchFolder(): void {
  ui.busy = 'folder'
  ui.folderError = ''
  call('POST /api/watch', { body: { path: ui.folderPath, account: ui.importAccount } }).then(scanned)
}
export function scanFolder(): void {
  ui.busy = 'folder'
  call('POST /api/watch/scan').then(scanned)
}
export function stopWatch(): void {
  call('POST /api/watch/clear').then((w) => {
    watched(w)
    ui.folderPath = ''
    ui.importAccount = ''
  })
}

// Export trades to CSV, client-side, matching the legacy exportCsv().
export async function exportCsv(): Promise<void> {
  ui.menuOpen = false
  // every trade under the filters applied, in the list's order: the list on screen shows only as far as scrolled
  const s = sort.trades
  const m = await read('GET /api/figures/trades', { query: { filters: JSON.stringify(applied.filters), sort: s.key, dir: s.dir } })
  if (!m.ok) {
    tell('Could not export the trades: ' + m.error, 'err')
    return
  }
  const cols = ['Open', 'Close', 'Symbol', 'Name', 'Account', 'Kind', 'Side', 'Status', 'Qty', 'Entry', 'Exit', 'Currency', 'P&L', 'P&L CAD', 'Fees', 'Hold days', 'Grade', 'Tags', 'Thesis']
  // an amount is written as the exact decimal the server sent; one that waits, as the page shows it
  const cell = (v: unknown) => (waits(v as never) ? waiting(v as { gaps: string[] }) : v)
  const q = (v: unknown) => '"' + String(v == null ? '' : cell(v)).replace(/"/g, '""') + '"'
  const lines = [cols.map(q).join(',')].concat(
    (m.trades || []).map((t) =>
      [t.entryDate, t.exitDate, t.symbol, t.name, t.account, t.kind, t.side, t.status, t.qty, t.entry, t.exit, t.currency, t.pnl, t.pnlCad, t.fees, t.holdDays, t.grade, (t.tags || []).join('; '), t.thesis].map(q).join(','),
    ),
  )
  const blob = new Blob([lines.join('\n')], { type: 'text/csv' })
  const a = document.createElement('a')
  a.href = URL.createObjectURL(blob)
  a.download = 'bagholder-trades.csv'
  a.click()
  URL.revokeObjectURL(a.href)
}
