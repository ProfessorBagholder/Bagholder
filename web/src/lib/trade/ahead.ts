// Every trade's and holding's page ready before it is first opened (docs/decisions.md,
// 2026-09-28: no screen ever opens with nothing but the very first open). Their
// executions come in one read of every trade's and holding's (`/api/figures/details`),
// and each chart is read once, one after another, as its page would first draw it, and
// kept as drawn: its page then opens on it at once, and its own reads replace only what
// changed. A chart already kept is not read again here.
import type { Trade } from '../model'
import { read, recall, remember, whenIdle } from '../reads.svelte'
import { chartTfFor, loadHistory, type History } from './chart'
import { quiet } from '../ui.svelte'

export interface Drawn {
  tf: string
  hist: History
  provisional: boolean
}

/** The chart a page opening now would settle on, as `TradeDetail` draws it. */
export async function chartAsDrawn(t: Trade): Promise<Drawn | null> {
  const want = chartTfFor(t)
  const h = await loadHistory(t, want, undefined, true)
  const tf = chartTfFor(t, h.available)
  if (tf && tf !== want) {
    const h2 = await loadHistory(t, tf, undefined, true)
    return h2.pending ? (h2.bars.length ? { tf, hist: h2, provisional: true } : null) : { tf, hist: h2, provisional: false }
  }
  if (!h.pending) return { tf: tf || want, hist: h, provisional: false }
  if (h.bars.length) return { tf: tf || want, hist: h, provisional: true }
  // with no minute bars stored yet, the daily chart stands in, as on the page
  if (want !== '1d' && h.available.indexOf('1d') >= 0) {
    const d = await loadHistory(t, '1d', undefined, true)
    return { tf: '1d', hist: d, provisional: true }
  }
  return null
}

let running = false
/** Read ahead what the pages of `trades` open on and nothing holds yet. Once a page load; the page's own reads come first. */
export async function readAhead(trades: Trade[]): Promise<void> {
  if (running) return
  running = true
  await whenIdle()
  const details = await read('GET /api/figures/details', undefined, { ahead: true })
  // reading ahead only: a page reads its own when opened
  if (!details.ok) quiet('the executions could not be read ahead', details.error)
  for (const t of trades) {
    if (recall<Drawn>('chart ' + t.id)) continue
    // one at a time, and only while the page reads nothing of its own
    await whenIdle()
    const drawn = await chartAsDrawn(t)
    // a page opened meanwhile drew its own chart: that one stands
    if (drawn && !recall<Drawn>('chart ' + t.id)) remember('chart ' + t.id, drawn)
  }
}
