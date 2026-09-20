// Short interest per listing, read from /api/shorts and cached. Ported from
// ledger.html (_shorts / ensureShorts / shortsKey / shortDay / shortSpan). The
// store is reactive $state so the ShortInterest card renders when a fetch lands.
import type { Trade, ShortsResp } from '../model'
import { listingTicker } from './chart'

const MON = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec']

export interface ShortsRec extends ShortsResp {
  at: number
}

export const shortsStore = $state<Record<string, ShortsRec>>({})
const _pending: Record<string, boolean> = {}
const _trend: Record<string, boolean> = {}

const SHORTS_KEEP_MS = 30 * 60 * 1000

export function discSymbol(t: Trade): string {
  return String(listingTicker(t) || t.symbol || '').toUpperCase()
}
export function shortsKey(t: Trade): string {
  return discSymbol(t) + '|' + String(t.exchange || '').toUpperCase()
}

export function ensureShorts(t: Trade): void {
  const sym = discSymbol(t)
  const key = shortsKey(t)
  const held = shortsStore[key]
  if (!sym || _pending[key] || (held && Date.now() - held.at < SHORTS_KEEP_MS)) return
  _pending[key] = true
  const q =
    'symbol=' + encodeURIComponent(sym) +
    (t.exchange ? '&exchange=' + encodeURIComponent(t.exchange) : '') +
    (t.currency ? '&currency=' + encodeURIComponent(t.currency) : '')
  fetch('/api/shorts?' + q, { headers: { 'X-Bagholder': '1' } })
    .then((r) => r.json())
    .catch(() => ({ ok: false }))
    .then((d: any) => {
      delete _pending[key]
      shortsStore[key] = Object.assign({ at: Date.now() }, d && d.ok ? d : { ok: false })
      // the run of past reports comes free with a US listing's answer; a Canadian
      // one is a file per reporting date, asked for once the figures are on screen
      if (d && d.ok && d.covered && !((d.shorts || {}).series || []).length && !_trend[key]) {
        _trend[key] = true
        fetch('/api/shorts?' + q + '&trend=1', { headers: { 'X-Bagholder': '1' } })
          .then((r) => r.json())
          .catch(() => ({ ok: false }))
          .then((more: any) => {
            const rec = shortsStore[key]
            if (more && more.ok && more.covered && rec && rec.shorts) rec.shorts.series = (more.shorts || {}).series || []
          })
      }
    })
}

export function shortDay(iso: string | null | undefined): string {
  const p = String(iso || '').split('-')
  return p.length === 3 ? MON[Number(p[1]) - 1] + ' ' + Number(p[2]) : ''
}
export function shortSpan(key: string | null | undefined): string {
  const ends = String(key || '').split('/')
  if (ends.length !== 2) return shortDay(key)
  const a = ends[0].split('-')
  const b = ends[1].split('-')
  if (a.length !== 3 || b.length !== 3) return ''
  return a[1] === b[1]
    ? MON[Number(a[1]) - 1] + ' ' + Number(a[2]) + '–' + Number(b[2])
    : shortDay(ends[0]) + ' – ' + shortDay(ends[1])
}
