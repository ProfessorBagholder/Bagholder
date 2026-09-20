// Small shared UI state for the header menu, modals and the confirm dialog —
// the pieces the legacy `state` object tracked (menuOpen, modal, confirmOpen).
import { store, loadModel } from './state.svelte'

export interface TradeForm {
  date: string
  account: string
  symbol: string
  side: 'BUY' | 'SELL'
  qty: string
  price: string
  currency: 'CAD' | 'USD'
  fees: string
  error: string
}
export interface ImportFileReport {
  file: string
  error?: string
  format?: string
  rows?: number
  added?: number
  duplicates?: number
  skipped?: { row: number; message: string }[]
  skippedCount?: number
}
export interface ImportReport {
  files: ImportFileReport[]
  added: number
  duplicates: number
}

function today(): string {
  return new Date().toISOString().slice(0, 10)
}
function freshTradeForm(): TradeForm {
  return { date: today(), account: '', symbol: '', side: 'BUY', qty: '', price: '', currency: 'CAD', fees: '', error: '' }
}

export const ui = $state<{
  menuOpen: boolean
  modal: '' | 'trade' | 'import' | 'folder'
  // '' | 'clear' | 'disconnect' | `cancel:<orderId>` | `bracket:<bracketId>`
  confirm: string
  notice: string
  noticeKind: '' | 'ok' | 'err'
  busy: '' | 'trade' | 'folder' | 'clearing'
  tradeForm: TradeForm
  importReport: ImportReport | null
  folderPath: string
  folderError: string
  watch: { watching: boolean; path: string; lastScan?: string; files?: { file: string; format?: string; added: number; duplicates: number }[] } | null
  connecting: boolean
  loginView: boolean
}>({
  menuOpen: false,
  modal: '',
  confirm: '',
  notice: '',
  noticeKind: '',
  busy: '',
  tradeForm: freshTradeForm(),
  importReport: null,
  folderPath: '',
  folderError: '',
  watch: null,
  connecting: false,
  loginView: false,
})

function api(method: string, path: string, body?: unknown): Promise<{ ok?: boolean; error?: string; [k: string]: unknown }> {
  const opts: RequestInit = { method, headers: { 'X-Bagholder': '1' } }
  if (body !== undefined) {
    ;(opts.headers as Record<string, string>)['Content-Type'] = 'application/json'
    opts.body = JSON.stringify(body)
  }
  return fetch(path, opts)
    .then((r) => r.json())
    .catch((e) => ({ ok: false, error: String(e) }))
}

function flash(msg: string, kind: '' | 'ok' | 'err' = 'ok', ms = 4000) {
  ui.notice = msg
  ui.noticeKind = kind
  setTimeout(() => {
    if (ui.notice === msg) {
      ui.notice = ''
      ui.noticeKind = ''
    }
  }, ms)
}

export function syncNow(): void {
  ui.menuOpen = false
  const s = store.model?.status
  if (s) {
    s.error = ''
    s.syncing = true
    s.syncStep = 'Syncing…'
  }
  api('POST', '/api/sync').then(() => loadModel())
}

export function refreshSession(): void {
  ui.menuOpen = false
  api('POST', '/api/refresh').then((r) => {
    flash(r && r.ok ? 'Session refreshed' : (r && (r.error as string)) || 'Refresh failed', r && r.ok ? 'ok' : 'err')
    loadModel()
  })
}

// Connect to Wealthsimple, ported from ledger.html connect(): start the login
// browser, show the in-app sign-in window if the server streams one, then poll
// status until the session lands (then sync) or the attempt ends/times out.
export function connect(): void {
  ui.menuOpen = false
  ui.connecting = true
  const s = store.model?.status
  if (s) s.error = ''
  api('POST', '/api/login/start').then((res) => {
    if (!res || !res.ok) {
      ui.connecting = false
      const st = store.model?.status
      if (st) st.error = (res && (res.error as string)) || 'Install Chrome. Passkey login has to happen on Wealthsimple’s site.'
      return
    }
    if (store.model?.status?.loginView) ui.loginView = true
    const t0 = Date.now()
    const tick = () =>
      api('GET', '/api/status').then((st) => {
        if (!ui.connecting) return // cancelled meanwhile
        if (store.model && st && st.ok) store.model.status = st as unknown as typeof store.model.status
        if (st && st.connected) {
          ui.connecting = false
          closeLoginView()
          syncNow()
          return
        }
        if (st && !st.capturing) {
          ui.connecting = false
          closeLoginView()
          const cur = store.model?.status
          if (cur) cur.error = (st.error as string) || 'No session yet. Finish login in the Chrome window, then try Sync now.'
          return
        }
        if (Date.now() - t0 > 180000) {
          ui.connecting = false
          closeLoginView()
          const cur = store.model?.status
          if (cur) cur.error = 'No session yet. Finish login in the Chrome window, then try Sync now.'
          return
        }
        setTimeout(tick, 500)
      })
    tick()
  })
}
function closeLoginView(): void {
  ui.loginView = false
}
export function cancelConnect(): void {
  ui.connecting = false
  closeLoginView()
  const cur = store.model?.status
  if (cur) cur.error = ''
  api('POST', '/api/login/cancel').then(() => loadModel())
}
// One login input event (click/key/wheel/text), forwarded to the streamed browser.
export function loginInput(ev: unknown): void {
  api('POST', '/api/login/input', ev)
}

