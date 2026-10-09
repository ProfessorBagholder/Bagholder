// Short interest per listing, read from /api/shorts and cached. Ported from
// the old page (_shorts / ensureShorts / shortsKey / shortDay / shortSpan). The
// store is reactive $state so the ShortInterest card renders when a fetch lands.
import type { Trade, ShortsPayload } from '../model'
import { listingTicker } from './chart'
import type { Answer } from '../api'
import { held, read } from '../reads.svelte'
import { tell } from '../ui.svelte'
import type { ShortsAnswer } from '../generated/markets'

const MON = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec']

export interface ShortsRec extends ShortsPayload {
  at: number
}

export const shortsStore = $state<Record<string, ShortsRec>>({})

const SHORTS_KEEP_MS = 30 * 60 * 1000

/**
 * Whether short selling is reported for it: a share listing only. A coin, an index, a
 * futures or currency contract and an option are reported by no one (SPEC.md §4 Trades,
 * Short interest), so nothing is asked for them and nothing is drawn.
 */
export function reportsShorts(t: Pick<Trade, 'kind'>): boolean {
  return t.kind === 'Shares'
}

export function discSymbol(t: Trade): string {
  return String(listingTicker(t) || t.symbol || '').toUpperCase()
}
export function shortsKey(t: Trade): string {
  return discSymbol(t) + '|' + String(t.exchange || '').toUpperCase()
}

// a reading is asked again once half an hour old; the run of past reports once per page load
// the answer each shown reading was made from, to tell a new reading from the one shown
const shownFrom = new Map<string, Answer<ShortsAnswer>>()

export async function ensureShorts(t: Trade): Promise<void> {
  const sym = discSymbol(t)
  const key = shortsKey(t)
  if (!sym || !reportsShorts(t)) return
  const q = { symbol: sym, exchange: t.exchange || '', currency: t.currency || '', name: '', trend: false }
  // the reading last held is drawn at once, its trend with it; the server's replaces it where it differs
  if (!shortsStore[key]) {
    const had = held('GET /api/shorts', { query: q }, { key: 'reading ' + key })
    if (had?.ok) {
      shortsStore[key] = Object.assign({ at: Date.now() }, structuredClone(had)) as ShortsRec
      const trend = held('GET /api/shorts', { query: { ...q, trend: true } }, { key: 'trend ' + key })
      if (trend?.ok && 'covered' in trend && trend.covered && shortsStore[key].shorts && !(shortsStore[key].shorts!.series || []).length) shortsStore[key].shorts!.series = trend.shorts?.series || []
    }
  }
  const d = await read('GET /api/shorts', { query: q }, { key: 'reading ' + key, maxAgeMs: SHORTS_KEEP_MS })
  // the reading already shown: nothing to write, and its trend stays on it
  if (shortsStore[key] && shownFrom.get(key) === d) return
  shownFrom.set(key, d)
  // a failure is said, and the reading shown stands
  if (!d.ok) {
    tell('Could not read short interest for ' + sym + ': ' + d.error, 'err')
    if (!shortsStore[key]) shortsStore[key] = { at: Date.now(), ok: false } as unknown as ShortsRec
    return
  }
  const series = shortsStore[key]?.shorts?.series
  const next = Object.assign({ at: Date.now() }, structuredClone(d)) as ShortsRec
  // the same reading moves nothing: its trend stays on it
  const same = shortsStore[key] && JSON.stringify({ ...shortsStore[key], at: 0, shorts: { ...shortsStore[key].shorts, series: [] } }) === JSON.stringify({ ...next, at: 0, shorts: { ...next.shorts, series: [] } })
  if (!same) {
    if (series?.length && next.shorts && !(next.shorts.series || []).length) next.shorts.series = series
    shortsStore[key] = next
  }
  // the run of past reports comes free with a US listing's answer; a Canadian one is a
  // file per reporting date, asked for once the figures are on screen
  if ('covered' in d && d.covered && !(d.shorts?.series || []).length) {
    const more = await read('GET /api/shorts', { query: { ...q, trend: true } }, { key: 'trend ' + key })
    const rec = shortsStore[key]
    if (!more.ok) tell('Could not read the short-interest trend for ' + sym + ': ' + more.error, 'err')
    else if ('covered' in more && more.covered && rec?.shorts && JSON.stringify(rec.shorts.series) !== JSON.stringify(more.shorts?.series || [])) rec.shorts.series = more.shorts?.series || []
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
