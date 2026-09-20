// Small formatters the Markets screen shares, ported verbatim from ledger.html
// so every figure matches the reference page. Kept local to markets/ to avoid
// touching the shared fmt module.

const MON = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec']

// en-US grouping with a fixed number of decimals (ledger's n2).
export function n2(v: number, dp: number): string {
  return Number(v).toLocaleString('en-US', { minimumFractionDigits: dp, maximumFractionDigits: dp })
}

// A signed percentage already in percent units (not a fraction): +3.92%, −1.67%,
// the em-minus, two decimals, "—" when absent. This is ledger's signedPct.
export function signedPct(v: number | null | undefined): string {
  return v == null || !isFinite(v) ? '—' : (v < 0 ? '−' : '+') + Math.abs(v).toFixed(2) + '%'
}

// "Sep 19" from an ISO date; "" when it is not a full YYYY-MM-DD.
export function shortDay(iso: string | null | undefined): string {
  const p = String(iso || '').split('-')
  return p.length === 3 ? MON[Number(p[1]) - 1] + ' ' + Number(p[2]) : ''
}

// "19 Sep 2026" from a filing's date/dateText (ledger's discDate).
export function discDate(item: { date?: string; dateText?: string }): string {
  const iso = String(item.date || '').slice(0, 10)
  const d = new Date(iso + 'T00:00:00')
  if (isNaN(d.getTime())) return item.dateText || iso
  return d.getDate() + ' ' + MON[d.getMonth()] + ' ' + d.getFullYear()
}

// The news "When" column: a time today, otherwise a day (with the year when it
// falls outside this one). Ported from ledger's newsWhen.
export function newsWhen(iso: string): string {
  const t = Date.parse(iso)
  if (!isFinite(t)) return '—'
  const d = new Date(t)
  const now = new Date()
  if (d.toDateString() === now.toDateString()) return (d.getHours() % 12 || 12) + ':' + String(d.getMinutes()).padStart(2, '0') + ' ' + (d.getHours() < 12 ? 'AM' : 'PM')
  return MON[d.getMonth()] + ' ' + d.getDate() + (d.getFullYear() !== now.getFullYear() ? ' ' + d.getFullYear() : '')
}

// The page's own copy of the model's headline key, so a wire release and a filed
// release with the same title collapse to one row (ledger's news_text_key).
export function newsTextKey(h: string): string {
  return String(h || '').toLowerCase().replace(/[^a-z0-9]+/g, ' ').trim()
}

// A ticker as the JSON body sends it. Small shared helper for the fetch calls.
export function api<T = unknown>(method: string, path: string, body?: unknown): Promise<T> {
  const opts: RequestInit = { method, headers: { 'X-Bagholder': '1' } }
  if (body !== undefined) {
    ;(opts.headers as Record<string, string>)['Content-Type'] = 'application/json'
    opts.body = JSON.stringify(body)
  }
  return fetch(path, opts)
    .then((r) => r.json())
    .catch((e) => ({ ok: false, error: String(e) })) as Promise<T>
}
