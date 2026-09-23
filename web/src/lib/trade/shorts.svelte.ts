// Short interest per listing, read from /api/shorts and cached. Ported from
// ledger.html (_shorts / ensureShorts / shortsKey / shortDay / shortSpan). The
// store is reactive $state so the ShortInterest card renders when a fetch lands.
import type { Trade, ShortsPayload } from '../model'
import { listingTicker } from './chart'
import { lookup, query, type Answer } from '../api'

const MON = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec']

export interface ShortsRec extends ShortsPayload {
  at: number
}

export const shortsStore = $state<Record<string, ShortsRec>>({})

const SHORTS_KEEP_MS = 30 * 60 * 1000

export function discSymbol(t: Trade): string {
  return String(listingTicker(t) || t.symbol || '').toUpperCase()
}
export function shortsKey(t: Trade): string {
  return discSymbol(t) + '|' + String(t.exchange || '').toUpperCase()
}

// a reading is good for half an hour; the run of past reports for as long as the page is open
const readings = lookup<ShortsPayload>({ keepMs: SHORTS_KEEP_MS })
const trends = lookup<ShortsPayload>()
// the answer each shown reading was made from, to tell a new reading from the one shown
const shownFrom = new Map<string, Answer<ShortsPayload>>()

export async function ensureShorts(t: Trade): Promise<void> {
  const sym = discSymbol(t)
  const key = shortsKey(t)
  if (!sym) return
  const q = query({ symbol: sym, exchange: t.exchange, currency: t.currency })
  const d = await readings.read('/api/shorts?' + q, { key })
  // the reading already shown: nothing to write, and its trend stays on it
  if (shortsStore[key] && shownFrom.get(key) === d) return
  shownFrom.set(key, d)
  shortsStore[key] = Object.assign({ at: Date.now() }, d.ok ? structuredClone(d) : { ok: false }) as ShortsRec
  // the run of past reports comes free with a US listing's answer; a Canadian one is a
  // file per reporting date, asked for once the figures are on screen
  if (d.ok && d.covered && !(d.shorts?.series || []).length) {
    const more = await trends.read('/api/shorts?' + q + '&trend=1', { key })
    const rec = shortsStore[key]
    if (more.ok && more.covered && rec?.shorts) rec.shorts.series = more.shorts?.series || []
  }
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
