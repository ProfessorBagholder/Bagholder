// Trade-chart data helpers, ported from ledger.html (chartSpan, loadHistory,
// chartTfFor/autoTf, chartColors, fillMarkers, listingTicker/underlyingOf,
// localTime and the TIMEFRAMES/step tables). Pure functions + two caches, kept
// out of the component so the `use:tradeChart` action and the component share
// them verbatim.
import type { Trade, Fill } from '../model'
import { qty, px } from '../fmt'
import { abs, sign, waits, type Dec, type Fig } from '../dec'
import { lookup, query } from '../api'
import type { ChartHistory, DayBar, HistoryQuery, TimeBar } from '../generated/chart'

export type { DayBar, TimeBar } from '../generated/chart'
// A daily (or weekly/monthly) bar has a `date`; an intraday one has a `time`
// instead -- never both. `'date' in b` tells the two apart.
export type Bar = DayBar | TimeBar
export type History = Pick<ChartHistory, 'reason' | 'available' | 'chartSymbol' | 'pending'> & { bars: Bar[] }
export interface ChartColors {
  text: string
  grid: string
  up: string
  down: string
  line: string
}

export const TIMEFRAMES: [string, string][] = [
  ['1h', '1H'],
  ['4h', '4H'],
  ['1d', '1D'],
  ['1w', '1W'],
  ['1M', '1M'],
]
export const STEP: Record<string, number> = { '1h': 3600, '4h': 14400 }

// The timeframe a user picks is kept for that trade while the page is open.
const _tfByTrade: Record<string, string> = {}
export function setChartTf(id: string, tf: string): void {
  if (id) _tfByTrade[id] = tf
}

// The default timeframe follows the trade's length so the whole trade fits the view.
function autoTf(t: Trade): string {
  if (t.listing) return '1d'
  // an open trade runs to now
  const end = t.exitDate != null ? Date.parse(t.exitDate) : Date.now()
  const days = Math.max(1, Math.round((end - Date.parse(t.entryDate)) / 86400000) + 1)
  return days <= 2 ? '1h' : days <= 10 ? '4h' : days <= 180 ? '1d' : days <= 1100 ? '1w' : '1M'
}
export function chartTfFor(t: Trade, available?: string[]): string {
  const order = TIMEFRAMES.map((x) => x[0])
  const want = _tfByTrade[t.id] || autoTf(t)
  if (!available) return want
  if (available.indexOf(want) >= 0) return want
  const from = order.indexOf(want)
  const coarser = order.slice(from + 1).find((x) => available.indexOf(x) >= 0)
  return coarser || available[available.length - 1] || ''
}

export function chartColors(): ChartColors {
  const cs = getComputedStyle(document.documentElement)
  const v = (n: string) => cs.getPropertyValue(n).trim()
  return { text: v('--ink60'), grid: v('--grid'), up: v('--pos'), down: v('--neg'), line: 'rgba(' + v('--ink-rgb') + ',.55)' }
}

function chartSpan(t: Trade): { from: string; to: string } {
  const day = 86400000
  const from = new Date(Date.parse(t.entryDate) - 10 * day).toISOString().slice(0, 10)
  const to = new Date(t.exitDate ? Math.min(Date.now(), Date.parse(t.exitDate) + 10 * day) : Date.now()).toISOString().slice(0, 10)
  return { from, to }
}

// bars still being read are not the answer: asked again when they are in
const histories = lookup('GET /api/history', { keep: (a) => 'pending' in a && !a.pending })
/** A server started again may have other bars, or a source it did not have: ask it. */
export function forgetHistory(): void {
  histories.forget()
}
/** The chart's own question, as the server is asked it. */
export function historyQuery(t: Trade, tf: string): HistoryQuery {
  const sp = chartSpan(t)
  return { symbol: t.symbol, exchange: t.exchange || '', currency: t.currency || '', kind: t.kind || '', from: sp.from, to: sp.to, tf }
}

/** The same question as the document key a chart watches while its answer is pending. */
export function historyKey(t: Trade, tf: string): string {
  return 'history:' + query(historyQuery(t, tf))
}

/** A trade's bars. `signal` is the reader's: a chart that closes stops waiting, and a request nobody waits for is dropped. */
export async function loadHistory(t: Trade, tf: string, signal?: AbortSignal): Promise<History> {
  const r = await histories.read({ query: historyQuery(t, tf) }, { key: t.id + '|' + tf, signal })
  // a request that failed is said in the chart's place, in the failure's own words, never as a span with no bars
  if (!('bars' in r)) return { reason: r.error || 'The chart was not answered.', bars: [], available: [], chartSymbol: t.symbol, pending: false }
  return { reason: r.reason, bars: r.bars as Bar[], available: r.available, chartSymbol: r.chartSymbol || t.symbol, pending: r.pending }
}

export function underlyingOf(t: Trade): string {
  return t.underlying || String(t.symbol || '').split(' ')[0]
}
export function listingTicker(t: Trade): string {
  return (t.kind === 'Options' ? underlyingOf(t) : String(t.symbol || '')).replace(/\.(TO|V|CN|NE)$/i, '')
}
export function localTime(ts: number): number {
  return ts - new Date(ts * 1000).getTimezoneOffset() * 60
}

export interface FillMarker {
  time: number | string
  position: 'aboveBar' | 'belowBar'
  shape: 'arrowUp' | 'arrowDown'
  color: string
  text: string
}
// A fill sits on the bar that contains it: by date on daily and coarser bars, by
// bucket on intraday ones.
const unsigned = (q: Fig<Dec>): Fig<Dec> => (waits(q) ? q : abs(q))
export function fillMarkers(fills: Fill[], bars: Bar[], c: ChartColors): FillMarker[] {
  const keys = bars.map((b) => ('date' in b ? (b.date as string | number) : (b.time as number)))
  const daily = bars.length > 0 && 'date' in bars[0]
  // an intraday bar holds a fill by its time; a fill whose time was not recorded sits on its day's first bar
  const timed = (f: Fill) => f.when != null && isFinite(Date.parse(f.when))
  const dayStart = (f: Fill) => Math.round(Date.parse(f.date + 'T00:00:00') / 1000)
  const onBar = (k: string | number) => {
    let best: string | number | null = null
    for (const x of keys) {
      if (x <= k) best = x
      else break
    }
    return best != null ? best : keys.length ? keys[0] : k
  }
  const firstFrom = (k: number) => keys.find((x) => (x as number) >= k) ?? onBar(k)
  const barOf = (f: Fill) => (daily ? onBar(f.date) : timed(f) ? onBar(Math.round(Date.parse(f.when as string) / 1000)) : firstFrom(dayStart(f)))
  const shown = (t: string | number) => (typeof t === 'number' ? localTime(t) : t)
  return fills.map((f) => ({
    time: shown(bars.length ? barOf(f) : timed(f) ? Math.round(Date.parse(f.when as string) / 1000) : f.date),
    position: (f.side === 'BUY' ? 'belowBar' : 'aboveBar') as 'aboveBar' | 'belowBar',
    shape: (f.side === 'BUY' ? 'arrowUp' : 'arrowDown') as 'arrowUp' | 'arrowDown',
    color: f.side === 'BUY' ? c.up : c.down,
    text:
      (f.side === 'BUY' ? '+' : '−') +
      qty(unsigned(f.qty)) +
      (!waits(f.price) && sign(f.price) > 0 ? ' @ ' + px(f.price) : f.flags.indexOf('reward') >= 0 ? ' reward' : ''),
  }))
}