export function disconnect(): void {
  ui.menuOpen = false
  ui.confirm = 'disconnect'
}
export function disconnectNow(): void {
  ui.confirm = ''
  api('POST', '/api/disconnect').then(() => loadModel())
}

export function openData(): void {
  ui.menuOpen = false
  ui.confirm = 'clear'
}
export function clearDataNow(): void {
  ui.confirm = ''
  api('POST', '/api/data/clear', { journal: true, market: true }).then(() => loadModel())
}

export function openTradeModal(): void {
  ui.menuOpen = false
  ui.tradeForm = freshTradeForm()
  ui.modal = 'trade'
}
export function closeModal(): void {
  ui.modal = ''
}

// Save a manually-entered trade, matching the legacy saveTrade()/book/append.
export function saveTrade(accounts: { id: string; name: string }[]): void {
  const f = ui.tradeForm
  const qty = Number(String(f.qty).replace(/,/g, ''))
  const price = Number(String(f.price).replace(/[$,]/g, ''))
  const fees = Number(String(f.fees || 0).replace(/[$,]/g, ''))
  if (!f.date || !f.symbol || !(qty > 0) || !(price >= 0) || isNaN(fees)) {
    f.error = 'Date, symbol, a positive quantity and a price are required.'
    return
  }
  const acc = accounts.find((a) => a.id === f.account)
  ui.busy = 'trade'
  f.error = ''
  api('POST', '/api/book/append', {
    date: f.date,
    symbol: f.symbol.trim().toUpperCase(),
    side: f.side,
    qty,
    price,
    currency: f.currency,
    commission: fees,
    accountId: acc ? acc.id : 'manual',
    accountType: acc ? acc.name : 'Manual',
  }).then((r) => {
    ui.busy = ''
    if (!r || !r.ok) {
      f.error = (r && (r.error as string)) || 'Could not save the trade.'
      return
    }
    ui.modal = ''
    flash(r.added ? 'Trade added' : 'That trade was already recorded')
    loadModel()
  })
}

// Import CSVs via a native file picker, then show the report modal.
export function importCsv(): void {
  ui.menuOpen = false
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
  ui.modal = 'import'
  ui.importReport = null
  const report: ImportReport = { files: [], added: 0, duplicates: 0 }
  for (const file of files) {
    try {
      const text = await file.text()
      const r = await api('POST', '/api/import', { name: file.name, text })
      if (!r || !r.ok) report.files.push({ file: file.name, error: (r && (r.error as string)) || 'Import failed' })
      else {
        report.files.push(r as unknown as ImportFileReport)
        report.added += (r.added as number) || 0
        report.duplicates += (r.duplicates as number) || 0
      }
    } catch (e) {
      report.files.push({ file: file.name, error: String(e) })
    }
  }
  ui.importReport = report
  if (report.added) loadModel()
}

export function openFolder(): void {
  ui.menuOpen = false
  ui.modal = 'folder'
  api('GET', '/api/watch').then((w) => {
    ui.watch = w && w.ok ? (w as typeof ui.watch) : null
  })
}
export function watchFolder(): void {
  ui.busy = 'folder'
  ui.folderError = ''
  api('POST', '/api/watch', { path: ui.folderPath }).then((r) => {
    ui.busy = ''
    if (!r || !r.ok) ui.folderError = (r && (r.error as string)) || 'Could not watch that folder.'
    else {
      ui.watch = r as typeof ui.watch
      loadModel()
    }
  })
}
export function scanFolder(): void {
  ui.busy = 'folder'
  api('POST', '/api/watch/scan', {}).then((r) => {
    ui.busy = ''
    if (r && r.ok) {
      ui.watch = r as typeof ui.watch
      loadModel()
    }
  })
}
export function stopWatch(): void {
  api('POST', '/api/watch/clear', {}).then((w) => {
    ui.watch = w && w.ok ? (w as typeof ui.watch) : null
    ui.folderPath = ''
  })
}

// Export trades to CSV, client-side, matching the legacy exportCsv().
export function exportCsv(): void {
  ui.menuOpen = false
  const m = store.model
  if (!m) return
  const cols = ['Open', 'Close', 'Symbol', 'Name', 'Account', 'Kind', 'Side', 'Status', 'Qty', 'Entry', 'Exit', 'Currency', 'P&L', 'P&L CAD', 'Fees', 'Hold days', 'Grade', 'Tags', 'Thesis']
  const q = (v: unknown) => '"' + String(v == null ? '' : v).replace(/"/g, '""') + '"'
  const lines = [cols.map(q).join(',')].concat(
    (m.trades || []).map((t) =>
      [t.entryDate, t.exitDate, t.symbol, t.name, t.account, t.kind, t.side, t.status, t.qty, t.entry, t.exit, t.currency, t.pnl.toFixed(2), t.pnlCad.toFixed(2), (t.fees ?? 0).toFixed(2), t.holdDays, t.grade, (t.tags || []).join('; '), t.thesis].map(q).join(','),
    ),
  )
  const blob = new Blob([lines.join('\n')], { type: 'text/csv' })
  const a = document.createElement('a')
  a.href = URL.createObjectURL(blob)
  a.download = 'bagholder-trades.csv'
  a.click()
  URL.revokeObjectURL(a.href)
}

export function notifyToggle(kind: string): void {
  const cur = store.model?.status?.notify
  if (!cur) return
  const patch = { [kind]: !cur[kind] }
  Object.assign(cur, patch)
  api('POST', '/api/notifications/settings', patch)
}
