// Small shared UI state for the header menu, modals and the confirm dialog —
// the pieces the legacy `state` object tracked (menuOpen, modal, confirmOpen).
import { store } from './state.svelte'
import { request, call } from './api'
import { localDay, waiting } from './fmt'
import { waits } from './dec'

// the server's own types, generated
import type { ImportReport as ImportedFileReport, WatchStatus } from './generated/model_api'
import type { LoginInput } from './generated/session'
import type { EntryRequest } from './generated/model_api'

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
  notice: string
  noticeKind: '' | 'ok' | 'err'
  busy: '' | 'trade' | 'import' | 'folder' | 'clearing' | 'refresh'
  tradeForm: TradeForm
  importReport: ImportReport | null
  /** the account files are imported into, or a folder's go to; '' the Manual account */
  importAccount: string
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
  notice: '',
  noticeKind: '',
  busy: '',
  tradeForm: freshTradeForm(),
  importReport: null,
  importAccount: '',
  folderPath: '',
  folderError: '',
  watch: null,
  ordersOpen: false,
  notesOpen: false,
  connecting: false,
  loginView: false,
})


/** Say something in the header's status line for a while. */
export function flash(msg: string, kind: '' | 'ok' | 'err' = 'ok', ms = 4000) {
  ui.notice = msg
  ui.noticeKind = kind
  setTimeout(() => {
    if (ui.notice === msg) {
      ui.notice = ''
      ui.noticeKind = ''
    }
  }, ms)
}

// Sync: ask for it. Each step, and the end, reach the header as changes to the
// status (the server's state is written, the page is told) -- nothing here asks how
// it is going. The rows a sync brings arrive the same way, as they are committed.
export function syncNow(): void {
  ui.menuOpen = false
  const s = store.model?.status
  if (s) {
    s.error = ''
    s.syncing = true
    s.syncStep = 'Syncing…'
  }
  call('POST /api/sync').then((r) => {
    if (r && r.ok) return
    const cur = store.model?.status
    if (cur) {
      cur.syncing = false
      cur.error = (r && (r.error as string)) || 'Sync failed.'
    }
  })
}

export function refreshSession(): void {
  ui.menuOpen = false
  ui.busy = 'refresh'
  call('POST /api/refresh').then((r) => {
    ui.busy = ''
    flash(r && r.ok ? 'Session refreshed' : (r && (r.error as string)) || 'Refresh failed', r && r.ok ? 'ok' : 'err')
  })
}

/** Install the release on offer. Its progress reaches the header as the status changes. */
export function updateNow(): void {
  call('POST /api/update').then((r) => {
    if (!r || !r.ok) flash((r && (r.error as string)) || 'Update failed.', 'err')
  })
}

// Connect to Wealthsimple: start the login browser and show the in-app sign-in
// window if the server streams one. How it goes is read off the status as it
// changes: the session landing (then sync), or the attempt ending without one. The
// one timer is a deadline -- three minutes to sign in -- not a question asked again.
const NO_SESSION = 'No session yet. Finish login in the Chrome window, then try Sync now.'
let sawCapturing = false
let connectDeadline: ReturnType<typeof setTimeout> | undefined
function endConnect(error: string): void {
  clearTimeout(connectDeadline)
  ui.connecting = false
  closeLoginView()
  const cur = store.model?.status
  if (cur && error) cur.error = error
}
export function connect(): void {
  ui.menuOpen = false
  ui.connecting = true
  sawCapturing = false
  const s = store.model?.status
  if (s) s.error = ''
  call('POST /api/login/start').then((res) => {
    if (!ui.connecting) return // cancelled meanwhile
    if (!res || !res.ok) {
      endConnect((res && (res.error as string)) || 'Install Chrome. Passkey login has to happen on Wealthsimple’s site.')
      return
    }
    if (store.model?.status?.loginView) ui.loginView = true
    clearTimeout(connectDeadline)
    connectDeadline = setTimeout(() => ui.connecting && endConnect(NO_SESSION), 180000)
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
    const st = store.model?.status
    if (!ui.connecting || !st) return
    if (st.connected) {
      endConnect('')
      syncNow()
    } else if (st.capturing) {
      sawCapturing = true
    } else if (sawCapturing) {
      // the login window was there and is gone, with no session
      endConnect((st.error as string) || NO_SESSION)
    }
  })
  })
}
function closeLoginView(): void {
  ui.loginView = false
}
export function cancelConnect(): void {
  endConnect('')
  const cur = store.model?.status
  if (cur) cur.error = ''
  call('POST /api/login/cancel')
}
// One login input event (click/key/wheel/text), forwarded to the streamed browser.
export function loginInput(ev: LoginInput): void {
  call('POST /api/login/input', { body: ev })
}

export function disconnect(): void {
  ui.menuOpen = false
  ui.confirm = 'disconnect'
}
export function disconnectNow(): void {
  ui.confirm = ''
  call('POST /api/disconnect')
}

export function openData(): void {
  ui.menuOpen = false
  ui.confirm = 'clear'
}
export function clearDataNow(): void {
  ui.confirm = ''
  call('POST /api/data/clear', { body: { journal: true, market: true } })
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
    f.error = r.error || 'Could not save it.'
    return
  }
  ui.modal = ''
  flash(f.mode === 'opening' ? 'Opening balance added' : 'Trade added')
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
async function importFiles(list: FileList | null): Promise<void> {
  const files = Array.from(list || []).filter((f) => /\.csv$/i.test(f.name) && !/^\._/.test(f.name))
  if (!files.length) return
  ui.busy = 'import'
  const report: ImportReport = { files: [] }
  for (const file of files) {
    let text: string
    try {
      text = await file.text()
    } catch (e) {
      report.files.push({ file: file.name, error: String(e) })
      continue
    }
    const r = await call('POST /api/import', { body: { name: file.name, text, account: ui.importAccount } })
    if (r.ok === false || typeof r.rows !== 'number') report.files.push({ file: file.name, error: r.error || 'Import failed' })
    else report.files.push({ file: file.name, report: r })
  }
  ui.busy = ''
  ui.importReport = report
}

export function openFolder(): void {
  ui.menuOpen = false
  ui.modal = 'folder'
  ui.folderError = ''
  call('GET /api/watch').then((w) => {
    if (w.ok === false) ui.folderError = w.error || 'Could not read the watched folder.'
    else {
      ui.watch = w
      ui.folderPath = w.path
      ui.importAccount = w.account
    }
  })
}
function watched(w: Awaited<ReturnType<typeof call<'GET /api/watch'>>>): void {
  ui.busy = ''
  if (w.ok === false) ui.folderError = w.error || 'Could not watch that folder.'
  else {
    ui.folderError = ''
    ui.watch = w
  }
}
export function watchFolder(): void {
  ui.busy = 'folder'
  ui.folderError = ''
  call('POST /api/watch', { body: { path: ui.folderPath, account: ui.importAccount } }).then(watched)
}
export function scanFolder(): void {
  ui.busy = 'folder'
  call('POST /api/watch/scan').then(watched)
}
export function stopWatch(): void {
  call('POST /api/watch/clear').then((w) => {
    watched(w)
    ui.folderPath = ''
    ui.importAccount = ''
  })
}

// Export trades to CSV, client-side, matching the legacy exportCsv().
export function exportCsv(): void {
  ui.menuOpen = false
  const m = store.model
  if (!m) return
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
